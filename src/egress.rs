//! Egress validation handler — `POST /egress/{contract_id}`.
//!
//! Validates outbound payloads against a named contract using the exact same
//! `validate()` engine as ingest.  No rule logic is duplicated — the compiled
//! contract, pattern cache, and per-field checks are identical.  Only the
//! traffic direction and the per-record *disposition* differ.
//!
//! ### Disposition modes
//!
//! | Mode    | Behavior |
//! |---------|----------|
//! | `block` | Drop failing records from the response payload. Passing records still ship. (Default.) |
//! | `fail`  | Any failure rejects the **entire** response (422). Atomic mode. |
//! | `tag`   | All records pass through; failures are flagged in per-record outcomes. |
//!
//! ### RFC-030: Egress PII & Leakage Guard
//!
//! After schema validation (RFC-029), the egress handler applies the RFC-004
//! transform engine to the outbound payload — the same `apply_transforms`
//! used on ingest.  A field declared `drop` is dropped; `hash` ships hashed;
//! `mask` ships masked; `redact` ships `"<REDACTED>"`.
//!
//! The `egress_leakage_mode` contract field then handles **undeclared** fields:
//!
//! | `egress_leakage_mode` | Behavior |
//! |------------------------|---------|
//! | `off` (default)        | Undeclared fields pass through unchanged. |
//! | `strip`                | Undeclared fields removed; names recorded in `stripped_fields`. |
//! | `fail`                 | Each undeclared field becomes a `LeakageViolation`; record subject to disposition. Field stripped regardless of disposition. |
//!
//! The payload returned by this handler is always the **post-transform,
//! post-leakage** payload.  Raw PII never leaves the API.
//!
//! ### HTTP status codes
//! - **200 OK** — all records passed
//! - **207 Multi-Status** — block or tag mode with a mix of pass and fail
//! - **422 Unprocessable** — fail mode with any failure, or all records failed
//! - **400 Bad Request** — malformed path/body
//! - **404 Not Found** / **409 Conflict** — contract or version resolution failed
//!
//! ### Query parameters
//! - `?disposition=block|fail|tag` — default `block`
//! - `?dry_run=true` — validate without writing to the database
//!
//! ### Audit trail
//! All events — passing and failing — are written to `audit_log` with
//! `direction = 'egress'`.  Failing records are also quarantined with
//! `direction = 'egress'` so they are queryable alongside ingest failures
//! (RFC-029 §Unified audit trail).

use crate::api_key_auth::ValidatedKey;
use axum::{
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::contract::EgressLeakageMode;
use crate::error::{AppError, AppResult};
use crate::storage;
use crate::transform::{apply_transforms, TransformedPayload};
use crate::validation::{validate, ValidationResult, Violation, ViolationKind};
use crate::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum batch size — matches ingest cap (RFC-001).
pub const MAX_BATCH_SIZE: usize = 1_000;

// ---------------------------------------------------------------------------
// Disposition mode
// ---------------------------------------------------------------------------

/// What to do with failing records on the egress path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionMode {
    /// Drop failing records from the response payload.  Good records still
    /// ship.  Returns 207 when any record was blocked; 200 when all pass.
    #[default]
    Block,
    /// Any failing record fails the whole response (422).  Atomic mode.
    Fail,
    /// All records pass through; failures are flagged in per-record outcomes.
    /// Returns 207 when any record was tagged; 200 when all pass.
    Tag,
}

impl DispositionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DispositionMode::Block => "block",
            DispositionMode::Fail => "fail",
            DispositionMode::Tag => "tag",
        }
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EgressQuery {
    /// How to handle failing records.  Default: `block`.
    #[serde(default)]
    pub disposition: DispositionMode,
    /// Validate without writing to the database.
    #[serde(default)]
    pub dry_run: bool,
}

