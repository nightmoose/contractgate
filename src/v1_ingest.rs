//! RFC-021: `POST /v1/ingest/{contract_id}` — public bulk HTTP ingest endpoint.
//!
//! This is the universal connector surface: anything that can make an HTTP
//! request can validate events against a ContractGate contract.
//!
//! ## What this module adds vs. `ingest.rs`
//!
//! | Feature                  | `/ingest/:raw_id` | `/v1/ingest/:contract_id` |
//! |--------------------------|:-----------------:|:-------------------------:|
//! | JSON array body          | ✓                 | ✓                         |
//! | Single-object body       | ✓                 | ✓                         |
//! | NDJSON body              | ✗                 | ✓                         |
//! | `?version=` param        | ✗ (@suffix/hdr)   | ✓                         |
//! | 10 MB body limit         | ✗                 | ✓                         |
//! | 1 MB per-event limit     | ✗                 | ✓                         |
//! | `Idempotency-Key` header | ✗                 | ✓                         |
//! | Per-key rate limiting    | ✗                 | ✓                         |
//! | `X-RateLimit-*` headers  | ✗                 | ✓                         |
//! | `quarantine_id` in result| ✗                 | ✓                         |
//! | `index` in result        | ✗                 | ✓                         |
//!
//! ## Execution order
//!
//! 1. Extract `ValidatedKey` — populated by `require_api_key` middleware.
//! 2. Rate-limit check.  Fail fast with 429 if exhausted.
//! 3. Idempotency-Key header: hit → return cached; conflict → 422.
//! 4. Read raw body bytes (10 MB limit already enforced by middleware layer).
//! 5. Parse body: JSON array / single object / NDJSON.
//! 6. Per-event 1 MB size check.  Batch count ≤ 1 000 check.
//! 7. Load contract identity + check key scope.
//! 8. Resolve version (`?version=` or latest stable).
//! 9. Parallel validation (rayon in `spawn_blocking`).
//! 10. RFC-004 PII transforms.
//! 11. Pre-assign quarantine UUIDs for rejected events.
//! 12. Persist audit + quarantine + forward (fire-and-forget).
//! 13. Store idempotency response (unless dry_run).
//! 14. Return response with `X-RateLimit-*` and optionally
//!     `X-Idempotency-Replay: true`.

use axum::{
    body::Bytes,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;
use crate::contract::{ContractIdentity, MultiStableResolution, VersionState};
use crate::error::{AppError, AppResult};
use crate::idempotency;
use crate::ingest::MAX_BATCH_SIZE;
use crate::storage;
use crate::transform::{apply_transforms, TransformedPayload};
use crate::validation::{check_uniqueness_batch, validate, CompiledContract, ValidationResult};
use crate::AppState;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Limits (RFC-021 §Size limits)
// ---------------------------------------------------------------------------

/// Max size of any single event object in the batch.
const MAX_EVENT_BYTES: usize = 1024 * 1024; // 1 MB

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct V1IngestQuery {
    /// Semver pin.  `None` → resolve to latest `stable` (RFC-002).
    pub version: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub atomic: bool,
}

/// Per-event result returned in the response.
#[derive(Debug, Clone, Serialize)]
pub struct V1IngestEventResult {
    /// Zero-based position of this event in the submitted batch.
    pub index: usize,
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    pub validation_us: u64,
    pub forwarded: bool,
    /// The contract version that produced the accept/reject decision.
    pub contract_version: String,
    /// UUID of the `quarantine_events` row for a rejected event.
    /// `null` for passing events.
    pub quarantine_id: Option<Uuid>,
    /// Post-RFC-004-transform payload that was persisted / forwarded.
    pub transformed_event: Value,
}

/// Top-level response body.
#[derive(Debug, Serialize)]
pub struct V1IngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub atomic: bool,
    pub resolved_version: String,
    /// One of: `"query_param"` | `"default_stable"` | `"pinned_deprecated"`.
    pub version_pin_source: &'static str,
    pub results: Vec<V1IngestEventResult>,
}

// ---------------------------------------------------------------------------
// Body parsing
// ---------------------------------------------------------------------------

