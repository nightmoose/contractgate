//! Ingestion API handlers — `POST /ingest/{contract_id}`.
//!
//! Accepts a single JSON event or a batch (array) and validates each event
//! against the named contract + resolved version.  On success the event(s)
//! are forwarded to the configured destination.  On failure, clear violation
//! details are returned and the event is quarantined in the audit log.
//!
//! ### Version resolution (RFC-002)
//! Order of precedence for picking which `contract_versions` row to use:
//!
//!   1. `X-Contract-Version: 1.2.3` header  (highest priority)
//!   2. Path suffix `/ingest/{contract_id}@1.2.3`
//!   3. Latest `stable` version by `promoted_at DESC` (`strict` default)
//!
//! If both header and path-suffix are supplied, the header wins and a warn
//! line is logged.  If the resolved version is `deprecated`, the entire
//! batch is quarantined wholesale with the pinned version on the audit row
//! (RFC-002 §5).  If no stable exists and no pin was provided, the handler
//! returns `409 NoStableVersion`.
//!
//! ### Multi-stable resolution (`multi_stable_resolution` flag)
//! When no pin is provided and the contract opts into
//! `multi_stable_resolution = 'fallback'`, each event that fails the
//! latest-stable validation is re-validated against the remaining `stable`
//! versions in parallel (rayon) in `promoted_at DESC` order.  The **first**
//! stable that accepts the event wins — the audit row records that
//! version, not the default latest-stable (audit honesty).  Deprecated and
//! draft versions are **never** fallback candidates.
//!
//! ### HTTP status codes
//! - **200 OK**               — all events passed validation
//! - **207 Multi-Status**     — batch had a mix of passed and failed events
//! - **422 Unprocessable**    — all events failed OR atomic+any-fail OR
//!                              deprecated-pin batch quarantine
//! - **409 Conflict**         — unpinned request on a contract with no
//!                              stable version yet
//! - **400 Bad Request**      — malformed `contract_id`, empty batch,
//!                              oversized batch
//! - **404 Not Found**        — pinned version doesn't exist on the contract
//!
//! ### Query parameters
//! - `?dry_run=true` — validate without writing to the database
//! - `?atomic=true`  — all-or-nothing batch semantics.  If ANY event in the
//!                     batch fails, the entire batch is rejected (422) and
//!                     no events are persisted or forwarded.
//!
//! ### Batch size cap
//! Up to **1 000 events** per request.  Above that, 400.  Parallel
//! validation is done in a `spawn_blocking` worker so the async reactor
//! stays responsive (see RFC-001).

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

use crate::contract::{ContractIdentity, MultiStableResolution, VersionState};
use crate::error::{AppError, AppResult};
use crate::storage;
use crate::validation::{validate, CompiledContract, ValidationResult};
use crate::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of events accepted in a single ingest request.
pub const MAX_BATCH_SIZE: usize = 1_000;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    #[serde(default)]
    pub dry_run: bool,

    #[serde(default)]
    pub atomic: bool,
}

#[derive(Debug, Serialize)]
pub struct IngestEventResult {
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    pub validation_us: u64,
    pub forwarded: bool,
    /// The version that actually produced the decision for this event.  Under
    /// `fallback` mode this is whichever stable first accepted the event (or
    /// the default latest-stable if nothing matched).  Under `strict` mode
    /// this always equals the resolved version.
    pub contract_version: String,
}

#[derive(Debug, Serialize)]
pub struct BatchIngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub atomic: bool,
    /// Resolved version the request was dispatched against (before any
    /// fallback).  Mirrors what got logged to tracing.
    pub resolved_version: String,
    /// Where the resolved version came from: `"header"`, `"path"`, or
    /// `"default_stable"`.
    pub version_pin_source: String,
    pub results: Vec<IngestEventResult>,
}

// ---------------------------------------------------------------------------
// Path parsing — `/ingest/{contract_id}` or `/ingest/{contract_id}@{version}`
// ---------------------------------------------------------------------------