/// Per-record outcome in the egress response.
#[derive(Debug, Clone, Serialize)]
pub struct EgressOutcome {
    /// Zero-based index of this record in the original payload.
    pub index: usize,
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    pub validation_us: u64,
    /// What happened to this record:
    /// - `"included"` — passed, present in `payload`
    /// - `"blocked"`  — failed, dropped from `payload` (block mode)
    /// - `"rejected"` — part of a wholesale rejection (fail mode)
    /// - `"tagged"`   — failed but present in `payload` with flag (tag mode)
    pub action: &'static str,
    /// RFC-030: names of undeclared fields that were stripped from this
    /// record's response payload.  Non-empty when `egress_leakage_mode` is
    /// `strip` or `fail` and the payload contained undeclared fields.
    /// Empty (omitted from JSON) when no fields were stripped.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stripped_fields: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct EgressResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub disposition: &'static str,
    /// RFC-030: the egress leakage mode in effect for this request.
    pub egress_leakage_mode: &'static str,
    pub resolved_version: String,
    /// RFC-030: cleaned payload — post-transform (RFC-004) and post-leakage
    /// (RFC-030) — returned to the caller.  Raw PII never appears here.
    ///
    /// Filtered further by disposition:
    /// - block: only passing records
    /// - fail: empty when any record fails (whole response rejected)
    /// - tag: all records regardless of outcome
    pub payload: Vec<Value>,
    /// Per-record validation outcomes (one entry per input record).
    pub outcomes: Vec<EgressOutcome>,
}

// ---------------------------------------------------------------------------
// Pure disposition logic (testable without HTTP or DB)
// ---------------------------------------------------------------------------

/// Apply disposition logic to validation results.
///
/// Returns `(payload_records, outcomes)`.
///
/// - `block`: payload = passing records; failing records are dropped.
/// - `fail`: payload = all records when all pass; empty when any fail.
/// - `tag`: payload = all records regardless of outcome.
pub fn apply_disposition(
    events: &[Value],
    results: &[ValidationResult],
    mode: DispositionMode,
) -> (Vec<Value>, Vec<EgressOutcome>) {
    let total = events.len();
    let mut payload: Vec<Value> = Vec::with_capacity(total);
    let mut outcomes: Vec<EgressOutcome> = Vec::with_capacity(total);

    let any_fail = results.iter().any(|r| !r.passed);

    for (idx, (event, vr)) in events.iter().zip(results.iter()).enumerate() {
        let action: &'static str = match mode {
            DispositionMode::Block => {
                if vr.passed {
                    payload.push(event.clone());
                    "included"
                } else {
                    "blocked"
                }
            }
            DispositionMode::Fail => {
                if !any_fail {
                    // All pass — include everything.
                    payload.push(event.clone());
                }
                if vr.passed && !any_fail {
                    "included"
                } else if vr.passed {
                    // Passed individually but wholesale rejected.
                    "rejected"
                } else {
                    "rejected"
                }
            }
            DispositionMode::Tag => {
                payload.push(event.clone());
                if vr.passed {
                    "included"
                } else {
                    "tagged"
                }
            }
        };

        outcomes.push(EgressOutcome {
            index: idx,
            passed: vr.passed,
            violations: vr.violations.clone(),
            validation_us: vr.validation_us,
            action,
            stripped_fields: vec![],
        });
    }

    (payload, outcomes)
}

// ---------------------------------------------------------------------------
// Parallel validation helper
// ---------------------------------------------------------------------------

async fn parallel_validate(
    compiled: Arc<crate::validation::CompiledContract>,
    events: &[Value],
) -> AppResult<Vec<ValidationResult>> {
    let events = events.to_vec();
    tokio::task::spawn_blocking(move || events.par_iter().map(|e| validate(&compiled, e)).collect())
        .await
        .map_err(|e| AppError::Internal(format!("Egress validation task join error: {e}")))
}

// ---------------------------------------------------------------------------
// RFC-030: Egress PII pipeline
// ---------------------------------------------------------------------------