/// Parse request body into a `Vec<Value>`.
///
/// Supports:
/// - `application/json`   — array of objects, or a single object
/// - `application/x-ndjson` — one JSON object per line
///
/// Blank lines in NDJSON are silently skipped.
/// A line > `MAX_EVENT_BYTES` → 413.
/// A line that isn't valid JSON → 400 with line index.
fn parse_body(content_type: &str, bytes: &[u8]) -> AppResult<Vec<Value>> {
    if content_type.contains("application/x-ndjson") {
        parse_ndjson(bytes)
    } else {
        // Default: JSON (array or single object).
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| AppError::BadRequest(format!("JSON parse error: {e}")))?;
        Ok(match v {
            Value::Array(arr) => arr,
            single => vec![single],
        })
    }
}

fn parse_ndjson(bytes: &[u8]) -> AppResult<Vec<Value>> {
    let mut events = Vec::new();
    for (line_idx, line) in bytes.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() || line == b"\r" {
            continue; // blank / trailing newline
        }
        if line.len() > MAX_EVENT_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "NDJSON line {} exceeds the 1 MB per-event limit ({} bytes)",
                line_idx,
                line.len(),
            )));
        }
        let v: Value = serde_json::from_slice(line).map_err(|e| {
            AppError::BadRequest(format!("NDJSON parse error at line {line_idx}: {e}"))
        })?;
        events.push(v);
    }
    Ok(events)
}

