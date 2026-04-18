//! Ingestion API handlers — POST /ingest/{contract_id}
//!
//! Accepts a single JSON event or a batch (array) and validates each event
//! against the named contract.  On success the event(s) are forwarded to the
//! configured destination.  On failure, clear violation details are returned
//! and the event is quarantined in the audit log.
//!
//! ### HTTP status codes
//! - **200 OK**               — all events passed validation
//! - **207 Multi-Status**     — batch had a mix of passed and failed events
//! - **422 Unprocessable**    — all events failed validation, or `atomic=true`
//!                              and at least one event failed
//!
//! ### Query parameters
//! - `?dry_run=true` — validate without writing to the database
//! - `?atomic=true`  — all-or-nothing batch semantics.  If ANY event in the
//!                     batch fails, the entire batch is rejected (422) and
//!                     no events are persisted or forwarded.  Has no effect
//!                     on single-event bodies (treated as a 1-item batch).
//!
//! ### Batch size cap
//! Up to **1 000 events** per request.  Above that, the request is rejected
//! with a 400.  Validation of the batch is **parallelised** across CPU cores
//! via `rayon`; the parallel stage runs inside `tokio::task::spawn_blocking`
//! so the async reactor stays responsive under load.  See `docs/rfcs/001-batch-ingest.md`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::storage;
use crate::validation::{validate, ValidationResult};
use crate::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of events accepted in a single ingest request.
///
/// Raised from 500 → 1 000 in RFC-001 once batch validation became
/// data-parallel.  Oversized requests are rejected with 400 before any
/// validation work is done so a misbehaving client can't drown the pool.
pub const MAX_BATCH_SIZE: usize = 1_000;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters accepted by the ingest endpoint.
#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    /// When true: validate only — do not write audit log, quarantine, or forward.
    #[serde(default)]
    pub dry_run: bool,

    /// When true: all-or-nothing batch semantics.  If ANY event in the batch
    /// fails validation, the entire batch is rejected with 422 and no data is
    /// persisted or forwarded.  A single `batch_rejected` audit entry is still
    /// recorded so the attempt is visible in the audit log.
    ///
    /// Ignored (no-op) for 1-event bodies — behaviour is identical to
    /// `atomic=false` on a single event.
    #[serde(default)]
    pub atomic: bool,
}

/// The result of validating (and optionally forwarding) a single event.
#[derive(Debug, Serialize)]
pub struct IngestEventResult {
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    /// Validation time in microseconds
    pub validation_us: u64,
    /// Whether the event was forwarded to the downstream destination.
    /// Always false when `dry_run=true` OR when an `atomic=true` batch was rejected.
    pub forwarded: bool,
}

/// Response for a (possibly single-event) ingestion request.
#[derive(Debug, Serialize)]
pub struct BatchIngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    /// true when dry_run=true was set; no data was persisted
    pub dry_run: bool,
    /// true when atomic=true was requested; echoes the flag back to the caller
    pub atomic: bool,
    pub results: Vec<IngestEventResult>,
}

// ---------------------------------------------------------------------------
// Handler: POST /ingest/{contract_id}
// ---------------------------------------------------------------------------