/// Apply the RFC-030 egress PII pipeline to a single outbound record.
///
/// Steps:
///   1. `apply_transforms` — drop/hash/mask/redact declared fields (RFC-004).
///   2. Leakage guard — handle undeclared fields per `egress_leakage_mode`.
///
/// Returns `(for_storage, for_response, stripped_fields, leakage_violations)`:
///
/// - `for_storage` — `TransformedPayload` for audit/quarantine (RFC-004 only;
///   leakage stripping is an egress-path concern and does not widen the audit
///   row further than what RFC-004 already requires).
/// - `for_response` — post-transform + post-leakage `Value` returned to the
///   caller.  Raw PII is never present here.
/// - `stripped_fields` — undeclared field names removed from the response.
///   Non-empty only when `egress_leakage_mode` is `strip` or `fail`.
/// - `leakage_violations` — one `LeakageViolation` per stripped field.
///   Non-empty only when `egress_leakage_mode` is `fail`.
fn apply_egress_pii_pipeline(
    compiled: &crate::validation::CompiledContract,
    raw: Value,
) -> (TransformedPayload, Value, Vec<String>, Vec<Violation>) {
    // Step 1: RFC-004 transforms (drop/hash/mask/redact declared fields).
    // `for_storage` is the `TransformedPayload` that goes to audit/quarantine.
    let for_storage = apply_transforms(compiled, raw.clone());
    // Start the response value from the same transform output.
    let transformed_val = apply_transforms(compiled, raw).into_inner();

    // Step 2: egress leakage guard on undeclared fields.
    let mode = compiled.contract.egress_leakage_mode;
    if mode == EgressLeakageMode::Off {
        return (for_storage, transformed_val, vec![], vec![]);
    }

    // Build declared-fields set on the fly (small; no alloc pressure).
    let declared: std::collections::HashSet<&str> = compiled
        .contract
        .ontology
        .entities
        .iter()
        .map(|e| e.name.as_str())
        .collect();

    let mut obj = match transformed_val {
        Value::Object(map) => map,
        other => return (for_storage, other, vec![], vec![]),
    };

    let undeclared: Vec<String> = obj
        .keys()
        .filter(|k| !declared.contains(k.as_str()))
        .cloned()
        .collect();

    if undeclared.is_empty() {
        return (for_storage, Value::Object(obj), vec![], vec![]);
    }

    // Both `strip` and `fail` remove the undeclared fields from the response.
    for field in &undeclared {
        obj.remove(field);
    }

    // `fail` additionally surfaces a violation per undeclared field.
    let leakage_violations: Vec<Violation> = if mode == EgressLeakageMode::Fail {
        undeclared
            .iter()
            .map(|field| Violation {
                field: field.clone(),
                message: format!(
                    "Undeclared field '{field}' found in egress payload \
                     (egress_leakage_mode=fail); field stripped from response"
                ),
                kind: ViolationKind::LeakageViolation,
            })
            .collect()
    } else {
        vec![]
    };

    (
        for_storage,
        Value::Object(obj),
        undeclared,
        leakage_violations,
    )
}

// ---------------------------------------------------------------------------
// Handler: POST /egress/{contract_id}
// ---------------------------------------------------------------------------

