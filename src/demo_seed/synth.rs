//! Synthetic payload generator — produces realistic-looking events for each
//! starter contract shape.
//!
//! Three generators, one per starter:
//!   - `rest_event`    — HTTP request log (method / path / status / latency)
//!   - `kafka_event`   — Kafka message metadata (topic / partition / offset)
//!   - `dbt_model_row` — dbt row (id / timestamps / source_system)
//!
//! Each generator takes an `Outcome` and produces a payload that is
//! guaranteed to pass or fail accordingly:
//!   - `Pass`       — all fields valid per the starter contract
//!   - `Fail`       — one field violates a constraint (wrong enum value,
//!                    out-of-range int, bad pattern match)
//!   - `Quarantine` — a required field is absent entirely (guaranteed miss)

use super::outcome::Outcome;
use rand::{rngs::SmallRng, Rng};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// rest_event
// ---------------------------------------------------------------------------

const REST_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];
const REST_PATHS: &[&str] = &[
    "/api/users",
    "/api/users/:id",
    "/api/orders",
    "/api/orders/:id",
    "/api/products",
    "/api/health",
    "/api/auth/token",
];

pub fn rest_event(rng: &mut SmallRng, outcome: Outcome) -> Value {
    let request_id = Uuid::new_v4().to_string();
    let method = REST_METHODS[rng.gen_range(0..REST_METHODS.len())];
    let path = REST_PATHS[rng.gen_range(0..REST_PATHS.len())];
    // Gamma-ish latency: mostly 5–200ms, occasional spike.
    let latency_ms: u64 = {
        let base: f64 = rng.gen_range(5.0..200.0_f64);
        if rng.gen_bool(0.05) {
            (base * 10.0) as u64
        } else {
            base as u64
        }
    };
    let timestamp: u64 = 1_700_000_000 + rng.gen_range(0..86_400_000u64);

    match outcome {
        Outcome::Pass => {
            let status: u16 = *[200u16, 200, 200, 201, 204, 400, 404, 500]
                .iter()
                .nth(rng.gen_range(0..8))
                .unwrap();
            json!({
                "request_id": request_id,
                "method": method,
                "path": path,
                "status": status,
                "latency_ms": latency_ms,
                "timestamp": timestamp
            })
        }
        Outcome::Fail => {
            // Enum violation: invalid HTTP method
            json!({
                "request_id": request_id,
                "method": "CONNECT",
                "path": path,
                "status": 200,
                "latency_ms": latency_ms,
                "timestamp": timestamp
            })
        }
        Outcome::Quarantine => {
            // Missing required field: request_id absent
            json!({
                "method": method,
                "path": path,
                "status": 200,
                "latency_ms": latency_ms,
                "timestamp": timestamp
            })
        }
    }
}

// ---------------------------------------------------------------------------
// kafka_event
// ---------------------------------------------------------------------------

const KAFKA_TOPICS: &[&str] = &[
    "payments.processed",
    "orders.created",
    "users.updated",
    "inventory.adjusted",
    "analytics.page_view",
];
const PRODUCER_IDS: &[&str] = &[
    "payments-svc",
    "orders-svc",
    "users-svc",
    "inventory-svc",
    "analytics-svc",
];

pub fn kafka_event(rng: &mut SmallRng, outcome: Outcome) -> Value {
    let topic = KAFKA_TOPICS[rng.gen_range(0..KAFKA_TOPICS.len())];
    let partition: u32 = rng.gen_range(0..12);
    let offset: u64 = rng.gen_range(0..10_000_000);
    let producer_id = PRODUCER_IDS[rng.gen_range(0..PRODUCER_IDS.len())];
    let timestamp: u64 = 1_700_000_000 + rng.gen_range(0..86_400_000u64);

    match outcome {
        Outcome::Pass => {
            let key: Option<String> = if rng.gen_bool(0.7) {
                Some(format!("key_{:08x}", rng.gen::<u32>()))
            } else {
                None
            };
            let mut ev = json!({
                "topic": topic,
                "partition": partition,
                "offset": offset,
                "producer_id": producer_id,
                "timestamp": timestamp
            });
            if let Some(k) = key {
                ev["key"] = json!(k);
            }
            ev
        }
        Outcome::Fail => {
            // Pattern violation: spaces in producer_id
            json!({
                "topic": topic,
                "partition": partition,
                "offset": offset,
                "producer_id": "bad producer id!",
                "timestamp": timestamp
            })
        }
        Outcome::Quarantine => {
            // Missing required: topic absent
            json!({
                "partition": partition,
                "offset": offset,
                "producer_id": producer_id,
                "timestamp": timestamp
            })
        }
    }
}

// ---------------------------------------------------------------------------
// dbt_model_row
// ---------------------------------------------------------------------------

const SOURCE_SYSTEMS: &[&str] = &["postgres", "mysql", "snowflake", "bigquery"];

pub fn dbt_model_row(rng: &mut SmallRng, outcome: Outcome) -> Value {
    let id = Uuid::new_v4().to_string();
    let created_at: u64 = 1_700_000_000 + rng.gen_range(0..86_400_000u64);
    let updated_at: u64 = created_at + rng.gen_range(0..3600u64);
    let source_system = SOURCE_SYSTEMS[rng.gen_range(0..SOURCE_SYSTEMS.len())];

    match outcome {
        Outcome::Pass => {
            let deleted_at: Option<u64> = if rng.gen_bool(0.1) {
                Some(updated_at + rng.gen_range(1..3600))
            } else {
                None
            };
            let mut ev = json!({
                "id": id,
                "created_at": created_at,
                "updated_at": updated_at,
                "source_system": source_system
            });
            if let Some(d) = deleted_at {
                ev["deleted_at"] = json!(d);
            }
            ev
        }
        Outcome::Fail => {
            // Enum violation: invalid source_system
            json!({
                "id": id,
                "created_at": created_at,
                "updated_at": updated_at,
                "source_system": "oracle"
            })
        }
        Outcome::Quarantine => {
            // Missing required: source_system absent
            json!({
                "id": id,
                "created_at": created_at,
                "updated_at": updated_at
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Generate a synthetic event for the named contract.
///
/// `contract_name` must match the `name` field in the starter YAML exactly.
/// Unknown names fall back to a minimal JSON object that will fail validation.
pub fn generate(contract_name: &str, outcome: Outcome, rng: &mut SmallRng) -> Value {
    match contract_name {
        "rest_event" => rest_event(rng, outcome),
        "kafka_event" => kafka_event(rng, outcome),
        "dbt_model_row" => dbt_model_row(rng, outcome),
        other => {
            tracing::warn!(
                contract = other,
                "unknown contract name in synth::generate — emitting empty object"
            );
            json!({})
        }
    }
}
