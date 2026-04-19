//! ContractGate speed-demo binary.
//!
//! Runs three pipelines sharing one Kafka cluster (Redpanda via
//! `docker-compose.yml`):
//!
//! ```
//!     [event generator]                                      ┌─► demo.events.valid
//!            │                                               │
//!            ▼                ┌─[ContractGate validator]─────┤
//!     demo.events.in ─────────┤                              └─► demo.events.quarantine
//!                             │
//!                             └─[straight-copy baseline]───────► demo.events.copy
//! ```
//!
//! A tiny embedded Axum server serves an HTML dashboard at
//! `http://localhost:8088/` and streams live stats over Server-Sent Events.
//!
//! The point: both pipelines consume the same Kafka stream, so any throughput
//! delta is the cost of semantic validation — not serialization, not network.
//!
//! ### Run
//! ```bash
//! docker compose up -d                  # start Redpanda
//! cargo run --release --bin demo        # start the demo
//! # open http://localhost:8088
//! ```

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context};
use axum::{
    extract::State,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use contractgate::{
    contract::Contract,
    validation::{validate, CompiledContract},
};
use futures_util::stream::Stream;
use hdrhistogram::Histogram;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use rdkafka::{
    admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
    client::DefaultClientContext,
    config::ClientConfig,
    consumer::{Consumer, StreamConsumer},
    // `Producer` is the trait that provides `flush`; bringing it into scope
    // lets us call `producer.flush(...)` on our FutureProducer.
    producer::{FutureProducer, FutureRecord, Producer},
    util::Timeout,
    Message,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

// ---------------------------------------------------------------------------
// Embedded assets
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = include_str!("../../demo/static/index.html");
const SCENARIO_SIMPLE: &str = include_str!("../../demo/scenarios/simple.yaml");
const SCENARIO_NESTED: &str = include_str!("../../demo/scenarios/nested.yaml");

// Topic names — kept on `demo.` prefix so they can't collide with the real
// ingest topics once this is run against a shared cluster.
const TOPIC_IN: &str = "demo.events.in";
const TOPIC_VALID: &str = "demo.events.valid";
const TOPIC_QUARANTINE: &str = "demo.events.quarantine";
const TOPIC_COPY: &str = "demo.events.copy";

// Consumer groups — each lane runs in its own group so they each see every
// event on the input topic.
const GROUP_VALIDATOR: &str = "contractgate-validator";
const GROUP_COPY: &str = "contractgate-copy-baseline";

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Parser)]
#[command(name = "contractgate-demo", about = "ContractGate speed demo")]
pub struct Cli {
    /// Kafka bootstrap servers (comma-separated `host:port` list).
    #[arg(long, env = "KAFKA_BROKERS", default_value = "localhost:9092")]
    brokers: String,

    /// Port the dashboard HTTP server should bind to.
    #[arg(long, default_value_t = 8088)]
    port: u16,

    /// How often to emit a stats snapshot to the dashboard.
    #[arg(long, default_value_t = 200)]
    snapshot_interval_ms: u64,
}

// ---------------------------------------------------------------------------
// Run configuration (received from the dashboard on Start)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    /// "simple" or "nested"
    pub scenario: String,
    /// Fraction of events that should intentionally violate the contract
    /// (0.0 = all pass, 1.0 = all fail).  Values outside [0,1] are clamped.
    pub fail_ratio: f32,
    /// Target events/sec from the producer.  0 or missing = unbounded ("max").
    pub target_rate: Option<u64>,
    /// How long to run (seconds).  0 means "until Stop is pressed".
    pub duration_secs: u64,
}

impl RunConfig {
    fn clamp(mut self) -> Self {
        self.fail_ratio = self.fail_ratio.clamp(0.0, 1.0);
        self
    }
}

// ---------------------------------------------------------------------------
// Per-lane stats
// ---------------------------------------------------------------------------

/// Atomic counters + a histogram for one pipeline lane.  Updates are
/// lock-free except for the histogram, which is sampled (1-in-N) to keep
/// the demo hot-path cheap.
pub struct LaneStats {
    pub name: &'static str,

    // Counters
    pub consumed: AtomicU64,
    pub produced_downstream: AtomicU64,
    pub passed: AtomicU64,     // validator-only; stays 0 for the copy lane
    pub failed: AtomicU64,     // validator-only
    pub bytes_in: AtomicU64,