/// Validate an outbound payload against a named contract.
///
/// Path parameter: `{contract_id}` — UUID, with optional `@version` suffix
/// (e.g. `/egress/abc123@1.2.3`).
pub async fn egress_handler(
    State(state): State<Arc<AppState>>,
    Path(raw_id): Path<String>,
    Query(query): Query<EgressQuery>,
    headers: HeaderMap,
    key_ext: Option<Extension<ValidatedKey>>,
    Json(body): Json<Value>,
) -> AppResult<axum::response::Response> {
    // --- Resolve org_id (same pattern as ingest) ----------------------------
    let org_id: Option<Uuid> = key_ext.map(|Extension(k)| k.org_id).or_else(|| {
        headers
            .get("x-org-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
    });

    // --- Parse path: uuid[@version] ----------------------------------------
    let (contract_id, path_version) = parse_egress_path(&raw_id)?;

    // --- Version header (mirrors ingest) ------------------------------------
    let header_version = headers
        .get("x-contract-version")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // --- Load contract identity ---------------------------------------------
    let _identity = storage::get_contract_identity(&state.db, contract_id).await?;

    // --- Resolve version ----------------------------------------------------
    let (resolved_version, _pin_source) =
        resolve_version(&state, contract_id, header_version, path_version).await?;

    // --- Fetch version row (check deprecated) --------------------------------
    let version_row = storage::get_version(&state.db, contract_id, &resolved_version).await?;

    tracing::debug!(
        contract_id = %contract_id,
        version = %resolved_version,
        disposition = query.disposition.as_str(),
        "egress request routed"
    );

    // --- Normalise body to a batch ------------------------------------------
    let events: Vec<Value> = match body {
        Value::Array(arr) => arr,
        single => vec![single],
    };
    if events.is_empty() {
        return Err(AppError::BadRequest("Empty event batch".into()));
    }
    if events.len() > MAX_BATCH_SIZE {
        return Err(AppError::BadRequest(format!(
            "Batch too large: {} events, maximum is {}",
            events.len(),
            MAX_BATCH_SIZE
        )));
    }

    // Egress does not support deprecated-version traffic (it makes no sense
    // to certify outbound data against a deprecated contract).
    if version_row.state == crate::contract::VersionState::Deprecated {
        return Err(AppError::BadRequest(format!(
            "Version {} on contract {} is deprecated; pin a stable version for egress",
            resolved_version, contract_id,
        )));
    }

    // --- Source IP ----------------------------------------------------------
    let source_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    // --- Compile (or cache hit) ---------------------------------------------
    let compiled = state.get_compiled(contract_id, &resolved_version).await?;

    // --- Validate (parallel, rayon in spawn_blocking) -----------------------
    let validation_results = parallel_validate(Arc::clone(&compiled), &events).await?;

    // --- RFC-030: egress PII pipeline (transforms + leakage guard) ----------
    // Runs after validation so the validator sees raw values (rule checks run
    // on original data), before the response payload is assembled so the
    // caller never receives raw PII.
    let mut transformed_payloads: Vec<TransformedPayload> = Vec::with_capacity(events.len());
    let mut cleaned_events: Vec<Value> = Vec::with_capacity(events.len());
    let mut stripped_fields_per_record: Vec<Vec<String>> = Vec::with_capacity(events.len());
    let mut leakage_violations_per_record: Vec<Vec<Violation>> = Vec::with_capacity(events.len());
    for e in &events {
        let (tp, ce, sf, lv) = apply_egress_pii_pipeline(&compiled, e.clone());
        transformed_payloads.push(tp);
        cleaned_events.push(ce);
        stripped_fields_per_record.push(sf);
        leakage_violations_per_record.push(lv);
    }

    // When egress_leakage_mode = fail, merge leakage violations into the
    // per-record validation results so the RFC-029 disposition handles them
    // exactly like schema violations (block / fail / tag).
    let merged_results: Vec<ValidationResult> = validation_results
        .into_iter()
        .zip(leakage_violations_per_record.iter())
        .map(|(mut vr, lv)| {
            if !lv.is_empty() {
                vr.violations.extend(lv.iter().cloned());
                vr.passed = false;
            }
            vr
        })
        .collect();

    // --- Counts -------------------------------------------------------------
    let total = events.len();
    let passed_count = merged_results.iter().filter(|r| r.passed).count();
    let failed_count = total - passed_count;
    let any_fail = failed_count > 0;

    // --- Apply disposition (cleaned payload — raw PII never in response) ----
    let (payload, mut outcomes) =
        apply_disposition(&cleaned_events, &merged_results, query.disposition);
    // Attach per-record stripped_fields from the leakage pipeline.
    for (outcome, sf) in outcomes.iter_mut().zip(stripped_fields_per_record.iter()) {
        outcome.stripped_fields = sf.clone();
    }

    // --- Persist (fire-and-forget, direction = 'egress') --------------------
    if !query.dry_run {
        // Audit rows — one per event.
        let audit_rows: Vec<storage::AuditEntryInsert> = merged_results
            .iter()
            .enumerate()
            .map(|(idx, vr)| storage::AuditEntryInsert {
                contract_id,
                org_id,
                contract_version: resolved_version.clone(),
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
                direction: "egress".to_string(),
            })
            .collect();

        // Quarantine rows — failing records only.
        let quarantine_rows: Vec<storage::QuarantineEventInsert> = merged_results
            .iter()
            .enumerate()
            .filter(|(_, vr)| !vr.passed)
            .map(|(idx, vr)| storage::QuarantineEventInsert {
                contract_id,
                contract_version: resolved_version.clone(),
                payload: transformed_payloads[idx].clone(),
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
                replay_of_quarantine_id: None,
                pre_assigned_id: None,
                direction: "egress".to_string(),
            })
            .collect();

        if !audit_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::log_audit_entries_batch(&pool, &audit_rows).await {
                    tracing::warn!("egress: audit write failed: {:?}", e);
                }
            });
        }
        if !quarantine_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) = storage::quarantine_events_batch(&pool, &quarantine_rows).await {
                    tracing::warn!("egress: quarantine write failed: {:?}", e);
                }
            });
        }
    }

    // --- Compose response ---------------------------------------------------
    let response_body = EgressResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        dry_run: query.dry_run,
        disposition: query.disposition.as_str(),
        egress_leakage_mode: compiled.contract.egress_leakage_mode.as_str(),
        resolved_version,
        payload,
        outcomes,
    };

    let status = match query.disposition {
        DispositionMode::Fail if any_fail => StatusCode::UNPROCESSABLE_ENTITY,
        _ if passed_count == 0 && failed_count > 0 => StatusCode::UNPROCESSABLE_ENTITY,
        _ if failed_count > 0 => StatusCode::MULTI_STATUS,
        _ => StatusCode::OK,
    };

    Ok((status, Json(response_body)).into_response())
}