/// Split the path parameter into `(uuid, optional_version)`.  The `@version`
/// suffix is an RFC-002 fallback for clients that can't set headers.
fn parse_ingest_path(raw: &str) -> AppResult<(Uuid, Option<String>)> {
    let (uuid_part, path_version) = match raw.split_once('@') {
        Some((id, v)) if !v.is_empty() => (id, Some(v.to_string())),
        // Empty suffix (e.g. `<uuid>@`) is treated as "no pin" — but the
        // UUID still lives before the `@`, so split on that boundary.
        Some((id, _)) => (id, None),
        None => (raw, None),
    };
    let uuid = Uuid::parse_str(uuid_part).map_err(|_| {
        AppError::BadRequest(format!(
            "invalid contract_id in path: {uuid_part}"
        ))
    })?;
    Ok((uuid, path_version))
}

/// Resolve the version this request should use and where it came from.
///
/// Returns `(version_string, pin_source)` where `pin_source` is one of
/// `"header"`, `"path"`, or `"default_stable"`.
async fn resolve_version(
    state: &AppState,
    contract_id: Uuid,
    header_version: Option<String>,
    path_version: Option<String>,
) -> AppResult<(String, &'static str)> {
    if header_version.is_some() && path_version.is_some() {
        tracing::warn!(
            "both X-Contract-Version header and @version path suffix provided; header wins"
        );
    }
    if let Some(v) = header_version {
        return Ok((v, "header"));
    }
    if let Some(v) = path_version {
        return Ok((v, "path"));
    }
    // Unpinned: resolve to latest stable.
    let latest = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok((latest.version, "default_stable"))
}

// ---------------------------------------------------------------------------
// Handler: POST /ingest/{contract_id}[@version]
// ---------------------------------------------------------------------------