    // End-to-end latency histogram (producer-timestamp → "done" in this lane),
    // in microseconds.  Bounded to 60s max value.
    pub latency_hist_us: Mutex<Histogram<u64>>,

    // When the lane started (used for overall throughput).  None until the
    // first consumed record.
    pub started_at: Mutex<Option<Instant>>,
}

impl LaneStats {
    pub fn new(name: &'static str) -> Self {
        LaneStats {
            name,
            consumed: AtomicU64::new(0),
            produced_downstream: AtomicU64::new(0),
            passed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            // 1us..60_000_000us with 3 significant digits.  ~40KB allocation.
            latency_hist_us: Mutex::new(Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap()),
            started_at: Mutex::new(None),
        }
    }

    pub fn reset(&self) {
        self.consumed.store(0, Ordering::Relaxed);
        self.produced_downstream.store(0, Ordering::Relaxed);
        self.passed.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        self.bytes_in.store(0, Ordering::Relaxed);
        // The locked fields are reset from the control task in `reset_all`
    }
}

/// Snapshot of both lanes plus the producer side — the shape streamed to the
/// browser over SSE.
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
// Shared demo state
// ---------------------------------------------------------------------------

pub struct DemoState {
    pub cli: Cli,
    pub producer: FutureProducer,

    /// Compiled contracts keyed by scenario name.
    pub contracts: RwLock<std::collections::HashMap<String, Arc<CompiledContract>>>,

    /// Is a run currently executing?  Controls whether producer/consumers push.
    pub running: AtomicBool,
    pub run_started_at: Mutex<Option<Instant>>,
    pub run_config: RwLock<Option<RunConfig>>,

    /// Producer-side counters
    pub producer_sent: AtomicU64,

    /// Lane stats
    pub validator: LaneStats,
    pub copy: LaneStats,

    /// Fan-out channel for SSE subscribers.  Messages are pre-serialized
    /// snapshot JSON strings.
    pub updates: broadcast::Sender<String>,
}

impl DemoState {
    fn reset_all(&self) {
        self.producer_sent.store(0, Ordering::Relaxed);
        self.validator.reset();
        self.copy.reset();
        // Histograms + started_at need to be cleared under their locks
        // from an async context — the caller (run_start_handler) does it.
    }