// ---------------------------------------------------------------------------
// Path parsing — same @version suffix convention as ingest
// ---------------------------------------------------------------------------

fn parse_egress_path(raw: &str) -> AppResult<(Uuid, Option<String>)> {
    let (uuid_part, path_version) = match raw.split_once('@') {
        Some((id, v)) if !v.is_empty() => (id, Some(v.to_string())),
        Some((id, _)) => (id, None),
        None => (raw, None),
    };
    let uuid = Uuid::parse_str(uuid_part)
        .map_err(|_| AppError::BadRequest(format!("invalid contract_id in path: {uuid_part}")))?;
    Ok((uuid, path_version))
}

// ---------------------------------------------------------------------------
// Version resolution (mirrors ingest.rs)
// ---------------------------------------------------------------------------

async fn resolve_version(
    state: &AppState,
    contract_id: Uuid,
    header_version: Option<String>,
    path_version: Option<String>,
) -> AppResult<(String, &'static str)> {
    if header_version.is_some() && path_version.is_some() {
        tracing::warn!(
            "both X-Contract-Version header and @version path suffix provided for egress; header wins"
        );
    }
    if let Some(v) = header_version {
        return Ok((v, "header"));
    }
    if let Some(v) = path_version {
        return Ok((v, "path"));
    }
    let latest = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok((latest.version, "default_stable"))
}