pub async fn ingest_handler(
    State(state): State<Arc<AppState>>,
    Path(raw_id): Path<String>,
    Query(query): Query<IngestQuery>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<axum::response::Response> {
    // --- Parse path + headers -----------------------------------------------
    let (contract_id, path_version) = parse_ingest_path(&raw_id)?;
    let header_version = headers
        .get("x-contract-version")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // --- Load contract identity (404 if unknown) ---------------------------
    let identity: ContractIdentity =
        storage::get_contract_identity(&state.db, contract_id).await?;

    // --- Resolve which version to use --------------------------------------
    let (resolved_version, pin_source) =
        resolve_version(&state, contract_id, header_version, path_version).await?;

    // --- Fetch version row so we know its state (draft/stable/deprecated) --
    let version_row =
        storage::get_version(&state.db, contract_id, &resolved_version).await?;

    tracing::debug!(
        contract_id = %contract_id,
        version = %resolved_version,
        pin_source = pin_source,
        state = version_row.state.as_str(),
        "ingest request routed"
    );

    // --- Normalise body to a batch -----------------------------------------
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

    // --- Capture source IP --------------------------------------------------
    let source_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    // --- Deprecated-pin short-circuit (RFC-002 §5) --------------------------
    //
    // Any traffic that resolves to a deprecated version is rejected wholesale:
    // no per-event validation is run, every event is quarantined under the
    // pinned version, and a single batch_rejected audit row records the pin.
    if version_row.state == VersionState::Deprecated {
        let latest_stable = storage::get_latest_stable_version(&state.db, contract_id)
            .await?
            .map(|v| v.version);
        return deprecated_pin_quarantine(
            &state,
            contract_id,
            &resolved_version,
            latest_stable,
            events,
            source_ip,
            query,
        )
        .await;
    }

    // --- Compile (or pull from cache) the resolved version -----------------
    let compiled = state
        .get_compiled(contract_id, &resolved_version)
        .await?;

    // --- First-pass validation against the resolved version ----------------
    let validation_results: Vec<ValidationResult> =
        parallel_validate(Arc::clone(&compiled), &events).await?;

    // --- Optional fallback retry ------------------------------------------
    //
    // Only applies when:
    //   - contract's resolution policy is `fallback`
    //   - this was an *unpinned* request (default_stable) — RFC §2b
    //   - there are other stables to try
    //   - there was at least one failure in the first pass
    let mut per_event_versions: Vec<String> =
        vec![resolved_version.clone(); events.len()];
    let mut effective_results = validation_results;

    let fallback_eligible = identity.multi_stable_resolution == MultiStableResolution::Fallback
        && pin_source == "default_stable"
        && effective_results.iter().any(|r| !r.passed);

    if fallback_eligible {
        // Get all stables ordered by promoted_at DESC and skip the one we
        // already tried (the latest).
        let stables = storage::list_stable_versions(&state.db, contract_id).await?;
        let other_stables: Vec<_> = stables
            .into_iter()
            .filter(|v| v.version != resolved_version)
            .collect();

        if !other_stables.is_empty() {
            tracing::debug!(
                contract_id = %contract_id,
                other_stable_count = other_stables.len(),
                "running fallback retry for failed events"
            );

            // Compile each fallback candidate once up front (cache hits
            // whenever they're already loaded).
            let mut compiled_fallbacks: Vec<(String, Arc<CompiledContract>)> =
                Vec::with_capacity(other_stables.len());
            for v in &other_stables {
                let cc = state.get_compiled(contract_id, &v.version).await?;
                compiled_fallbacks.push((v.version.clone(), cc));
            }

            // For each failing event, try fallbacks in order; first that
            // passes wins.
            for (idx, result) in effective_results.iter_mut().enumerate() {
                if result.passed {
                    continue;
                }
                let event = &events[idx];
                // Parallel fan-out across fallbacks; pick the first that
                // passes (order preserved by Vec iteration).
                let candidate_results: Vec<(String, ValidationResult)> = compiled_fallbacks
                    .par_iter()
                    .map(|(ver, cc)| (ver.clone(), validate(cc, event)))
                    .collect();

                if let Some((winning_version, winning_vr)) = candidate_results
                    .into_iter()
                    .find(|(_, vr)| vr.passed)
                {
                    // Replace this event's result + recorded version.
                    *result = winning_vr;
                    per_event_versions[idx] = winning_version;
                }
                // If nothing matched, leave the original (latest-stable)
                // failure in place — audit records the default.
            }
        }
    }

    // --- Assemble per-event results + roll-up counts -----------------------
    let total = events.len();
    let mut passed_count = 0usize;
    let mut failed_indices: Vec<usize> = Vec::new();
    let mut per_event_results: Vec<IngestEventResult> = Vec::with_capacity(total);

    for (idx, vr) in effective_results.iter().enumerate() {
        if vr.passed {
            passed_count += 1;
        } else {
            failed_indices.push(idx);
        }
        per_event_results.push(IngestEventResult {
            passed: vr.passed,
            violations: vr.violations.clone(),
            validation_us: vr.validation_us,
            forwarded: false,
            contract_version: per_event_versions[idx].clone(),
        });
    }
    let failed_count = total - passed_count;

    // --- Atomic-mode short-circuit ----------------------------------------
    let atomic_rejected = query.atomic && failed_count > 0;
    if atomic_rejected && !query.dry_run {
        write_batch_rejected_audit(
            &state,
            contract_id,
            &resolved_version, // atomic rejection is charged to the resolved version
            total,
            &failed_indices,
            &effective_results,
            source_ip.as_deref(),
        );
    }

    // --- Per-event persistence --------------------------------------------
    let should_persist_per_event = !query.dry_run && !atomic_rejected;

    if should_persist_per_event {
        let audit_rows: Vec<storage::AuditEntryInsert> = effective_results
            .iter()
            .enumerate()
            .zip(events.iter())
            .map(|((idx, vr), event)| storage::AuditEntryInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                passed: vr.passed,
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                raw_event: event.clone(),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
            })
            .collect();

        let quarantine_rows: Vec<storage::QuarantineEventInsert> = effective_results
            .iter()
            .enumerate()
            .zip(events.iter())
            .filter(|((_, vr), _)| !vr.passed)
            .map(|((idx, vr), event)| storage::QuarantineEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: event.clone(),
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
            })
            .collect();

        let forward_rows: Vec<storage::ForwardEventInsert> = effective_results
            .iter()
            .enumerate()
            .zip(events.iter())
            .filter(|((_, vr), _)| vr.passed)
            .map(|((idx, _vr), event)| storage::ForwardEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: event.clone(),
            })
            .collect();

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
                if let Err(e) =
                    storage::quarantine_events_batch(&pool, &quarantine_rows).await
                {
                    tracing::warn!("Failed to batch-quarantine events: {:?}", e);
                }
            });
        }
        if !forward_rows.is_empty() {
            let pool = state.db.clone();
            let row_count = forward_rows.len();
            let forward_ok = storage::forward_events_batch(&pool, &forward_rows)
                .await
                .is_ok();
            if !forward_ok {
                tracing::warn!("Failed to batch-forward {} events", row_count);
            } else {
                for (result, vr) in per_event_results.iter_mut().zip(effective_results.iter())
                {
                    if vr.passed {
                        result.forwarded = true;
                    }
                }
            }
        }
    }

    // --- Compose response --------------------------------------------------
    let response_body = BatchIngestResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        dry_run: query.dry_run,
        atomic: query.atomic,
        resolved_version: resolved_version.clone(),
        version_pin_source: pin_source.to_string(),
        results: per_event_results,
    };

    let status = if atomic_rejected {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if failed_count == 0 {
        StatusCode::OK
    } else if passed_count == 0 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::MULTI_STATUS
    };

    Ok((status, Json(response_body)).into_response())
}

