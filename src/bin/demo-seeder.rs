//! ContractGate demo seeder (RFC-017).
//!
//! Publishes the three starter contracts to a running gateway, then posts
//! realistic synthetic events for a configured duration so audit_log fills,
//! Prometheus metrics are populated, and the audit search UI (RFC-015) has
//! rows to filter.
//!
//! ### Run
//! ```bash
//! # Against a local gateway (docker compose up first):
//! demo-seeder --gateway-url http://localhost:8080
//!
//! # Custom rate + duration:
//! demo-seeder --rate 50 --duration 60s
//!
//! # Inside Compose (--profile demo):
//! docker compose --profile demo up
//! ```
//!
//! ### Behavior
//! 1. For each contract in `--contracts`: POST to `/contracts` if not already
//!    published; promote to stable if no stable version exists yet.
//! 2. Loop until `--duration` elapses, sleeping `1/rate` seconds each iteration:
//!    - Pick a random contract from the list.
//!    - Roll dice against pass/fail/quarantine percentages.
//!    - Generate a payload matching the outcome.
//!    - POST to `/ingest/:contract_id`.
//! 3. Print summary on exit: events sent, outcome breakdown, p99 round-trip.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::Parser;
use contractgate::demo_seed::{
    client::GatewayClient,
    outcome::{roll, Outcome},
    synth::generate,
};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Starter YAML embedded at compile time so the binary ships self-contained.
// ---------------------------------------------------------------------------

const REST_EVENT_YAML: &str = include_str!("../../contracts/starters/rest_event.yaml");
const KAFKA_EVENT_YAML: &str = include_str!("../../contracts/starters/kafka_event.yaml");
const DBT_MODEL_YAML: &str = include_str!("../../contracts/starters/dbt_model.yaml");

fn starter_yaml(name: &str) -> &'static str {
    match name {
        "rest_event" => REST_EVENT_YAML,
        "kafka_event" => KAFKA_EVENT_YAML,
        "dbt_model_row" | "dbt_model" => DBT_MODEL_YAML,
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Duration parser: accepts "30s", "5m", "1h" or plain seconds.
// ---------------------------------------------------------------------------

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('s') {
        n.parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| e.to_string())
    } else if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>()
            .map(|m| Duration::from_secs(m * 60))
            .map_err(|e| e.to_string())
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>()
            .map(|h| Duration::from_secs(h * 3600))
            .map_err(|e| e.to_string())
    } else {
        s.parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| format!("invalid duration '{s}': {e}"))
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Parser)]
#[command(
    name = "demo-seeder",
    about = "Publish starter contracts and seed the gateway with synthetic events (RFC-017)"
)]
struct Cli {
    /// Gateway base URL.
    #[arg(long, env = "GATEWAY_URL", default_value = "http://localhost:8080")]
    gateway_url: String,

    /// API key sent as `x-api-key` header.  Empty = no auth (dev mode).
    #[arg(long, env = "CONTRACTGATE_API_KEY", default_value = "")]
    api_key: String,

    /// Events per second.
    #[arg(long, default_value_t = 10)]
    rate: u64,

    /// How long to run (e.g. 300s, 5m, 1h).
    #[arg(long, value_parser = parse_duration, default_value = "5m")]
    duration: Duration,

    /// Fraction of events that should pass (0.0–1.0).
    #[arg(long, default_value_t = 0.80)]
    pass_pct: f64,

    /// Fraction of events that should fail with a constraint violation.
    #[arg(long, default_value_t = 0.15)]
    fail_pct: f64,