    async fn set_run_config(&self, cfg: RunConfig) {
        *self.run_config.write().await = Some(cfg);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "demo=info,rdkafka=warn".into()),
        )
        .init();

    let cli = Cli::parse();

    // --- Build the shared FutureProducer used by generator + both lanes ---
    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &cli.brokers)
        // Aggressive batching — we want to showcase speed, not per-message durability
        .set("queue.buffering.max.messages", "2000000")
        .set("queue.buffering.max.ms", "5")
        .set("compression.type", "lz4")
        .set("acks", "1")
        .create()
        .context("failed to build Kafka producer (is Redpanda running on localhost:9092?)")?;

    // --- Ensure topics exist with 1 partition / replication 1 ---
    ensure_topics(&cli.brokers).await?;

    // --- Pre-compile the available contracts ---
    let mut contracts = std::collections::HashMap::new();
    for (name, yaml) in [("simple", SCENARIO_SIMPLE), ("nested", SCENARIO_NESTED)] {
        let parsed: Contract =
            serde_yaml::from_str(yaml).with_context(|| format!("parsing {name} scenario YAML"))?;
        let compiled =
            CompiledContract::compile(parsed).with_context(|| format!("compiling {name} contract"))?;
        contracts.insert(name.to_string(), Arc::new(compiled));
    }

    let (tx, _rx) = broadcast::channel::<String>(256);

    let state = Arc::new(DemoState {
        cli: cli.clone(),
        producer,
        contracts: RwLock::new(contracts),
        running: AtomicBool::new(false),
        run_started_at: Mutex::new(None),
        run_config: RwLock::new(None),
        producer_sent: AtomicU64::new(0),
        validator: LaneStats::new("validator"),
        copy: LaneStats::new("copy"),
        updates: tx,
    });

    // --- Spawn the two consumer tasks (persistent — they subscribe on startup) ---
    tokio::spawn(validator_lane(state.clone()));
    tokio::spawn(copy_lane(state.clone()));

    // --- Spawn the periodic stats broadcaster ---
    tokio::spawn(stats_broadcaster(state.clone()));

    // --- Spin up HTTP server ---
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/scenarios", get(scenarios_handler))
        .route("/api/start", post(start_handler))
        .route("/api/stop", post(stop_handler))
        .route("/api/stream", get(stream_handler))
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    tracing::info!(
        "ContractGate speed demo running — open http://localhost:{}",
        cli.port
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Topic bootstrap
// ---------------------------------------------------------------------------

async fn ensure_topics(brokers: &str) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .create()
        .context("failed to build Kafka admin client")?;

    let topics = [TOPIC_IN, TOPIC_VALID, TOPIC_QUARANTINE, TOPIC_COPY];
    let new_topics: Vec<NewTopic> = topics
        .iter()
        .map(|t| NewTopic::new(t, 1, TopicReplication::Fixed(1)))
        .collect();

    let opts = AdminOptions::new();
    // Best-effort: "already exists" is fine, anything else we log but don't fail on
    // so the demo still runs against clusters with auto-create enabled.
    match admin.create_topics(new_topics.iter(), &opts).await {
        Ok(results) => {
            for r in results {
                match r {
                    Ok(t) => tracing::info!(topic = %t, "created topic"),
                    Err((t, e)) => tracing::debug!(topic = %t, error = ?e, "topic create skipped"),
                }
            }
        }
        Err(e) => tracing::warn!("admin create_topics failed (continuing anyway): {:?}", e),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

async fn index_handler() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn scenarios_handler() -> Json<Value> {
    Json(json!({
        "scenarios": [
            {
                "key": "simple",
                "name": "user_events_simple",
                "description": "Small flat click-stream event. Fast-path baseline.",
                "yaml": SCENARIO_SIMPLE,
            },
            {
                "key": "nested",
                "name": "order_events_nested",
                "description": "Deeply nested e-commerce order with line-item array.",
                "yaml": SCENARIO_NESTED,
            }
        ]
    }))
}

async fn start_handler(
    State(state): State<Arc<DemoState>>,
    Json(cfg): Json<RunConfig>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    let cfg = cfg.clamp();

    // Validate scenario exists
    if !state.contracts.read().await.contains_key(&cfg.scenario) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("unknown scenario '{}'", cfg.scenario),
        ));
    }

    // If already running, ignore
    if state.running.load(Ordering::Acquire) {
        return Ok(Json(json!({ "status": "already_running" })));
    }

    // Reset counters
    state.reset_all();
    {
        let mut h = state.validator.latency_hist_us.lock().await;
        h.reset();
    }
    {
        let mut h = state.copy.latency_hist_us.lock().await;
        h.reset();
    }
    *state.validator.started_at.lock().await = None;
    *state.copy.started_at.lock().await = None;
    *state.run_started_at.lock().await = Some(Instant::now());

    state.set_run_config(cfg.clone()).await;
    state.running.store(true, Ordering::Release);

    // Spawn producer task — it owns its own lifetime, exits when `running` flips
    // off or duration elapses.
    tokio::spawn(producer_loop(state.clone(), cfg.clone()));

    tracing::info!(?cfg, "run started");
    Ok(Json(json!({ "status": "started", "config": cfg })))
}

async fn stop_handler(State(state): State<Arc<DemoState>>) -> Json<Value> {
    state.running.store(false, Ordering::Release);
    tracing::info!("run stopped");
    Json(json!({ "status": "stopped" }))
}