// ---------------------------------------------------------------------------
// Parallel validation helper (rayon in a blocking worker)
// ---------------------------------------------------------------------------

async fn parallel_validate(
    compiled: Arc<CompiledContract>,
    events: &[Value],
) -> AppResult<Vec<ValidationResult>> {
    let events = events.to_vec();
    tokio::task::spawn_blocking(move || {
        events.par_iter().map(|e| validate(&compiled, e)).collect()
    })
    .await
    .map_err(|e| AppError::Internal(format!("Validation task join error: {e}")))
}

// ---------------------------------------------------------------------------
// Deprecated-pin wholesale quarantine (RFC-002 §5)
// ---------------------------------------------------------------------------

/// Write every event in `events` to `quarantine_events` with a synthetic
/// `deprecated_contract_version` violation, plus a single `batch_rejected`
/// audit row tagged with the pinned (deprecated) version.  Returns a 422
/// with per-event details so the client can identify which batch was
/// rejected and why.
#[allow(clippy::too_many_arguments)]
async fn deprecated_pin_quarantine(
    state: &AppState,
    contract_id: Uuid,
    pinned_version: &str,
    latest_stable: Option<String>,
    events: Vec<Value>,
    source_ip: Option<String>,
    query: IngestQuery,
) -> AppResult<axum::response::Response> {
    let total = events.len();
    let synthetic_violation = json!({
        "kind": "deprecated_contract_version",
        "pinned_version": pinned_version,
        "latest_stable": latest_stable,
    });

    // Per-event synthetic results so the response is shaped consistently
    // with the happy path.
    let per_event_results: Vec<IngestEventResult> = events
        .iter()
        .map(|_| IngestEventResult {
            passed: false,
            violations: vec![],
            validation_us: 0,
            forwarded: false,
            contract_version: pinned_version.to_string(),
        })
        .collect();

    if !query.dry_run {
        // Bulk-quarantine every event under the pinned (deprecated) version.
        let quarantine_rows: Vec<storage::QuarantineEventInsert> = events
            .iter()
            .map(|event| storage::QuarantineEventInsert {
                contract_id,
                contract_version: pinned_version.to_string(),
                payload: event.clone(),
                violation_count: 1,
                violation_details: Value::Array(vec![synthetic_violation.clone()]),
                validation_us: 0,
                source_ip: source_ip.clone(),
            })
            .collect();

        let pool_q = state.db.clone();
        let q_rows = quarantine_rows.clone();
        tokio::spawn(async move {
            if let Err(e) = storage::quarantine_events_batch(&pool_q, &q_rows).await {
                tracing::warn!("Failed to batch-quarantine deprecated-pin events: {:?}", e);
            }
        });

        // One summary audit row documenting the wholesale rejection.
        let pool_a = state.db.clone();
        let pinned_v = pinned_version.to_string();
        let source_ip_owned = source_ip.clone();
        let summary_raw = json!({
            "batch_rejected": true,
            "reason": "deprecated_contract_version",
            "batch_size": total,
            "pinned_version": pinned_version,
            "latest_stable": latest_stable,
        });
        let violation_details =
            Value::Array(vec![json!({
                "kind": "deprecated_contract_version",
                "batch_size": total,
                "pinned_version": pinned_version,
                "latest_stable": latest_stable,
            })]);

        tokio::spawn(async move {
            if let Err(e) = storage::log_audit_entry(
                &pool_a,
                contract_id,
                &pinned_v,
                false,
                1,
                violation_details,
                summary_raw,
                0,
                source_ip_owned.as_deref(),
            )
            .await
            {
                tracing::warn!(
                    "Failed to write deprecated-pin batch_rejected audit row for contract {}: {:?}",
                    contract_id,
                    e
                );
            }
        });
    }

    let response_body = BatchIngestResponse {
        total,
        passed: 0,
        failed: total,
        dry_run: query.dry_run,
        atomic: query.atomic,
        resolved_version: pinned_version.to_string(),
        version_pin_source: "pinned_deprecated".to_string(),
        results: per_event_results,
    };

    let body = json!({
        "error": format!(
            "Version {} on contract {} is deprecated; batch quarantined.",
            pinned_version, contract_id,
        ),
        "status": StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
        "batch": response_body,
    });

    Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

// ---------------------------------------------------------------------------
// Atomic-mode summary audit entry
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_batch_rejected_audit(
    state: &AppState,
    contract_id: Uuid,
    contract_version: &str,
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
    let version = contract_version.to_string();
    let violation_count = failed_indices.len() as i32;
    let details_value = Value::Array(details);

    tokio::spawn(async move {
        if let Err(e) = storage::log_audit_entry(
            &pool,
            contract_id,
            &version,
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
    // Verify contract exists (clean 404 if not).
    let _ = storage::get_contract_identity(&state.db, contract_id).await?;
    let stats = storage::ingestion_stats(&state.db, Some(contract_id)).await?;
    Ok(Json(stats))
}

// ---------------------------------------------------------------------------
// Tests for path parsing (pure function, no DB needed)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn parses_plain_uuid() {
        let uuid = Uuid::new_v4();
        let raw = uuid.to_string();
        let (parsed, v) = parse_ingest_path(&raw).unwrap();
        assert_eq!(parsed, uuid);
        assert_eq!(v, None);
    }

    #[test]
    fn parses_uuid_with_version_suffix() {
        let uuid = Uuid::new_v4();
        let raw = format!("{uuid}@1.2.3");
        let (parsed, v) = parse_ingest_path(&raw).unwrap();
        assert_eq!(parsed, uuid);
        assert_eq!(v.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn rejects_bad_uuid() {
        let raw = "not-a-uuid@1.0.0";
        assert!(parse_ingest_path(raw).is_err());
    }

    #[test]
    fn empty_version_suffix_is_treated_as_no_pin() {
        let uuid = Uuid::new_v4();
        let raw = format!("{uuid}@");
        let (parsed, v) = parse_ingest_path(&raw).unwrap();
        assert_eq!(parsed, uuid);
        assert_eq!(v, None);
    }
}