    // quarantine_pct is implicit: 1 - pass_pct - fail_pct
    /// Comma-separated contract names to publish and seed.
    #[arg(
        long,
        default_value = "rest_event,kafka_event,dbt_model_row",
        value_delimiter = ','
    )]
    contracts: Vec<String>,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "demo_seeder=info,contractgate=warn".into()),
        )
        .init();

    let cli = Cli::parse();

    let api_key = if cli.api_key.is_empty() {
        None
    } else {
        Some(cli.api_key.clone())
    };
    let client = GatewayClient::new(cli.gateway_url.clone(), api_key);

    // --- Wait for gateway to be ready (retries for up to 60s) ----------------
    wait_for_health(&client, &cli.gateway_url)?;

    // --- Publish starter contracts -------------------------------------------
    let mut contract_ids: HashMap<String, Uuid> = HashMap::new();
    for name in &cli.contracts {
        let yaml = starter_yaml(name);
        if yaml.is_empty() {
            tracing::warn!(
                name = name.as_str(),
                "no embedded YAML for contract name; skipping"
            );
            continue;
        }
        // The gateway creates the contract with the `name` from the YAML.
        // Use the canonical name from the YAML, not the CLI arg, as the lookup key.
        let canonical = canonical_name(yaml);
        let id = client
            .ensure_contract_published(&canonical, yaml)
            .with_context(|| format!("ensure_contract_published({canonical})"))?;
        contract_ids.insert(canonical.clone(), id);
        tracing::info!(name = canonical, %id, "contract ready");
    }

    if contract_ids.is_empty() {
        anyhow::bail!("no contracts available to seed; exiting");
    }

    let names: Vec<String> = contract_ids.keys().cloned().collect();

    // --- Main seeding loop ---------------------------------------------------
    let mut rng = SmallRng::from_entropy();
    let sleep_per_event = Duration::from_secs_f64(1.0 / cli.rate.max(1) as f64);
    let deadline = Instant::now() + cli.duration;

    let mut sent: u64 = 0;
    let mut passed: u64 = 0;
    let mut failed: u64 = 0;
    let mut quarantined: u64 = 0;
    // Simple latency tracking: store all round-trip times and compute p99 at end.
    let mut latencies_ms: Vec<u64> =
        Vec::with_capacity((cli.duration.as_secs_f64() * cli.rate as f64).ceil() as usize + 64);

    tracing::info!(
        rate = cli.rate,
        duration_secs = cli.duration.as_secs(),
        contracts = ?names,
        "starting seeder loop"
    );

    while Instant::now() < deadline {
        let name = &names[rng.gen_range(0..names.len())];
        let contract_id = contract_ids[name];
        let outcome = roll(&mut rng, cli.pass_pct, cli.fail_pct);
        let event = generate(name, outcome, &mut rng);

        match client.post_event(contract_id, &event) {
            Ok((gateway_passed, rt_ms)) => {
                sent += 1;
                latencies_ms.push(rt_ms);
                match outcome {
                    Outcome::Pass => {
                        if gateway_passed {
                            passed += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    Outcome::Fail => {
                        failed += 1;
                    }
                    Outcome::Quarantine => {
                        quarantined += 1;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "event post failed; continuing");
            }
        }

        let elapsed_this_iter = Instant::now();
        let _ = elapsed_this_iter; // used implicitly via sleep below
        std::thread::sleep(sleep_per_event);
    }

    // --- Summary -------------------------------------------------------------
    latencies_ms.sort_unstable();
    let p99_ms = percentile(&latencies_ms, 0.99);
    let p50_ms = percentile(&latencies_ms, 0.50);

    println!();
    println!("=== demo-seeder complete ===");
    println!("  events sent:    {sent}");
    println!("  passed:         {passed}");
    println!("  failed:         {failed}");
    println!("  quarantined:    {quarantined}");
    println!("  p50 latency:    {p50_ms}ms");
    println!("  p99 latency:    {p99_ms}ms");
    println!("  gateway:        {}", cli.gateway_url);
    println!("  contracts:      {}", names.join(", "));
    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Block until GET /health returns 200, or until 60s elapses.
fn wait_for_health(_client: &GatewayClient, base_url: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let poll_interval = Duration::from_secs(2);
    tracing::info!(base_url = base_url, "waiting for gateway to be healthy");
    loop {
        let health_url = format!("{base_url}/health");
        match reqwest::blocking::get(&health_url) {
            Ok(r) if r.status().is_success() => {
                tracing::info!("gateway is healthy");
                return Ok(());
            }
            Ok(r) => tracing::debug!(status = %r.status(), "gateway not yet healthy"),
            Err(e) => tracing::debug!(error = %e, "gateway health check failed"),
        }
        if Instant::now() >= deadline {
            anyhow::bail!("gateway did not become healthy within 60s at {base_url}");
        }
        std::thread::sleep(poll_interval);
    }
}

/// Extract the `name:` field from a YAML string (fast naive parse for starters).
fn canonical_name(yaml: &str) -> String {
    for line in yaml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name:") {
            return rest.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

/// Compute the p-th percentile of a sorted slice.  Returns 0 for empty slices.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