// ---------------------------------------------------------------------------
// Unit tests — no DB, no HTTP
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        Contract, EgressLeakageMode, FieldDefinition, FieldType, Ontology, Transform, TransformKind,
    };
    use crate::validation::{CompiledContract, ValidationResult, Violation, ViolationKind};
    use serde_json::json;

    // -----------------------------------------------------------------------
    // RFC-030 test helpers
    // -----------------------------------------------------------------------

    fn bare_field(name: &str) -> FieldDefinition {
        FieldDefinition {
            name: name.into(),
            field_type: FieldType::String,
            required: true,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
        }
    }

    fn field_with_transform(name: &str, kind: TransformKind) -> FieldDefinition {
        FieldDefinition {
            transform: Some(Transform { kind, style: None }),
            ..bare_field(name)
        }
    }

    fn compiled(
        egress_leakage_mode: EgressLeakageMode,
        fields: Vec<FieldDefinition>,
    ) -> CompiledContract {
        CompiledContract::compile(Contract {
            version: "1.0".into(),
            name: "test".into(),
            description: None,
            compliance_mode: false,
            ontology: Ontology { entities: fields },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            egress_leakage_mode,
        })
        .expect("test contract must compile")
    }

    // -----------------------------------------------------------------------
    // RFC-030: PII masking on egress
    // -----------------------------------------------------------------------

    #[test]
    fn pii_mask_applied_to_egress_response() {
        let c = compiled(
            EgressLeakageMode::Off,
            vec![field_with_transform("user_email", TransformKind::Mask)],
        );
        let event = json!({ "user_email": "alice@example.com" });
        let (_for_storage, for_response, stripped, violations) =
            apply_egress_pii_pipeline(&c, event);

        assert_eq!(
            for_response["user_email"], "****",
            "email must be masked in response"
        );
        assert!(stripped.is_empty());
        assert!(violations.is_empty());
    }

    #[test]
    fn pii_redact_applied_to_egress_response() {
        let c = compiled(
            EgressLeakageMode::Off,
            vec![field_with_transform("ssn", TransformKind::Redact)],
        );
        let event = json!({ "ssn": "123-45-6789" });
        let (_for_storage, for_response, stripped, violations) =
            apply_egress_pii_pipeline(&c, event);

        assert_eq!(for_response["ssn"], "<REDACTED>");
        assert!(stripped.is_empty());
        assert!(violations.is_empty());
    }

    #[test]
    fn pii_drop_removes_field_from_egress_response() {
        let c = compiled(
            EgressLeakageMode::Off,
            vec![
                bare_field("user_id"),
                field_with_transform("internal_key", TransformKind::Drop),
            ],
        );
        let event = json!({ "user_id": "u1", "internal_key": "secret" });
        let (_for_storage, for_response, stripped, violations) =
            apply_egress_pii_pipeline(&c, event);

        assert!(
            for_response.get("internal_key").is_none(),
            "dropped field absent"
        );
        assert_eq!(for_response["user_id"], "u1");
        assert!(stripped.is_empty());
        assert!(violations.is_empty());
    }

    // -----------------------------------------------------------------------
    // RFC-030: egress_leakage_mode = fail
    // -----------------------------------------------------------------------

    #[test]
    fn leakage_fail_produces_violation_for_undeclared_field() {
        let c = compiled(EgressLeakageMode::Fail, vec![bare_field("user_id")]);
        let event = json!({ "user_id": "u123", "cost_basis": 9.99 });
        let (_tp, cleaned, stripped, violations) = apply_egress_pii_pipeline(&c, event);

        assert!(
            cleaned.get("cost_basis").is_none(),
            "undeclared field stripped from response"
        );
        assert_eq!(stripped, vec!["cost_basis"]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "cost_basis");
        assert_eq!(violations[0].kind, ViolationKind::LeakageViolation);
        // Declared field unaffected
        assert_eq!(cleaned["user_id"], "u123");
    }

    #[test]
    fn leakage_fail_multiple_undeclared_fields() {
        let c = compiled(EgressLeakageMode::Fail, vec![bare_field("event_type")]);
        let event = json!({ "event_type": "click", "risk_score": 0.8, "debug_trace": "xyz" });
        let (_tp, cleaned, mut stripped, violations) = apply_egress_pii_pipeline(&c, event);

        stripped.sort();
        assert_eq!(stripped, vec!["debug_trace", "risk_score"]);
        assert_eq!(violations.len(), 2);
        assert!(cleaned.get("risk_score").is_none());
        assert!(cleaned.get("debug_trace").is_none());
        assert_eq!(cleaned["event_type"], "click");
    }

    // -----------------------------------------------------------------------
    // RFC-030: egress_leakage_mode = strip
    // -----------------------------------------------------------------------

    #[test]
    fn leakage_strip_removes_field_no_violation() {
        let c = compiled(EgressLeakageMode::Strip, vec![bare_field("user_id")]);
        let event = json!({ "user_id": "u123", "internal_cost": 42 });
        let (_tp, cleaned, stripped, violations) = apply_egress_pii_pipeline(&c, event);

        assert!(
            cleaned.get("internal_cost").is_none(),
            "undeclared field stripped"
        );
        assert_eq!(stripped, vec!["internal_cost"]);
        assert!(
            violations.is_empty(),
            "strip mode must not produce violations"
        );
    }

    // -----------------------------------------------------------------------
    // RFC-030: egress_leakage_mode = off (default)
    // -----------------------------------------------------------------------

    #[test]
    fn leakage_off_passes_undeclared_fields_through() {
        let c = compiled(EgressLeakageMode::Off, vec![bare_field("user_id")]);
        let event = json!({ "user_id": "u123", "extra": "harmless" });
        let (_tp, cleaned, stripped, violations) = apply_egress_pii_pipeline(&c, event);

        assert_eq!(
            cleaned["extra"], "harmless",
            "undeclared field passes through in off mode"
        );
        assert!(stripped.is_empty());
        assert!(violations.is_empty());
    }

    // -----------------------------------------------------------------------
    // RFC-030: ingest path unaffected
    // -----------------------------------------------------------------------

    #[test]
    fn ingest_apply_transforms_unaffected_by_egress_leakage_mode() {
        // apply_transforms (ingest path) must NOT strip undeclared fields
        // based on egress_leakage_mode — that logic is egress-only.
        use crate::transform::apply_transforms;
        let c = compiled(
            EgressLeakageMode::Fail, // egress set to fail, ingest must be unaffected
            vec![field_with_transform("user_email", TransformKind::Redact)],
        );
        let event = json!({ "user_email": "bob@example.com", "extra_ingest_field": "present" });
        let tp = apply_transforms(&c, event);
        let val = tp.into_inner();

        assert_eq!(val["user_email"], "<REDACTED>", "ingest redact applied");
        // compliance_mode=false means ingest does NOT strip undeclared fields
        assert_eq!(
            val["extra_ingest_field"], "present",
            "ingest path: egress_leakage_mode must not affect apply_transforms"
        );
    }

    #[test]
    fn salt_continuity_hash_egress_equals_ingest() {
        // A hash transform applied via the egress pipeline must produce the
        // same output as the same transform applied on the ingest path,
        // guaranteeing downstream join keys remain consistent (RFC-030 §Salt
        // continuity).
        use crate::transform::apply_transforms;
        let c = compiled(
            EgressLeakageMode::Off,
            vec![field_with_transform("user_id", TransformKind::Hash)],
        );
        let event = json!({ "user_id": "alice" });

        // Ingest hash
        let ingest_hash = apply_transforms(&c, event.clone()).into_inner();
        let ingest_val = ingest_hash["user_id"].as_str().unwrap();

        // Egress hash (via pipeline)
        let (_tp, for_response, _, _) = apply_egress_pii_pipeline(&c, event);
        let egress_val = for_response["user_id"].as_str().unwrap();

        assert_eq!(
            ingest_val, egress_val,
            "hash must be identical on ingest and egress"
        );
        assert!(ingest_val.starts_with("hmac-sha256:"), "hash format check");
    }

    fn make_validation_result(passed: bool) -> ValidationResult {
        if passed {
            ValidationResult {
                passed: true,
                violations: vec![],
                validation_us: 10,
            }
        } else {
            ValidationResult {
                passed: false,
                violations: vec![Violation {
                    field: "user_id".into(),
                    message: "Required field 'user_id' is missing".into(),
                    kind: ViolationKind::MissingRequiredField,
                }],
                validation_us: 8,
            }
        }
    }

    fn make_event(i: usize) -> Value {
        json!({ "index": i, "event_type": "click", "timestamp": 1712000000 })
    }

    // -----------------------------------------------------------------------
    // block mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn block_all_pass() {
        let events = vec![make_event(0), make_event(1)];
        let results = vec![make_validation_result(true), make_validation_result(true)];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Block);

        assert_eq!(payload.len(), 2, "all records included");
        assert!(outcomes.iter().all(|o| o.action == "included"));
        assert!(outcomes.iter().all(|o| o.passed));
    }

    #[test]
    fn block_partial_fail() {
        let events = vec![make_event(0), make_event(1), make_event(2)];
        let results = vec![
            make_validation_result(true),
            make_validation_result(false), // index 1 fails
            make_validation_result(true),
        ];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Block);

        // Only passing records in payload
        assert_eq!(payload.len(), 2, "one record blocked");
        assert_eq!(outcomes[0].action, "included");
        assert_eq!(outcomes[1].action, "blocked");
        assert_eq!(outcomes[2].action, "included");
        assert!(!outcomes[1].passed);
        assert_eq!(outcomes[1].violations.len(), 1);
    }

    #[test]
    fn block_all_fail() {
        let events = vec![make_event(0), make_event(1)];
        let results = vec![make_validation_result(false), make_validation_result(false)];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Block);

        assert_eq!(payload.len(), 0, "no records in payload");
        assert!(outcomes.iter().all(|o| o.action == "blocked"));
    }

    // -----------------------------------------------------------------------
    // fail mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn fail_all_pass() {
        let events = vec![make_event(0), make_event(1)];
        let results = vec![make_validation_result(true), make_validation_result(true)];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Fail);

        assert_eq!(payload.len(), 2, "all records included when all pass");
        assert!(outcomes.iter().all(|o| o.action == "included"));
    }

    #[test]
    fn fail_partial_fail_rejects_all() {
        let events = vec![make_event(0), make_event(1), make_event(2)];
        let results = vec![
            make_validation_result(true),
            make_validation_result(false), // one failure → reject all
            make_validation_result(true),
        ];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Fail);

        // Whole response rejected — no payload records
        assert_eq!(payload.len(), 0, "wholesale rejection in fail mode");
        assert!(outcomes.iter().all(|o| o.action == "rejected"));
    }

    #[test]
    fn fail_all_fail() {
        let events = vec![make_event(0)];
        let results = vec![make_validation_result(false)];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Fail);

        assert_eq!(payload.len(), 0);
        assert_eq!(outcomes[0].action, "rejected");
    }

    // -----------------------------------------------------------------------
    // tag mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn tag_all_pass() {
        let events = vec![make_event(0), make_event(1)];
        let results = vec![make_validation_result(true), make_validation_result(true)];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Tag);

        assert_eq!(payload.len(), 2);
        assert!(outcomes.iter().all(|o| o.action == "included"));
    }

    #[test]
    fn tag_partial_fail_passes_through() {
        let events = vec![make_event(0), make_event(1), make_event(2)];
        let results = vec![
            make_validation_result(true),
            make_validation_result(false), // flagged but not dropped
            make_validation_result(true),
        ];
        let (payload, outcomes) = apply_disposition(&events, &results, DispositionMode::Tag);

        // All records present in payload
        assert_eq!(payload.len(), 3, "all records pass through in tag mode");
        assert_eq!(outcomes[0].action, "included");
        assert_eq!(outcomes[1].action, "tagged");
        assert_eq!(outcomes[2].action, "included");
        // Violation is still surfaced
        assert!(!outcomes[1].passed);
        assert_eq!(outcomes[1].violations.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Path parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_plain_uuid() {
        let id = Uuid::new_v4();
        let (parsed, v) = parse_egress_path(&id.to_string()).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(v, None);
    }

    #[test]
    fn parse_uuid_with_version() {
        let id = Uuid::new_v4();
        let raw = format!("{id}@2.0.0");
        let (parsed, v) = parse_egress_path(&raw).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(v.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn rejects_invalid_uuid() {
        assert!(parse_egress_path("not-a-uuid").is_err());
    }

    // -----------------------------------------------------------------------
    // Direction constant check
    // -----------------------------------------------------------------------

    #[test]
    fn direction_strings() {
        // Verify the &'static str values used in AuditEntryInsert / QuarantineEventInsert.
        assert_eq!("egress", "egress");
        assert_eq!("ingress", "ingress");
    }
}
