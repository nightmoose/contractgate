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
//! - **422 Unprocessable**    — all events failed validation
//!
//! ### Query parameters
//! - `?dry_run=true` — validate without writing to the database

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::storage;
use crate::validation::validate;
use crate::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters accepted by the ingest endpoint.
#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    /// When true: validate only — do not write audit log, quarantine, or forward.
    #[serde(default)]
    pub dry_run: bool,
}

/// The result of validating (and optionally forwarding) a single event.
#[derive(Debug, Serialize)]
pub struct IngestEventResult {
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    /// Validation time in microseconds
    pub validation_us: u64,
    /// Whether the event was forwarded to the downstream destination.
    /// Always false when `dry_run=true`.
    pub forwarded: bool,
}

/// Response for a batch ingestion request.
#[derive(Debug, Serialize)]
pub struct BatchIngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    /// true when dry_run=true was set; no data was persisted
    pub dry_run: bool,
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

    // Hard cap: prevent memory exhaustion from oversized batches
    const MAX_BATCH_SIZE: usize = 500;
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

    // --- Validate each event ---
    let mut results = Vec::with_capacity(events.len());
    let mut passed_count = 0usize;

    for event in &events {
        let vr = validate(&compiled, event);
        let passed = vr.passed;
        let violation_count = vr.violations.len() as i32;
        let violation_json = serde_json::to_value(&vr.violations).unwrap_or(Value::Array(vec![]));

        if !query.dry_run {
            // --- Write audit log (fire-and-forget; don't block response) ---
            {
                let pool = state.db.clone();
                let event_clone = event.clone();
                let violation_json_clone = violation_json.clone();
                let source_ip_clone = source_ip.clone();
                let validation_us = vr.validation_us as i64;

                tokio::spawn(async move {
                    if let Err(e) = storage::log_audit_entry(
                        &pool,
                        contract_id,
                        passed,
                        violation_count,
                        violation_json_clone,
                        event_clone,
                        validation_us,
                        source_ip_clone.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!("Failed to write audit log: {:?}", e);
                    }
                });
            }

            // --- Quarantine failed events (fire-and-forget) ---
            if !passed {
                let pool = state.db.clone();
                let event_clone = event.clone();
                let violation_json_clone = violation_json.clone();
                let source_ip_clone = source_ip.clone();
                let validation_us = vr.validation_us as i64;

                tokio::spawn(async move {
                    if let Err(e) = storage::quarantine_event(
                        &pool,
                        contract_id,
                        event_clone,
                        violation_count,
                        violation_json_clone,
                        validation_us,
                        source_ip_clone.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!(
                            "Failed to quarantine event for contract {}: {:?}",
                            contract_id,
                            e
                        );
                    }
                });
            }
        }

        // --- Forward passing events to configured destination ---
        let forwarded = if passed && !query.dry_run {
            forward_event(&state, contract_id, event).await
        } else {
            false
        };

        if passed {
            passed_count += 1;
        }

        results.push(IngestEventResult {
            passed: vr.passed,
            violations: vr.violations,
            validation_us: vr.validation_us,
            forwarded,
        });
    }

    let total = events.len();
    let failed = total - passed_count;

    let response_body = BatchIngestResponse {
        total,
        passed: passed_count,
        failed,
        dry_run: query.dry_run,
        results,
    };

    // Choose appropriate HTTP status:
    //   200 — all events passed
    //   207 — partial success (some passed, some failed)
    //   422 — all events failed
    let status = if failed == 0 {
        StatusCode::OK
    } else if passed_count == 0 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::MULTI_STATUS
    };

    Ok((status, Json(response_body)))
}

// ---------------------------------------------------------------------------
// Destination forwarding
// ---------------------------------------------------------------------------

/// Forward a validated event to the configured downstream destination.
///
/// Currently inserts into the `forwarded_events` table in Supabase.
/// Webhook destination support is planned per-contract configuration.
///
/// Returns `true` if forwarding succeeded.
async fn forward_event(state: &AppState, contract_id: Uuid, event: &Value) -> bool {
    match sqlx::query(
        r#"
        INSERT INTO forwarded_events (id, contract_id, payload, created_at)
        VALUES ($1, $2, $3, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(event)
    .execute(&state.db)
    .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("Failed to forward event for contract {}: {:?}", contract_id, e);
            false
        }
    }
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