/// SSE endpoint — streams stat snapshots to the dashboard.
async fn stream_handler(
    State(state): State<Arc<DemoState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>> + Send + 'static> {
    let rx = state.updates.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(payload) => Some(Ok(SseEvent::default().data(payload))),
        Err(_) => None, // dropped messages — SSE is best-effort
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Producer loop (synthesises events and publishes to demo.events.in)
// ---------------------------------------------------------------------------

async fn producer_loop(state: Arc<DemoState>, cfg: RunConfig) {
    // Verify the requested scenario exists (the contract is used by the
    // validator lane, not the producer — but we fail fast here rather than
    // let the run limp along with no consumer side).
    if !state.contracts.read().await.contains_key(&cfg.scenario) {
        tracing::error!(scenario = %cfg.scenario, "scenario not found, aborting producer");
        state.running.store(false, Ordering::Release);
        return;
    }

    let mut rng = SmallRng::from_entropy();
    let started = Instant::now();
    let deadline = if cfg.duration_secs > 0 {
        Some(started + Duration::from_secs(cfg.duration_secs))
    } else {
        None
    };

    // Simple rate limiter — if target_rate is set (> 0) we'll sleep after every
    // `r` messages to keep pace over a 1-second window.  None = burst mode,
    // produce as fast as the broker will take.
    let sleep_every_n = cfg
        .target_rate
        .and_then(|r| (r > 0).then_some((r, Duration::from_secs(1))));

    let mut batch_start = Instant::now();
    let mut batch_count = 0u64;

    while state.running.load(Ordering::Acquire) {
        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                break;
            }
        }

        // Build one event — depending on `fail_ratio`, may intentionally violate
        let fail = rng.gen::<f32>() < cfg.fail_ratio;
        let event = match cfg.scenario.as_str() {
            "simple" => generate_simple_event(&mut rng, fail),
            "nested" => generate_nested_event(&mut rng, fail),
            _ => {
                tracing::error!(scenario = %cfg.scenario, "unknown scenario in loop");
                break;
            }
        };

        // Embed the producer-side wall clock so consumers can compute end-to-end latency.
        let produced_at_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let payload_obj = match event {
            Value::Object(mut m) => {
                m.insert("_produced_at_ns".into(), json!(produced_at_ns));
                Value::Object(m)
            }
            other => other,
        };
        let payload = serde_json::to_vec(&payload_obj).unwrap();

        // Fire and forget — we await the queue (not the broker ack) so we stay fast
        let key = format!("k{}", state.producer_sent.load(Ordering::Relaxed));
        let record = FutureRecord::to(TOPIC_IN).payload(&payload).key(&key);
        match state.producer.send(record, Timeout::After(Duration::from_secs(5))).await {
            Ok(_) => {
                state.producer_sent.fetch_add(1, Ordering::Relaxed);
            }
            Err((e, _)) => {
                tracing::warn!("producer send failed: {}", e);
                // Back off briefly on error
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        // Rate limiting: if a target rate is set, pace ourselves.
        if let Some((r, window)) = sleep_every_n {
            batch_count += 1;
            if batch_count >= r {
                let elapsed = batch_start.elapsed();
                if elapsed < window {
                    tokio::time::sleep(window - elapsed).await;
                }
                batch_start = Instant::now();
                batch_count = 0;
            }
        }
    }

    // Flush + mark run as finished
    let _ = state.producer.flush(Timeout::After(Duration::from_secs(5)));
    state.running.store(false, Ordering::Release);
    tracing::info!("producer loop exit");
}

// ---------------------------------------------------------------------------
// Validator lane — consumes events.in, validates, publishes to valid/quarantine
// ---------------------------------------------------------------------------

async fn validator_lane(state: Arc<DemoState>) {
    let consumer = match build_consumer(&state.cli.brokers, GROUP_VALIDATOR) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("validator consumer build failed: {:?}", e);
            return;
        }
    };
    if let Err(e) = consumer.subscribe(&[TOPIC_IN]) {
        tracing::error!("validator subscribe failed: {:?}", e);
        return;
    }

    loop {
        match consumer.recv().await {
            Err(e) => {
                tracing::warn!("validator consumer recv error: {:?}", e);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(msg) => {
                if !state.running.load(Ordering::Acquire) {
                    continue; // drain but don't count when not running
                }

                let payload = match msg.payload() {
                    Some(p) => p,
                    None => continue,
                };
                state
                    .validator
                    .bytes_in
                    .fetch_add(payload.len() as u64, Ordering::Relaxed);

                // First-consume marker
                {
                    let mut s = state.validator.started_at.lock().await;
                    if s.is_none() {
                        *s = Some(Instant::now());
                    }
                }

                let event: Value = match serde_json::from_slice(payload) {
                    Ok(v) => v,
                    Err(_) => {
                        state.validator.failed.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };

                // Capture the producer timestamp *before* we potentially move
                // `event` into the quarantine JSON below.
                let produced_at_ns =
                    event.get("_produced_at_ns").and_then(|v| v.as_u64());

                // Pick the contract from the current run config
                let scenario = {
                    let cfg = state.run_config.read().await;
                    cfg.as_ref().map(|c| c.scenario.clone()).unwrap_or_default()
                };
                let compiled = state.contracts.read().await.get(&scenario).cloned();
                let compiled = match compiled {
                    Some(c) => c,
                    None => continue,
                };

                let result = validate(&compiled, &event);

                if result.passed {
                    state.validator.passed.fetch_add(1, Ordering::Relaxed);
                    // Forward to valid topic — re-use the original payload bytes.
                    // Turbofish pins K=() (rdkafka provides `impl ToBytes for ()`)
                    // so inference succeeds without needing a real key.
                    let record = FutureRecord::<(), [u8]>::to(TOPIC_VALID).payload(payload);
                    if state
                        .producer
                        .send(record, Timeout::After(Duration::from_secs(1)))
                        .await
                        .is_ok()
                    {
                        state
                            .validator
                            .produced_downstream
                            .fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    state.validator.failed.fetch_add(1, Ordering::Relaxed);
                    // Forward to quarantine topic with the violation list attached
                    let quarantined = json!({
                        "event": event,
                        "violations": result.violations,
                        "validation_us": result.validation_us,
                    });
                    let bytes = serde_json::to_vec(&quarantined).unwrap_or_default();
                    let record = FutureRecord::<(), [u8]>::to(TOPIC_QUARANTINE).payload(&bytes);
                    if state
                        .producer
                        .send(record, Timeout::After(Duration::from_secs(1)))
                        .await
                        .is_ok()
                    {
                        state
                            .validator
                            .produced_downstream
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }

                state.validator.consumed.fetch_add(1, Ordering::Relaxed);

                // End-to-end latency (producer_wall_clock → now)
                if let Some(produced_at_ns) = produced_at_ns {
                    if let Ok(now_ns) = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                    {
                        if now_ns > produced_at_ns {
                            let delta_us = (now_ns - produced_at_ns) / 1_000;
                            // Sample roughly 1-in-8 records for the histogram to keep
                            // contention down at very high rates.
                            if state.validator.consumed.load(Ordering::Relaxed) % 8 == 0 {
                                let mut hist = state.validator.latency_hist_us.lock().await;
                                let _ = hist.record(delta_us.min(60_000_000));
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Straight-copy baseline lane — consumes events.in, re-emits to events.copy
// ---------------------------------------------------------------------------

async fn copy_lane(state: Arc<DemoState>) {
    let consumer = match build_consumer(&state.cli.brokers, GROUP_COPY) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("copy consumer build failed: {:?}", e);
            return;
        }
    };
    if let Err(e) = consumer.subscribe(&[TOPIC_IN]) {
        tracing::error!("copy subscribe failed: {:?}", e);
        return;
    }

    loop {
        match consumer.recv().await {
            Err(e) => {
                tracing::warn!("copy consumer recv error: {:?}", e);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(msg) => {
                if !state.running.load(Ordering::Acquire) {
                    continue;
                }
                let payload = match msg.payload() {
                    Some(p) => p,
                    None => continue,
                };
                state.copy.bytes_in.fetch_add(payload.len() as u64, Ordering::Relaxed);

                {
                    let mut s = state.copy.started_at.lock().await;
                    if s.is_none() {
                        *s = Some(Instant::now());
                    }
                }

                // Re-emit unchanged — the baseline doesn't validate, doesn't parse.
                // It deliberately represents "just move bytes through".
                let record = FutureRecord::<(), [u8]>::to(TOPIC_COPY).payload(payload);
                // `payload` borrows `msg`; `msg` lives through this match arm.
                if state
                    .producer
                    .send(record, Timeout::After(Duration::from_secs(1)))
                    .await
                    .is_ok()
                {
                    state.copy.produced_downstream.fetch_add(1, Ordering::Relaxed);
                }

                state.copy.consumed.fetch_add(1, Ordering::Relaxed);

                // End-to-end latency — parse the produced-at timestamp cheaply
                // without a full JSON parse when possible.
                if let Some(produced_at_ns) = extract_produced_at_ns(payload) {
                    if let Ok(now_ns) = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                    {
                        if now_ns > produced_at_ns {
                            let delta_us = (now_ns - produced_at_ns) / 1_000;
                            if state.copy.consumed.load(Ordering::Relaxed) % 8 == 0 {
                                let mut hist = state.copy.latency_hist_us.lock().await;
                                let _ = hist.record(delta_us.min(60_000_000));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Cheap byte-scan for "_produced_at_ns":<u64>.  Avoids a full JSON parse for
/// the baseline lane so its overhead really is "just copy bytes".  Falls back
/// to `None` on parse failure — which just means we skip the latency sample.
fn extract_produced_at_ns(bytes: &[u8]) -> Option<u64> {
    let needle = b"\"_produced_at_ns\":";
    let pos = bytes.windows(needle.len()).position(|w| w == needle)?;
    let rest = &bytes[pos + needle.len()..];
    // Skip optional whitespace
    let start = rest.iter().position(|&b| b.is_ascii_digit())?;
    let end = rest[start..]
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .unwrap_or(rest.len() - start);
    let s = std::str::from_utf8(&rest[start..start + end]).ok()?;
    s.parse::<u64>().ok()
}

// ---------------------------------------------------------------------------
// Kafka helpers
// ---------------------------------------------------------------------------

fn build_consumer(brokers: &str, group: &str) -> anyhow::Result<StreamConsumer> {
    let c: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "true")
        .set("auto.commit.interval.ms", "1000")
        // "latest" — when a fresh run starts, don't replay backlog from prior runs
        .set("auto.offset.reset", "latest")
        .set("fetch.min.bytes", "1")
        .set("fetch.wait.max.ms", "5")
        .set("session.timeout.ms", "15000")
        .create()
        .map_err(|e| anyhow!("consumer create failed: {e}"))?;
    Ok(c)
}

// ---------------------------------------------------------------------------
// Stats broadcaster — emits a snapshot every N ms over the SSE channel
// ---------------------------------------------------------------------------

async fn stats_broadcaster(state: Arc<DemoState>) {
    let interval = Duration::from_millis(state.cli.snapshot_interval_ms);
    loop {
        tokio::time::sleep(interval).await;

        let running = state.running.load(Ordering::Acquire);
        let run_started = *state.run_started_at.lock().await;
        let elapsed_ms = run_started
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let cfg = state.run_config.read().await.clone();
        let (scenario, fail_ratio) = cfg
            .as_ref()
            .map(|c| (c.scenario.clone(), c.fail_ratio))
            .unwrap_or_else(|| ("-".into(), 0.0));

        // Producer
        let producer_sent = state.producer_sent.load(Ordering::Relaxed);
        let producer_rate = rate_per_sec(producer_sent, elapsed_ms);

        let validator = snapshot_lane(&state.validator).await;
        let copy = snapshot_lane(&state.copy).await;

        let snap = StatsSnapshot {
            running,
            elapsed_ms,
            scenario,
            fail_ratio,
            producer: ProducerSnapshot {
                sent: producer_sent,
                rate_per_sec: producer_rate,
            },
            validator,
            copy,
        };

        let json = match serde_json::to_string(&snap) {
            Ok(j) => j,
            Err(_) => continue,
        };
        let _ = state.updates.send(json);
    }
}

async fn snapshot_lane(lane: &LaneStats) -> LaneSnapshot {
    let consumed = lane.consumed.load(Ordering::Relaxed);
    let produced_downstream = lane.produced_downstream.load(Ordering::Relaxed);
    let passed = lane.passed.load(Ordering::Relaxed);
    let failed = lane.failed.load(Ordering::Relaxed);
    let bytes_in = lane.bytes_in.load(Ordering::Relaxed);

    let started = *lane.started_at.lock().await;
    let ms = started.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    let rate = rate_per_sec(consumed, ms);

    let hist = lane.latency_hist_us.lock().await;
    let (p50, p95, p99, max) = if hist.len() == 0 {
        (0, 0, 0, 0)
    } else {
        (
            hist.value_at_quantile(0.50),
            hist.value_at_quantile(0.95),
            hist.value_at_quantile(0.99),
            hist.max(),
        )
    };

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

fn rate_per_sec(count: u64, elapsed_ms: u64) -> f64 {
    if elapsed_ms == 0 {
        return 0.0;
    }
    (count as f64) * 1000.0 / (elapsed_ms as f64)
}

// ---------------------------------------------------------------------------
// Event generators
// ---------------------------------------------------------------------------

fn generate_simple_event(rng: &mut SmallRng, fail: bool) -> Value {
    const EVENTS: &[&str] = &["click", "view", "purchase", "login", "logout"];
    let event_type = EVENTS[rng.gen_range(0..EVENTS.len())];

    let user_id = format!("u_{:08x}", rng.gen::<u32>());
    let session_id: String = (0..32)
        .map(|_| {
            let n = rng.gen_range(0..16u8);
            std::char::from_digit(n as u32, 16).unwrap()
        })
        .collect();
    let timestamp: u64 = 1_700_000_000 + rng.gen_range(0..10_000_000);
    let amount: f64 = if event_type == "purchase" {
        (rng.gen::<f64>() * 999.0).round() / 100.0
    } else {
        0.0
    };

    let mut event = json!({
        "user_id": user_id,
        "event_type": event_type,
        "timestamp": timestamp,
        "session_id": session_id,
    });
    if event_type == "purchase" {
        event["amount"] = json!(amount);
    }

    if fail {
        // Pick one of a handful of violation modes
        match rng.gen_range(0..5u8) {
            0 => {
                // Missing required field
                event.as_object_mut().unwrap().remove("session_id");
            }
            1 => {
                // Pattern violation on user_id (spaces + uppercase)
                event["user_id"] = json!("NOT A VALID ID!!");
            }
            2 => {
                // Enum violation on event_type
                event["event_type"] = json!("unknown_action");
            }
            3 => {
                // Type mismatch — timestamp as string
                event["timestamp"] = json!("not-a-number");
            }
            _ => {
                // Range violation — negative amount
                event["amount"] = json!(-5.0);
            }
        }
    }

    event
}

fn generate_nested_event(rng: &mut SmallRng, fail: bool) -> Value {
    const CHANNELS: &[&str] = &["web", "ios", "android", "pos", "partner_api"];
    const TIERS: &[&str] = &["free", "silver", "gold", "platinum"];
    const METHODS: &[&str] = &["card", "paypal", "apple_pay", "google_pay", "bank_transfer"];
    const COUNTRIES: &[&str] = &["US", "DE", "GB", "FR", "CA", "AU", "JP", "BR"];

    let order_id = format!(
        "ord_{}",
        (0..16)
            .map(|_| {
                let n = rng.gen_range(0..36u8);
                if n < 10 {
                    (b'0' + n) as char
                } else {
                    (b'a' + (n - 10)) as char
                }
            })
            .collect::<String>()
    );

    let num_items = rng.gen_range(1..5);
    let items: Vec<Value> = (0..num_items)
        .map(|_| {
            json!({
                "sku": format!("SKU-{:06X}", rng.gen::<u32>() & 0xFFFFFF),
                "quantity": rng.gen_range(1..20),
                "unit_price": (rng.gen::<f64>() * 99999.0).round() / 100.0,
            })
        })
        .collect();

    let total_amount: f64 = items
        .iter()
        .map(|it| {
            let q = it.get("quantity").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let p = it.get("unit_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            q * p
        })
        .sum();

    // Hoisted out of `json!`: a value position starting with `[` commits the
    // macro to parsing a JSON array literal, so we can't write
    // `["USD", ...][idx]` inline.
    const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "JPY"];
    const CITIES: &[&str] = &["Berlin", "Paris", "Austin", "Tokyo", "Sydney"];
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
        "total_amount": (total_amount * 100.0).round() / 100.0,
        "payment": {
            "method": METHODS[rng.gen_range(0..METHODS.len())],
            "last4": format!("{:04}", rng.gen_range(0..10000u32)),
        }
    });

    if fail {
        match rng.gen_range(0..6u8) {
            0 => {
                // Missing nested required field
                event["customer"]["address"].as_object_mut().unwrap().remove("country");
            }
            1 => {
                // Enum violation at top level
                event["channel"] = json!("carrier_pigeon");
            }
            2 => {
                // Pattern violation on order_id
                event["order_id"] = json!("not-an-order");
            }
            3 => {
                // Pattern violation on email
                event["customer"]["email"] = json!("not-an-email");
            }
            4 => {
                // Array item violation — bad SKU pattern
                if let Some(items) = event["items"].as_array_mut() {
                    if let Some(first) = items.first_mut() {
                        first["sku"] = json!("bad sku!");
                    }
                }
            }
            _ => {
                // Metric range violation
                event["total_amount"] = json!(20_000_000.0);
            }
        }
    }

    event
}