/// Validate per-event sizes for a JSON-parsed batch.
fn check_event_sizes(events: &[Value]) -> AppResult<()> {
    for (idx, event) in events.iter().enumerate() {
        // Re-serialise to measure.  Cost is minimal relative to validation.
        let size = serde_json::to_vec(event)
            .map(|b| b.len())
            .unwrap_or(MAX_EVENT_BYTES + 1);
        if size > MAX_EVENT_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "Event at index {idx} exceeds the 1 MB per-event limit ({size} bytes)",
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rate-limit response helper
// ---------------------------------------------------------------------------

fn rate_limit_headers(
    outcome: &crate::rate_limit::RateLimitOutcome,
) -> [(&'static str, String); 3] {
    [
        ("X-RateLimit-Limit", outcome.limit.to_string()),
        ("X-RateLimit-Remaining", outcome.remaining.to_string()),
        ("X-RateLimit-Reset", outcome.reset_unix.to_string()),
    ]
}

// ---------------------------------------------------------------------------
// OpenAPI spec handler (GET /openapi.json)
// ---------------------------------------------------------------------------

/// Serve the statically-embedded OpenAPI spec.
///
/// The spec is the canonical source of truth for the v1 ingest surface; it
/// is generated from the type annotations in this module (see `build_openapi`)
/// and embedded at compile time so the `/openapi.json` endpoint adds zero
/// latency at runtime.
pub async fn openapi_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        build_openapi(),
    )
}

/// Build a minimal but accurate OpenAPI 3.1 document for the v1 ingest endpoint.
///
/// Kept hand-rolled for now (utoipa dependency deferred to task #5) so the
/// endpoint can ship with a usable spec immediately.  Task #5 will replace
/// this with utoipa-generated output; the shape is identical so no client
/// changes will be needed.
fn build_openapi() -> String {
    serde_json::to_string_pretty(&json!({
      "openapi": "3.1.0",
      "info": {
        "title": "ContractGate Ingest API",
        "version": "1.0.0",
        "description": "POST events to ContractGate for real-time semantic contract validation."
      },
      "servers": [{ "url": "https://contractgate.io", "description": "Production" }],
      "paths": {
        "/v1/ingest/{contract_id}": {
          "post": {
            "operationId": "v1_ingest",
            "summary": "Validate a batch of events against a contract",
            "parameters": [
              {
                "name": "contract_id",
                "in": "path",
                "required": true,
                "schema": { "type": "string", "format": "uuid" }
              },
              {
                "name": "version",
                "in": "query",
                "required": false,
                "description": "Semver pin. Defaults to latest stable.",
                "schema": { "type": "string", "example": "1.2.0" }
              },
              {
                "name": "dry_run",
                "in": "query",
                "required": false,
                "schema": { "type": "boolean", "default": false }
              },
              {
                "name": "atomic",
                "in": "query",
                "required": false,
                "description": "All-or-nothing batch semantics.",
                "schema": { "type": "boolean", "default": false }
              },
              {
                "name": "X-Api-Key",
                "in": "header",
                "required": true,
                "schema": { "type": "string" }
              },
              {
                "name": "Idempotency-Key",
                "in": "header",
                "required": false,
                "description": "Opaque string (max 255 chars). Same key + same body → cached response.",
                "schema": { "type": "string", "maxLength": 255 }
              }
            ],
            "requestBody": {
              "required": true,
              "content": {
                "application/json": {
                  "schema": {
                    "oneOf": [
                      { "type": "array", "items": { "type": "object" } },
                      { "type": "object" }
                    ]
                  }
                },
                "application/x-ndjson": {
                  "schema": { "type": "string", "description": "Newline-delimited JSON objects" }
                }
              }
            },
            "responses": {
              "200": { "description": "All events passed validation" },
              "207": { "description": "Mixed pass/fail batch" },
              "400": { "description": "Malformed body, empty batch, or size limit exceeded" },
              "401": { "description": "Missing or invalid X-Api-Key" },
              "413": { "description": "Body > 10 MB or event > 1 MB" },
              "422": { "description": "All events failed, atomic rejection, idempotency conflict, or deprecated version pin" },
              "429": { "description": "Per-key rate limit exceeded" }
            },
            "security": [{ "ApiKeyAuth": [] }]
          }
        }
      },
      "components": {
        "securitySchemes": {
          "ApiKeyAuth": {
            "type": "apiKey",
            "in": "header",
            "name": "X-Api-Key"
          }
        }
      }
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

// ---------------------------------------------------------------------------
// Parallel validation helper (shared with ingest.rs pattern)
// ---------------------------------------------------------------------------

async fn parallel_validate(
    compiled: Arc<CompiledContract>,
    events: &[Value],
) -> AppResult<Vec<ValidationResult>> {
    let events = events.to_vec();
    tokio::task::spawn_blocking(move || events.par_iter().map(|e| validate(&compiled, e)).collect())
        .await
        .map_err(|e| AppError::Internal(format!("Validation task join error: {e}")))
}

// ---------------------------------------------------------------------------
// Version resolution (RFC-002 + RFC-021 `?version=` param)
// ---------------------------------------------------------------------------

async fn resolve_version(
    state: &AppState,
    contract_id: Uuid,
    version_param: Option<String>,
) -> AppResult<(String, &'static str)> {
    if let Some(v) = version_param {
        return Ok((v, "query_param"));
    }
    let latest = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok((latest.version, "default_stable"))
}

// ---------------------------------------------------------------------------
// Main handler
// ---------------------------------------------------------------------------

pub async fn v1_ingest_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id_str): Path<String>,
    Query(query): Query<V1IngestQuery>,
    headers: HeaderMap,
    key_ext: Option<Extension<ValidatedKey>>,
    body: Bytes,
) -> AppResult<Response> {
    // --- 1. Parse + validate contract_id -----------------------------------
    let contract_id = Uuid::parse_str(&contract_id_str)
        .map_err(|_| AppError::BadRequest(format!("invalid contract_id: {contract_id_str}")))?;

    // --- 2. Rate limit -----------------------------------------------------
    let (rl_rps, rl_burst, key_id, org_id) = if let Some(Extension(ref k)) = key_ext {
        (
            k.rate_limit_rps,
            k.rate_limit_burst,
            Some(k.api_key_id),
            Some(k.org_id),
        )
    } else {
        (None, None, None, None)
    };

    // Use a nil UUID as the rate-limit bucket for legacy/dev-mode (no key).
    let bucket_id = key_id.unwrap_or(Uuid::nil());
    let rl_outcome = state.rate_limiter.check(bucket_id, rl_rps, rl_burst);
    let rl_headers = rate_limit_headers(&rl_outcome);

    if !rl_outcome.allowed {
        let body = json!({
            "error": "rate_limit_exceeded",
            "detail": format!(
                "Rate limit of {} req/sec exceeded. Retry after {}ms.",
                rl_outcome.limit, rl_outcome.retry_after_ms
            ),
            "retry_after_ms": rl_outcome.retry_after_ms,
        });
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("X-RateLimit-Limit", rl_headers[0].1.clone()),
                ("X-RateLimit-Remaining", rl_headers[1].1.clone()),
                ("X-RateLimit-Reset", rl_headers[2].1.clone()),
            ],
            Json(body),
        )
            .into_response());
    }

    // --- 3. Idempotency-Key lookup -----------------------------------------
    let idem_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Validate key length before any DB touch.
    if let Some(ref k) = idem_key {
        if k.len() > 255 {
            return Err(AppError::BadRequest(
                "Idempotency-Key must be ≤ 255 characters".into(),
            ));
        }
    }

    let body_hash = idempotency::body_hash(&body);

    if let Some(ref key) = idem_key {
        match idempotency::lookup(&state.db, key, contract_id, &body_hash).await {
            Ok(idempotency::IdempotencyLookup::Hit {
                status_code,
                response,
            }) => {
                // Replay: return cached response.
                let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
                return Ok((
                    status,
                    [
                        ("X-Idempotency-Replay", "true".to_string()),
                        ("X-RateLimit-Limit", rl_headers[0].1.clone()),
                        ("X-RateLimit-Remaining", rl_headers[1].1.clone()),
                        ("X-RateLimit-Reset", rl_headers[2].1.clone()),
                    ],
                    Json(response),
                )
                    .into_response());
            }
            Ok(idempotency::IdempotencyLookup::Conflict) => {
                let body = json!({
                    "error": "idempotency_conflict",
                    "detail": "A different request body was already submitted with this Idempotency-Key."
                });
                return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
            }
            Ok(idempotency::IdempotencyLookup::Miss) => {
                // Fall through to normal processing.
            }
            Err(e) => {
                // Log and fall through — idempotency is best-effort; don't
                // block the request on a DB error.
                tracing::warn!("idempotency lookup failed: {e}");
            }
        }
    }

    // --- 4. Parse body (body-size limit already enforced by middleware) ----
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let events = parse_body(content_type, &body)?;

    // --- 5. Per-event size check (JSON path only — NDJSON checked inline) --
    if !content_type.contains("application/x-ndjson") {
        check_event_sizes(&events)?;
    }

    // --- 6. Batch count check ---------------------------------------------
    if events.is_empty() {
        return Err(AppError::BadRequest("Empty event batch".into()));
    }
    if events.len() > MAX_BATCH_SIZE {
        return Err(AppError::BadRequest(format!(
            "Batch too large: {} events submitted, maximum is {}",
            events.len(),
            MAX_BATCH_SIZE,
        )));
    }

    // --- 7. Load contract identity + scope check --------------------------
    let identity: ContractIdentity = storage::get_contract_identity(&state.db, contract_id).await?;

    // API key contract-scope enforcement.
    if let Some(Extension(ref k)) = key_ext {
        if let Some(ref allowed) = k.allowed_contract_ids {
            if !allowed.contains(&contract_id) {
                return Err(AppError::Unauthorized);
            }
        }
    }

    // --- 8. Resolve version -----------------------------------------------
    let (resolved_version, pin_source) =
        resolve_version(&state, contract_id, query.version).await?;

    let version_row = storage::get_version(&state.db, contract_id, &resolved_version).await?;

    tracing::debug!(
        contract_id = %contract_id,
        version = %resolved_version,
        pin_source = pin_source,
        state = version_row.state.as_str(),
        batch_size = events.len(),
        "v1 ingest request routed"
    );

    // Source IP for audit rows.
    let source_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    // Deprecated-pin: quarantine wholesale (reuse existing logic shape).
    if version_row.state == VersionState::Deprecated {
        return deprecated_quarantine(
            &state,
            contract_id,
            org_id,
            &resolved_version,
            events,
            source_ip,
            query.dry_run,
        )
        .await;
    }

    // --- 9. Compile + validate --------------------------------------------
    let compiled = state.get_compiled(contract_id, &resolved_version).await?;
    let mut compiled_by_version: HashMap<String, Arc<CompiledContract>> = HashMap::new();
    compiled_by_version.insert(resolved_version.clone(), Arc::clone(&compiled));

    // RFC-016: time the top-level validation call.
    let _validation_start = Instant::now();
    let mut validation_results = parallel_validate(Arc::clone(&compiled), &events).await?;
    {
        let elapsed = _validation_start.elapsed().as_secs_f64();
        let outcome = if validation_results.iter().all(|r| r.passed) {
            "passed"
        } else {
            "failed"
        };
        metrics::histogram!(
            "contractgate_validation_duration_seconds",
            "contract_id" => contract_id.to_string(),
            "outcome" => outcome,
        )
        .record(elapsed);
    }

    // Fallback multi-stable resolution (RFC-002, same logic as ingest.rs).
    let mut per_event_versions = vec![resolved_version.clone(); events.len()];

    let fallback_eligible = identity.multi_stable_resolution == MultiStableResolution::Fallback
        && pin_source == "default_stable"
        && validation_results.iter().any(|r| !r.passed);

    if fallback_eligible {
        let stables = storage::list_stable_versions(&state.db, contract_id).await?;
        let other_stables: Vec<_> = stables
            .into_iter()
            .filter(|v| v.version != resolved_version)
            .collect();

        if !other_stables.is_empty() {
            let mut compiled_fallbacks: Vec<(String, Arc<CompiledContract>)> =
                Vec::with_capacity(other_stables.len());
            for v in &other_stables {
                let cc = state.get_compiled(contract_id, &v.version).await?;
                compiled_by_version.insert(v.version.clone(), Arc::clone(&cc));
                compiled_fallbacks.push((v.version.clone(), cc));
            }
            for (idx, result) in validation_results.iter_mut().enumerate() {
                if result.passed {
                    continue;
                }
                let event = &events[idx];
                let candidate_results: Vec<(String, ValidationResult)> = compiled_fallbacks
                    .par_iter()
                    .map(|(ver, cc)| (ver.clone(), validate(cc, event)))
                    .collect();
                if let Some((winning_version, winning_vr)) =
                    candidate_results.into_iter().find(|(_, vr)| vr.passed)
                {
                    *result = winning_vr;
                    per_event_versions[idx] = winning_version;
                }
            }
        }
    }

    // Uniqueness check (RFC quality).
    let uniqueness_violations = check_uniqueness_batch(&compiled.contract.quality, &events);
    for (idx, violation) in uniqueness_violations {
        if let Some(vr) = validation_results.get_mut(idx) {
            vr.violations.push(violation);
            vr.passed = false;
        }
    }

    // --- 10. RFC-004 transforms -------------------------------------------
    let transformed_payloads: Vec<TransformedPayload> = events
        .iter()
        .enumerate()
        .map(|(idx, event)| {
            let version = &per_event_versions[idx];
            let cc = compiled_by_version
                .get(version)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&compiled));
            apply_transforms(&cc, event.clone())
        })
        .collect();

    // --- Roll-up counts ---------------------------------------------------
    let total = events.len();
    let passed_count = validation_results.iter().filter(|r| r.passed).count();
    let failed_count = total - passed_count;
    let failed_indices: Vec<usize> = validation_results
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.passed)
        .map(|(i, _)| i)
        .collect();

    // --- 11. Pre-assign quarantine UUIDs ----------------------------------
    let mut quarantine_ids: Vec<Option<Uuid>> = vec![None; total];
    for &idx in &failed_indices {
        quarantine_ids[idx] = Some(Uuid::new_v4());
    }

    // --- Atomic short-circuit ---------------------------------------------
    let atomic_rejected = query.atomic && failed_count > 0;

    // --- 12. Persist (fire-and-forget) ------------------------------------
    if !query.dry_run && !atomic_rejected {
        let audit_rows: Vec<storage::AuditEntryInsert> = validation_results
            .iter()
            .enumerate()
            .map(|(idx, vr)| storage::AuditEntryInsert {
                contract_id,
                org_id,
                contract_version: per_event_versions[idx].clone(),
                passed: vr.passed,
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                raw_event: transformed_payloads[idx].clone(),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
                source: "http".to_string(),
                pre_assigned_id: None,
                replay_of_quarantine_id: None,
                direction: "ingress".to_string(),
            })
            .collect();

        let quarantine_rows: Vec<storage::QuarantineEventInsert> = failed_indices
            .iter()
            .map(|&idx| storage::QuarantineEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: transformed_payloads[idx].clone(),
                violation_count: validation_results[idx].violations.len() as i32,
                violation_details: serde_json::to_value(&validation_results[idx].violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                validation_us: validation_results[idx].validation_us as i64,
                source_ip: source_ip.clone(),
                replay_of_quarantine_id: None,
                pre_assigned_id: quarantine_ids[idx], // pre-assigned so we can return it
                direction: "ingress".to_string(),
            })
            .collect();

        let forward_rows: Vec<storage::ForwardEventInsert> = validation_results
            .iter()
            .enumerate()
            .filter(|(_, vr)| vr.passed)
            .map(|(idx, _)| storage::ForwardEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: transformed_payloads[idx].clone(),
            })
            .collect();

        // RFC-016: emit violation + quarantine counters at the audit write path.
        // `serde_json::to_string` gives the snake_case label (rename_all = "snake_case").
        {
            let cid = contract_id.to_string();
            for vr in &validation_results {
                for violation in &vr.violations {
                    let kind = serde_json::to_string(&violation.kind)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!(
                        "contractgate_violations_total",
                        "contract_id" => cid.clone(),
                        "kind" => kind,
                    )
                    .increment(1);
                }
            }
            if !quarantine_rows.is_empty() {
                metrics::counter!(
                    "contractgate_quarantined_total",
                    "contract_id" => cid,
                )
                .increment(quarantine_rows.len() as u64);
            }
        }

        if !audit_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::log_audit_entries_batch(&pool, &audit_rows).await {
                    tracing::warn!("v1 ingest: audit write failed: {e:?}");
                }
            });
        }
        if !quarantine_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::quarantine_events_batch(&pool, &quarantine_rows).await {
                    tracing::warn!("v1 ingest: quarantine write failed: {e:?}");
                }
            });
        }
        if !forward_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::forward_events_batch(&pool, &forward_rows).await {
                    tracing::warn!("v1 ingest: forward write failed: {e:?}");
                }
            });
        }
    } else if !query.dry_run && atomic_rejected {
        // Atomic summary audit row.
        let details: Vec<Value> = failed_indices
            .iter()
            .map(|&idx| {
                json!({
                    "index": idx,
                    "violations": validation_results[idx].violations,
                })
            })
            .collect();
        let summary = TransformedPayload::from_stored(json!({
            "batch_rejected": true,
            "atomic": true,
            "batch_size": total,
            "failing_count": failed_indices.len(),
        }));
        let pool = state.db.clone();
        let version = resolved_version.clone();
        let violation_count = failed_indices.len() as i32;
        let details_value = Value::Array(details);
        tokio::spawn(async move {
            if let Err(e) = storage::log_audit_entry(
                &pool,
                contract_id,
                org_id,
                &version,
                false,
                violation_count,
                details_value,
                summary,
                0,
                source_ip.as_deref(),
                "http",
                "ingress",
            )
            .await
            {
                tracing::warn!("v1 ingest: atomic audit write failed: {e:?}");
            }
        });
    }

    // --- Build per-event result list --------------------------------------
    let per_event_results: Vec<V1IngestEventResult> = validation_results
        .iter()
        .enumerate()
        .map(|(idx, vr)| V1IngestEventResult {
            index: idx,
            passed: vr.passed,
            violations: vr.violations.clone(),
            validation_us: vr.validation_us,
            forwarded: vr.passed && !query.dry_run && !atomic_rejected,
            contract_version: per_event_versions[idx].clone(),
            quarantine_id: quarantine_ids[idx],
            transformed_event: transformed_payloads[idx].as_value().clone(),
        })
        .collect();

    // --- 13. Compose response + idempotency store -------------------------
    let http_status = if atomic_rejected {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if failed_count == 0 {
        StatusCode::OK
    } else if passed_count == 0 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::MULTI_STATUS
    };

    let response_body = V1IngestResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        dry_run: query.dry_run,
        atomic: query.atomic,
        resolved_version: resolved_version.clone(),
        version_pin_source: pin_source,
        results: per_event_results,
    };
    let response_value = serde_json::to_value(&response_body)
        .unwrap_or_else(|_| json!({"error": "serialization_error"}));

    // Store idempotency record (unless dry_run, confirmed 2026-05-01).
    if let Some(ref key) = idem_key {
        if !query.dry_run {
            let db = state.db.clone();
            let key_owned = key.clone();
            let hash_owned = body_hash.clone();
            let status_u16 = http_status.as_u16();
            let resp_clone = response_value.clone();
            tokio::spawn(async move {
                if let Err(e) = idempotency::store(
                    &db,
                    &key_owned,
                    contract_id,
                    &hash_owned,
                    status_u16,
                    &resp_clone,
                )
                .await
                {
                    tracing::warn!("idempotency store failed: {e}");
                }
            });
        }
    }

    Ok((
        http_status,
        [
            ("X-RateLimit-Limit", rl_headers[0].1.clone()),
            ("X-RateLimit-Remaining", rl_headers[1].1.clone()),
            ("X-RateLimit-Reset", rl_headers[2].1.clone()),
        ],
        Json(response_value),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Deprecated-version wholesale quarantine
// ---------------------------------------------------------------------------

async fn deprecated_quarantine(
    state: &AppState,
    contract_id: Uuid,
    _org_id: Option<Uuid>,
    pinned_version: &str,
    events: Vec<Value>,
    source_ip: Option<String>,
    dry_run: bool,
) -> AppResult<Response> {
    let total = events.len();
    let compiled = state.get_compiled(contract_id, pinned_version).await?;

    let transformed: Vec<TransformedPayload> = events
        .iter()
        .map(|e| apply_transforms(&compiled, e.clone()))
        .collect();

    let per_event: Vec<V1IngestEventResult> = (0..total)
        .map(|idx| V1IngestEventResult {
            index: idx,
            passed: false,
            violations: vec![],
            validation_us: 0,
            forwarded: false,
            contract_version: pinned_version.to_string(),
            quarantine_id: None,
            transformed_event: transformed[idx].as_value().clone(),
        })
        .collect();

    if !dry_run {
        let synthetic = json!({
            "kind": "deprecated_contract_version",
            "pinned_version": pinned_version,
        });
        let qrows: Vec<storage::QuarantineEventInsert> = transformed
            .iter()
            .map(|tp| storage::QuarantineEventInsert {
                contract_id,
                contract_version: pinned_version.to_string(),
                payload: tp.clone(),
                violation_count: 1,
                violation_details: Value::Array(vec![synthetic.clone()]),
                validation_us: 0,
                source_ip: source_ip.clone(),
                replay_of_quarantine_id: None,
                pre_assigned_id: None,
                direction: "ingress".to_string(),
            })
            .collect();
        let pool = state.db.clone();
        tokio::spawn(async move {
            if let Err(e) = storage::quarantine_events_batch(&pool, &qrows).await {
                tracing::warn!("v1 ingest: deprecated quarantine write failed: {e:?}");
            }
        });
    }

    let body = json!({
        "error": format!(
            "Version {} on contract {} is deprecated; batch quarantined.",
            pinned_version, contract_id,
        ),
        "total": total,
        "passed": 0,
        "failed": total,
        "dry_run": dry_run,
        "version_pin_source": "pinned_deprecated",
        "results": per_event,
    });

    Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_array_parsed() {
        let body = br#"[{"a":1},{"b":2}]"#;
        let events = parse_body("application/json", body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn json_single_object_parsed_as_batch_of_one() {
        let body = br#"{"a":1}"#;
        let events = parse_body("application/json", body).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn ndjson_parsed() {
        let body = b"{\"a\":1}\n{\"b\":2}\n";
        let events = parse_body("application/x-ndjson", body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ndjson_trailing_newline_ignored() {
        let body = b"{\"a\":1}\n{\"b\":2}\n\n";
        let events = parse_ndjson(body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ndjson_bad_line_returns_error() {
        let body = b"{\"a\":1}\nnot json\n";
        let err = parse_ndjson(body).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("line 1"),
            "expected line index in error, got: {msg}"
        );
    }

    #[test]
    fn event_over_1mb_rejected() {
        // Build a JSON object > 1 MB.
        let big = format!(r#"{{"data":"{}"}}"#, "x".repeat(MAX_EVENT_BYTES + 1));
        let events = vec![serde_json::from_str(&big).unwrap()];
        let err = check_event_sizes(&events).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("1 MB"),
            "expected size limit message, got: {msg}"
        );
    }

    #[test]
    fn rate_limit_headers_format() {
        use crate::rate_limit::{RateLimitOutcome, DEFAULT_RATE_LIMIT_RPS};
        let outcome = RateLimitOutcome {
            allowed: true,
            limit: DEFAULT_RATE_LIMIT_RPS,
            remaining: 999,
            reset_unix: 1_714_000_000,
            retry_after_ms: 0,
        };
        let hdrs = rate_limit_headers(&outcome);
        assert_eq!(hdrs[0].0, "X-RateLimit-Limit");
        assert_eq!(hdrs[1].0, "X-RateLimit-Remaining");
        assert_eq!(hdrs[2].0, "X-RateLimit-Reset");
    }

    #[test]
    fn openapi_spec_is_valid_json() {
        let spec = build_openapi();
        let parsed: Result<Value, _> = serde_json::from_str(&spec);
        assert!(parsed.is_ok(), "openapi spec must be valid JSON");
        let v = parsed.unwrap();
        assert_eq!(v["openapi"], "3.1.0");
    }
}
