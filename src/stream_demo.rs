//! In-process Stream Demo — the same validation pipeline as the Kafka demo
//! but without Kafka and without any database writes.
//!
//! Architecture:
//!   - A single `run_loop` Tokio task generates synthetic events, runs them
//!     through the real `validation::validate()` engine (validator lane) and
//!     through a serde-only round-trip (copy/baseline lane), then updates
//!     atomic counters.
//!   - A `stats_broadcaster` task wakes every 200 ms, snapshots the counters
//!     + HDR histograms, and fans the JSON out over a broadcast channel.
//!   - Three HTTP endpoints: POST /demo/start, POST /demo/stop, GET /demo/stream
//!     (SSE).  All three are placed on the *public* router — no API key needed
//!     so the browser's EventSource can connect without auth headers.
//!
//! Latency measurements:
//!   Validator lane  = time to call `validate()` on a pre-parsed `Value`
//!   Copy lane       = time to `serde_json::to_vec()` the same `Value`
//!   Overhead        = validator_p99 − copy_p99  (pure validation cost)
//!
//! These are real wall-clock measurements of the Rust engine, not simulated.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    Json,
};
use futures_util::stream::Stream;
use hdrhistogram::Histogram;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use tokio_stream::{wrappers::BroadcastStream, StreamExt as _};

use crate::validation::{validate, CompiledContract};
use crate::contract::Contract;

// Scenario YAML files are embedded at compile time — same approach as the
// Kafka demo binary so they're always in sync.
const SCENARIO_SIMPLE: &str = include_str!("../demo/scenarios/simple.yaml");
const SCENARIO_NESTED: &str = include_str!("../demo/scenarios/nested.yaml");

// SSE snapshot broadcast interval
const SNAPSHOT_MS: u64 = 200;

// ---------------------------------------------------------------------------
// Wire types (identical shape to the Kafka demo for frontend compatibility)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub scenario: String,
    /// Fraction of events that should intentionally violate the contract.
    pub fail_ratio: f32,
    /// Target events/sec; 0 or None = unbounded (max throughput).
    pub target_rate: Option<u64>,
    /// Seconds to run; 0 = until Stop.
    pub duration_secs: u64,
}