pub async fn ingest_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    Query(query): Query<IngestQuery>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<impl IntoResponse> {
    // --- Load & compile contract (hot-path: cached in state) ---
    let compiled = state
        .get_compiled_contract(contract_id)
        .await
        .ok_or(AppError::ContractNotFound(contract_id))?;

    // --- Normalise to batch ---
    let events: Vec<Value> = match body {
        Value::Array(arr) => arr,
        single => vec![single],
    };

    if events.is_empty() {
        return Err(AppError::BadRequest("Empty event batch".into()));
    }

    if events.len() > MAX_BATCH_SIZE {
        return Err(AppError::BadRequest(format!(
            "Batch too large: {} events submitted, maximum is {}",
            events.len(),
            MAX_BATCH_SIZE
        )));
    }

    let source_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    // --- Parallel validation stage ---
    //
    // We move the rayon fan-out onto a blocking worker so the tokio reactor
    // doesn't stall while N cores chew through a large batch.  Rayon's global
    // pool is shared across requests — calls coalesce naturally under load.
    //
    // `par_iter().map().collect::<Vec<_>>()` preserves input order, which the
    // response contract depends on (results[i] ↔ events[i]).
    let events = Arc::new(events);
    let compiled_for_validation = Arc::clone(&compiled);
    let events_for_validation = Arc::clone(&events);

    let validation_results: Vec<ValidationResult> =
        tokio::task::spawn_blocking(move || {
            events_for_validation
                .par_iter()
                .map(|event| validate(&compiled_for_validation, event))
                .collect()
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Validation task join error: {e}")))?;

    // --- Assemble per-event results + roll-up counts ---
    let total = events.len();
    let mut passed_count = 0usize;
    let mut failed_indices: Vec<usize> = Vec::new();
    let mut per_event_results: Vec<IngestEventResult> = Vec::with_capacity(total);

    for (idx, vr) in validation_results.iter().enumerate() {
        if vr.passed {
            passed_count += 1;
        } else {
            failed_indices.push(idx);
        }
        per_event_results.push(IngestEventResult {
            passed: vr.passed,
            violations: vr.violations.clone(),
            validation_us: vr.validation_us,
            forwarded: false, // populated below once we know what we're doing
        });
    }
    let failed_count = total - passed_count;

    // --- Atomic-mode short-circuit ---
    //
    // If the caller asked for all-or-nothing AND any event failed, bail before
    // any per-event persistence.  One summary audit row is still written so
    // the rejected attempt is visible in the audit log.
    let atomic_rejected = query.atomic && failed_count > 0;

    if atomic_rejected && !query.dry_run {
        write_batch_rejected_audit(
            &state,
            contract_id,
            total,
            &failed_indices,
            &validation_results,
            source_ip.as_deref(),
        );
    }

    // --- Per-event persistence (non-atomic OR atomic-all-pass) ---
    let should_persist_per_event = !query.dry_run && !atomic_rejected;

    if should_persist_per_event {
        // Build batched audit-log inserts for every event in the batch.
        let audit_rows: Vec<storage::AuditEntryInsert> = validation_results
            .iter()
            .zip(events.iter())
            .map(|(vr, event)| storage::AuditEntryInsert {
                contract_id,
                passed: vr.passed,
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                raw_event: event.clone(),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
            })
            .collect();

        // Build batched quarantine inserts for failures.
        let quarantine_rows: Vec<storage::QuarantineEventInsert> = validation_results
            .iter()
            .zip(events.iter())
            .filter(|(vr, _)| !vr.passed)
            .map(|(vr, event)| storage::QuarantineEventInsert {
                contract_id,
                payload: event.clone(),
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
            })
            .collect();

        // Build batched forwarded-event inserts for passing events.
        let forward_rows: Vec<storage::ForwardEventInsert> = validation_results
            .iter()
            .zip(events.iter())
            .filter(|(vr, _)| vr.passed)
            .map(|(_, event)| storage::ForwardEventInsert {
                contract_id,
                payload: event.clone(),
            })
            .collect();

        // Spawn the writes — fire-and-forget so the HTTP response isn't
        // gated on durability.  Each is a single multi-row INSERT regardless
        // of batch size, so the DB pool sees at most 3 extra connections per
        // request rather than 3 × batch_size.
        if !audit_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::log_audit_entries_batch(&pool, &audit_rows).await {
                    tracing::warn!("Failed to write batch audit log: {:?}", e);
                }
            });
        }
        if !quarantine_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::quarantine_events_batch(&pool, &quarantine_rows).await {
                    tracing::warn!("Failed to batch-quarantine events: {:?}", e);
                }
            });
        }
        if !forward_rows.is_empty() {
            let pool = state.db.clone();
            let row_count = forward_rows.len();
            // forwarded_events writes are awaited (not spawned) so we can
            // correctly populate `forwarded: true` in the response.  This
            // is a single query so the added latency is small.
            let forward_ok = storage::forward_events_batch(&pool, &forward_rows)
                .await
                .is_ok();
            if !forward_ok {
                tracing::warn!("Failed to batch-forward {} events", row_count);
            } else {
                // Mark passing events as forwarded in the response.
                for (result, vr) in per_event_results.iter_mut().zip(validation_results.iter()) {
                    if vr.passed {
                        result.forwarded = true;
                    }
                }
            }
        }
    }

    // --- Compose response ---
    let response_body = BatchIngestResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        dry_run: query.dry_run,
        atomic: query.atomic,
        results: per_event_results,
    };

    // Choose appropriate HTTP status:
    //   atomic + any failure → 422  (batch rejected as a unit)
    //   all pass             → 200
    //   all fail             → 422
    //   partial              → 207
    let status = if atomic_rejected {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if failed_count == 0 {
        StatusCode::OK
    } else if passed_count == 0 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::MULTI_STATUS
    };

    Ok((status, Json(response_body)))
}

// ---------------------------------------------------------------------------
// Atomic-mode summary audit entry
// ---------------------------------------------------------------------------

/// Write a single `batch_rejected` row to the audit log summarising an
/// atomic-mode failure.  Fire-and-forget: errors are logged but do not affect
/// the HTTP response.
///
/// The row is constructed so existing audit queries keep working:
///   - `passed = false`
///   - `violation_count = failing_indices.len()`
///   - `violation_details` — JSON array of `{ index, violations: [...] }`
///   - `raw_event` — a small summary object, not an actual event payload
///   - `validation_us` — 0 (per-event timings live in the response, not the summary)
fn write_batch_rejected_audit(
    state: &AppState,
    contract_id: Uuid,
    total: usize,
    failed_indices: &[usize],
    validation_results: &[ValidationResult],
    source_ip: Option<&str>,
) {
    let details: Vec<Value> = failed_indices
        .iter()
        .map(|&idx| {
            json!({
                "index": idx,
                "violations": validation_results[idx].violations,
            })
        })
        .collect();

    let summary = json!({
        "batch_rejected": true,
        "atomic": true,
        "batch_size": total,
        "failing_count": failed_indices.len(),
        "first_failing_index": failed_indices.first().copied(),
    });

    let pool = state.db.clone();
    let source_ip_owned = source_ip.map(|s| s.to_string());
    let violation_count = failed_indices.len() as i32;
    let details_value = Value::Array(details);

    tokio::spawn(async move {
        if let Err(e) = storage::log_audit_entry(
            &pool,
            contract_id,
            false,
            violation_count,
            details_value,
            summary,
            0,
            source_ip_owned.as_deref(),
        )
        .await
        {
            tracing::warn!(
                "Failed to write atomic batch-rejected audit entry for contract {}: {:?}",
                contract_id,
                e
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Handler: GET /ingest/{contract_id}/stats
// ---------------------------------------------------------------------------

pub async fn ingest_stats_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<storage::IngestionStats>> {
    // Verify contract exists (returns 404 if not found)
    let _ = storage::get_contract(&state.db, contract_id)
        .await
        .map_err(|_| AppError::ContractNotFound(contract_id))?;

    let stats = storage::ingestion_stats(&state.db, Some(contract_id)).await?;
    Ok(Json(stats))
}