impl RunConfig {
    fn clamp(mut self) -> Self {
        self.fail_ratio = self.fail_ratio.clamp(0.0, 1.0);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    pub running: bool,
    pub elapsed_ms: u64,
    pub scenario: String,
    pub fail_ratio: f32,
    pub producer: ProducerSnapshot,
    pub validator: LaneSnapshot,
    pub copy: LaneSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProducerSnapshot {
    pub sent: u64,
    pub rate_per_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaneSnapshot {
    pub consumed: u64,
    pub produced_downstream: u64,
    pub passed: u64,
    pub failed: u64,
    pub bytes_in: u64,
    pub rate_per_sec: f64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

// ---------------------------------------------------------------------------
// Per-lane counters + latency histogram
// ---------------------------------------------------------------------------

pub struct LaneCounters {
    pub consumed: AtomicU64,
    pub produced_downstream: AtomicU64,
    pub passed: AtomicU64,
    pub failed: AtomicU64,
    pub bytes_in: AtomicU64,
    /// HDR histogram in microseconds, 1 µs – 60 s.  Sampled 1-in-8 to keep
    /// lock contention negligible on the hot path.
    pub hist: Mutex<Histogram<u64>>,
}

impl LaneCounters {
    fn new() -> Self {
        LaneCounters {
            consumed: AtomicU64::new(0),
            produced_downstream: AtomicU64::new(0),
            passed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            hist: Mutex::new(
                Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)
                    .expect("valid HDR histogram bounds"),
            ),
        }
    }

    fn reset(&self) {
        self.consumed.store(0, Ordering::Relaxed);
        self.produced_downstream.store(0, Ordering::Relaxed);
        self.passed.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        self.bytes_in.store(0, Ordering::Relaxed);
        if let Ok(mut h) = self.hist.lock() {
            h.reset();
        }
    }

    fn snapshot(&self, elapsed_ms: u64) -> LaneSnapshot {
        let consumed = self.consumed.load(Ordering::Relaxed);
        let produced_downstream = self.produced_downstream.load(Ordering::Relaxed);
        let passed = self.passed.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        let bytes_in = self.bytes_in.load(Ordering::Relaxed);

        let rate = if elapsed_ms > 0 {
            consumed as f64 * 1_000.0 / elapsed_ms as f64
        } else {
            0.0
        };

        let (p50, p95, p99, max) = self
            .hist
            .lock()
            .map(|h| {
                if h.len() == 0 {
                    (0, 0, 0, 0)
                } else {
                    (
                        h.value_at_quantile(0.50),
                        h.value_at_quantile(0.95),
                        h.value_at_quantile(0.99),
                        h.max(),
                    )
                }
            })
            .unwrap_or((0, 0, 0, 0));

        LaneSnapshot {
            consumed,
            produced_downstream,
            passed,
            failed,
            bytes_in,
            rate_per_sec: rate,
            p50_us: p50,
            p95_us: p95,
            p99_us: p99,
            max_us: max,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared demo state
// ---------------------------------------------------------------------------

pub struct StreamDemoState {
    /// Pre-compiled contracts, keyed by scenario name.  Built once on server
    /// start; immutable thereafter.
    contracts: HashMap<&'static str, Arc<CompiledContract>>,

    pub running: AtomicBool,
    run_started_at: Mutex<Option<Instant>>,
    run_config: RwLock<Option<RunConfig>>,

    producer_sent: AtomicU64,

    pub validator: LaneCounters,
    pub copy: LaneCounters,

    /// Broadcast channel for SSE fan-out.  Messages are pre-serialised JSON.
    pub updates: broadcast::Sender<String>,

    /// Abort handle for the active run task.  Replaced on every Start.
    run_handle: tokio::sync::Mutex<Option<tokio::task::AbortHandle>>,
}

impl StreamDemoState {
    pub fn new() -> Self {
        let mut contracts: HashMap<&'static str, Arc<CompiledContract>> = HashMap::new();
        for (name, yaml_src) in [("simple", SCENARIO_SIMPLE), ("nested", SCENARIO_NESTED)] {
            let parsed: Contract = serde_yaml::from_str(yaml_src)
                .unwrap_or_else(|e| panic!("demo scenario '{name}' YAML invalid: {e}"));
            let compiled = CompiledContract::compile(parsed)
                .unwrap_or_else(|e| panic!("demo scenario '{name}' failed to compile: {e}"));
            contracts.insert(name, Arc::new(compiled));
        }

        let (tx, _) = broadcast::channel(512);

        StreamDemoState {
            contracts,
            running: AtomicBool::new(false),
            run_started_at: Mutex::new(None),
            run_config: RwLock::new(None),
            producer_sent: AtomicU64::new(0),
            validator: LaneCounters::new(),
            copy: LaneCounters::new(),
            updates: tx,
            run_handle: tokio::sync::Mutex::new(None),
        }
    }

    fn reset(&self) {
        self.producer_sent.store(0, Ordering::Relaxed);
        self.validator.reset();
        self.copy.reset();
        if let Ok(mut t) = self.run_started_at.lock() {
            *t = Some(Instant::now());
        }
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        let running = self.running.load(Ordering::Acquire);
        let elapsed_ms = self
            .run_started_at
            .lock()
            .ok()
            .and_then(|g| *g)
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let (scenario, fail_ratio) = self
            .run_config
            .try_read()
            .ok()
            .and_then(|g| g.clone())
            .map(|c| (c.scenario, c.fail_ratio))
            .unwrap_or_else(|| ("-".into(), 0.0));

        let producer_sent = self.producer_sent.load(Ordering::Relaxed);
        let producer_rate = if elapsed_ms > 0 {
            producer_sent as f64 * 1_000.0 / elapsed_ms as f64
        } else {
            0.0
        };

        StatsSnapshot {
            running,
            elapsed_ms,
            scenario,
            fail_ratio,
            producer: ProducerSnapshot {
                sent: producer_sent,
                rate_per_sec: producer_rate,
            },
            validator: self.validator.snapshot(elapsed_ms),
            copy: self.copy.snapshot(elapsed_ms),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

pub async fn start_handler(
    State(state): State<Arc<crate::AppState>>,
    Json(cfg): Json<RunConfig>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let cfg = cfg.clamp();

    if !state.stream_demo.contracts.contains_key(cfg.scenario.as_str()) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("unknown scenario '{}'", cfg.scenario),
        ));
    }

    // If a run is already active, stop it cleanly first.
    if state.stream_demo.running.load(Ordering::Acquire) {
        stop_run(&state.stream_demo).await;
    }

    state.stream_demo.reset();
    *state.stream_demo.run_config.write().await = Some(cfg.clone());
    state.stream_demo.running.store(true, Ordering::Release);

    // Spawn the event-generation + validation loop.
    let demo = Arc::clone(&state.stream_demo);
    let handle = tokio::spawn(run_loop(demo, cfg.clone())).abort_handle();
    *state.stream_demo.run_handle.lock().await = Some(handle);

    // Spawn the stats broadcaster (exits when running goes false).
    let demo = Arc::clone(&state.stream_demo);
    tokio::spawn(stats_broadcaster(demo));

    tracing::info!(scenario = %cfg.scenario, fail_ratio = %cfg.fail_ratio, "stream demo started");
    Ok(Json(json!({ "status": "started", "scenario": cfg.scenario })))
}

pub async fn stop_handler(
    State(state): State<Arc<crate::AppState>>,
) -> Json<Value> {
    stop_run(&state.stream_demo).await;
    tracing::info!("stream demo stopped");
    Json(json!({ "status": "stopped" }))
}

pub async fn stream_handler(
    State(state): State<Arc<crate::AppState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>> + Send + 'static> {
    // Push the current snapshot immediately so the browser doesn't wait for
    // the first 200 ms broadcaster tick.
    let initial = serde_json::to_string(&state.stream_demo.snapshot()).unwrap_or_default();
    let _ = state.stream_demo.updates.send(initial);

    let rx = state.stream_demo.updates.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(payload) => Some(Ok(SseEvent::default().data(payload))),
        Err(_) => None, // lagged — skip
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

async fn stop_run(demo: &StreamDemoState) {
    demo.running.store(false, Ordering::Release);
    if let Some(handle) = demo.run_handle.lock().await.take() {
        handle.abort();
    }
    // Broadcast one final snapshot with running=false.
    let snap = demo.snapshot();
    if let Ok(json) = serde_json::to_string(&snap) {
        let _ = demo.updates.send(json);
    }
}

// ---------------------------------------------------------------------------
// Run loop — generates events, validates, simulates copy lane
// ---------------------------------------------------------------------------

async fn run_loop(state: Arc<StreamDemoState>, cfg: RunConfig) {
    let compiled = match state.contracts.get(cfg.scenario.as_str()) {
        Some(c) => Arc::clone(c),
        None => {
            state.running.store(false, Ordering::Release);
            return;
        }
    };

    let mut rng = SmallRng::from_entropy();
    let started = Instant::now();
    let deadline = (cfg.duration_secs > 0)
        .then(|| started + Duration::from_secs(cfg.duration_secs));

    // Rate-limit: pace over 1-second windows when target_rate is set.
    let target_per_sec = cfg.target_rate.filter(|&r| r > 0);
    let mut window_start = Instant::now();
    let mut window_count = 0u64;

    while state.running.load(Ordering::Acquire) {
        if deadline.map_or(false, |d| Instant::now() >= d) {
            break;
        }

        let fail = rng.gen::<f32>() < cfg.fail_ratio;
        let event = match cfg.scenario.as_str() {
            "simple" => generate_simple_event(&mut rng, fail),
            "nested" => generate_nested_event(&mut rng, fail),
            _ => break,
        };

        // Serialise once — both lanes share these bytes.
        let bytes = serde_json::to_vec(&event).unwrap_or_default();
        let byte_len = bytes.len() as u64;

        state.producer_sent.fetch_add(1, Ordering::Relaxed);

        // ── Validator lane ────────────────────────────────────────────────
        let v_start = Instant::now();
        let result = validate(&compiled, &event);
        let v_us = v_start.elapsed().as_micros() as u64;

        let v_seq = state.validator.consumed.fetch_add(1, Ordering::Relaxed);
        state.validator.produced_downstream.fetch_add(1, Ordering::Relaxed);
        state.validator.bytes_in.fetch_add(byte_len, Ordering::Relaxed);
        if result.passed {
            state.validator.passed.fetch_add(1, Ordering::Relaxed);
        } else {
            state.validator.failed.fetch_add(1, Ordering::Relaxed);
        }
        // Sample 1-in-8 into the histogram.
        if v_seq % 8 == 0 {
            if let Ok(mut h) = state.validator.hist.lock() {
                let _ = h.record(v_us.max(1).min(60_000_000));
            }
        }

        // ── Copy / baseline lane (serde only, no validation) ─────────────
        let c_start = Instant::now();
        let _ = serde_json::to_vec(&event); // ~same cost as re-encoding in the Kafka lane
        let c_us = c_start.elapsed().as_micros() as u64;

        let c_seq = state.copy.consumed.fetch_add(1, Ordering::Relaxed);
        state.copy.produced_downstream.fetch_add(1, Ordering::Relaxed);
        state.copy.bytes_in.fetch_add(byte_len, Ordering::Relaxed);
        state.copy.passed.fetch_add(1, Ordering::Relaxed);
        if c_seq % 8 == 0 {
            if let Ok(mut h) = state.copy.hist.lock() {
                let _ = h.record(c_us.max(1).min(60_000_000));
            }
        }

        // ── Rate limiting ─────────────────────────────────────────────────
        if let Some(r) = target_per_sec {
            window_count += 1;
            if window_count >= r {
                let elapsed = window_start.elapsed();
                if elapsed < Duration::from_secs(1) {
                    tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
                }
                window_start = Instant::now();
                window_count = 0;
            }
        } else {
            // Unbounded: yield periodically so the reactor stays responsive.
            if state.producer_sent.load(Ordering::Relaxed) % 5_000 == 0 {
                tokio::task::yield_now().await;
            }
        }
    }

    state.running.store(false, Ordering::Release);
    tracing::info!("stream demo run loop exited");
}

// ---------------------------------------------------------------------------
// Stats broadcaster
// ---------------------------------------------------------------------------

async fn stats_broadcaster(state: Arc<StreamDemoState>) {
    let interval = Duration::from_millis(SNAPSHOT_MS);
    // Loop as long as the run is active.
    loop {
        tokio::time::sleep(interval).await;
        let snap = state.snapshot();
        let still_running = snap.running;
        if let Ok(json) = serde_json::to_string(&snap) {
            let _ = state.updates.send(json);
        }
        if !still_running {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Event generators (identical to demo binary — kept in sync manually)
// ---------------------------------------------------------------------------

fn generate_simple_event(rng: &mut SmallRng, fail: bool) -> Value {
    const EVENTS: &[&str] = &["click", "view", "purchase", "login", "logout"];
    let event_type = EVENTS[rng.gen_range(0..EVENTS.len())];
    let user_id = format!("u_{:08x}", rng.gen::<u32>());
    let session_id: String = (0..32)
        .map(|_| {
            let n = rng.gen_range(0..16u8);
            char::from_digit(n as u32, 16).unwrap()
        })
        .collect();
    let timestamp: u64 = 1_700_000_000 + rng.gen_range(0..10_000_000);

    let mut event = json!({
        "user_id": user_id,
        "event_type": event_type,
        "timestamp": timestamp,
        "session_id": session_id,
    });
    if event_type == "purchase" {
        let amount = (rng.gen::<f64>() * 999.0 * 100.0).round() / 100.0;
        event["amount"] = json!(amount);
    }

    if fail {
        match rng.gen_range(0..5u8) {
            0 => { event.as_object_mut().unwrap().remove("session_id"); }
            1 => { event["user_id"] = json!("NOT A VALID ID!!"); }
            2 => { event["event_type"] = json!("unknown_action"); }
            3 => { event["timestamp"] = json!("not-a-number"); }
            _ => { event["amount"] = json!(-5.0); }
        }
    }
    event
}

fn generate_nested_event(rng: &mut SmallRng, fail: bool) -> Value {
    const CHANNELS: &[&str] = &["web", "ios", "android", "pos", "partner_api"];
    const TIERS: &[&str] = &["free", "silver", "gold", "platinum"];
    const METHODS: &[&str] = &["card", "paypal", "apple_pay", "google_pay", "bank_transfer"];
    const COUNTRIES: &[&str] = &["US", "DE", "GB", "FR", "CA", "AU", "JP", "BR"];
    const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "JPY"];
    const CITIES: &[&str] = &["Berlin", "Paris", "Austin", "Tokyo", "Sydney"];

    let order_id = format!(
        "ord_{}",
        (0..16)
            .map(|_| {
                let n = rng.gen_range(0..36u8);
                if n < 10 { (b'0' + n) as char } else { (b'a' + (n - 10)) as char }
            })
            .collect::<String>()
    );

    let num_items = rng.gen_range(1..5usize);
    let items: Vec<Value> = (0..num_items)
        .map(|_| json!({
            "sku": format!("SKU-{:06X}", rng.gen::<u32>() & 0xFF_FFFF),
            "quantity": rng.gen_range(1..20u32),
            "unit_price": (rng.gen::<f64>() * 99_999.0 * 100.0).round() / 100.0,
        }))
        .collect();

    let total: f64 = items.iter().map(|it| {
        let q = it["quantity"].as_f64().unwrap_or(0.0);
        let p = it["unit_price"].as_f64().unwrap_or(0.0);
        q * p
    }).sum();

    let currency = CURRENCIES[rng.gen_range(0..CURRENCIES.len())];
    let city = CITIES[rng.gen_range(0..CITIES.len())];

    let mut event = json!({
        "order_id": order_id,
        "placed_at": 1_700_000_000u64 + rng.gen_range(0..10_000_000u64),
        "channel": CHANNELS[rng.gen_range(0..CHANNELS.len())],
        "currency": currency,
        "customer": {
            "id": format!("c_{:08x}", rng.gen::<u32>()),
            "email": format!("user{}@example.com", rng.gen::<u16>()),
            "tier": TIERS[rng.gen_range(0..TIERS.len())],
            "address": {
                "country": COUNTRIES[rng.gen_range(0..COUNTRIES.len())],
                "postal_code": format!("{:05}", rng.gen_range(10000..99999u32)),
                "city": city,
            }
        },
        "items": items,
        "total_amount": (total * 100.0).round() / 100.0,
        "payment": {
            "method": METHODS[rng.gen_range(0..METHODS.len())],
            "last4": format!("{:04}", rng.gen_range(0..10000u32)),
        }
    });

    if fail {
        match rng.gen_range(0..6u8) {
            0 => { event["customer"]["address"].as_object_mut().unwrap().remove("country"); }
            1 => { event["channel"] = json!("carrier_pigeon"); }
            2 => { event["order_id"] = json!("not-an-order"); }
            3 => { event["customer"]["email"] = json!("not-an-email"); }
            4 => {
                if let Some(items) = event["items"].as_array_mut() {
                    if let Some(first) = items.first_mut() {
                        first["sku"] = json!("bad sku!");
                    }
                }
            }
            _ => { event["total_amount"] = json!(20_000_000.0); }
        }
    }
    event
}
