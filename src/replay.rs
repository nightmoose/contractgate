//! Manual Replay Quarantine — RFC-003.
//!
//! Exposes two endpoints:
//!
//! * `POST /contracts/:id/quarantine/replay` — re-validate a list of
//!   previously-quarantined payloads against a target contract version.
//!   Passes land in `audit_log` + `forwarded_events` (Q3=A).  Failures
//!   write a new `quarantine_events` row linked back to the source.  The
//!   source row is stamped `status='replayed'` + `replayed_at` +
//!   `replayed_into_audit_id` only on success; on failure it is untouched.
//!
//! * `GET /contracts/:id/quarantine/:quar_id/replay-history` — returns the
//!   chain of replay attempts for a given source quarantine row for the
//!   dashboard history drawer.
//!
//! Design constraints locked in by RFC-003 sign-off (2026-04-18):
//!
//! - **Q1**: `reviewed` rows are replayable.  Only `purged` is terminal.
//! - **Q2**: Draft versions are allowed as targets, flagged in the
//!   response (`target_is_draft: true`).
//! - **Q3**: Replay-passes fire the contract's forward destination the
//!   same way fresh ingest does.
//! - **Q4**: 1 000-row cap per request, matching batch ingest.

/// Maximum event ids accepted by a single replay request.
pub const MAX_REPLAY_IDS: usize = 1_000;

use axum::{
    extract::{Path, State},
    Json,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::contract::{MultiStableResolution, VersionState};
use crate::error::{AppError, AppResult};
use crate::storage;
use crate::transform::TransformedPayload;
use crate::validation::{validate, CompiledContract, ValidationResult, Violation};
use crate::{AppState, OrgId};

// ---------------------------------------------------------------------------
// Public request / response types
// ---------------------------------------------------------------------------

/// Request body for `POST /contracts/:id/quarantine/replay`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReplayRequest {
    /// Source quarantine row IDs to replay.  1..=1000.
    pub ids: Vec<Uuid>,
    /// Optional pin.  If unset, defaults to latest stable.  Must exist on
    /// the contract; draft targets are allowed but flagged.
    pub target_version: Option<String>,
}

impl ReplayRequest {
    /// 400 if the batch is empty or exceeds the replay cap. Shared by the
    /// contracts-scoped and RFC-081 top-level replay paths (both work from a
    /// bare id list).
    pub fn validate_ids_bounds(ids: &[Uuid]) -> AppResult<()> {
        const MAX: usize = MAX_REPLAY_IDS;
        if ids.is_empty() {
            return Err(AppError::BadRequest(
                "replay request must include at least one id".into(),
            ));
        }
        if ids.len() > MAX {
            return Err(AppError::BadRequest(format!(
                "replay request exceeds cap of {MAX} ids (got {})",
                ids.len()
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayResponse {
    pub total: usize,
    pub replayed: usize,
    pub still_quarantined: usize,
    pub already_replayed: usize,
    pub not_found: usize,
    pub wrong_contract: usize,
    pub purged: usize,
    pub redacted: usize,
    pub target_version: String,
    pub target_version_source: &'static str, // "explicit" | "default_stable"
    pub target_is_draft: bool,
    pub results: Vec<ReplayItemResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayItemResult {
    pub quarantine_id: Uuid,
    #[serde(flatten)]
    pub outcome: ReplayItemOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ReplayItemOutcome {
    /// Replay passed; source row stamped, new audit_log row created.
    Replayed {
        replayed_into_audit_id: Uuid,
        contract_version_matched: String,
    },
    /// Replay failed against the target version; new quarantine row created
    /// linked back to the source; source row untouched.
    StillQuarantined {
        new_quarantine_id: Uuid,
        contract_version_attempted: String,
        violation_count: usize,
        violations: Vec<Violation>,
    },
    /// Source row already has `status='replayed'` from a prior successful
    /// replay; no-op.
    AlreadyReplayed,
    /// No quarantine row with this ID exists.
    NotFound,
    /// Row exists but belongs to a different contract.
    WrongContract,
    /// Row exists but is in terminal `purged` state.
    Purged,
    /// RFC-086: the source body was never stored, or was purged
    /// (`payload_redacted`), so there is nothing to re-validate. Non-replayable;
    /// source row untouched.
    Redacted,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /contracts/:id/quarantine/replay`
pub async fn replay_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<ReplayRequest>,
) -> AppResult<Json<ReplayResponse>> {
    let resp =
        replay_for_contract(&state, org_id, contract_id, req.ids, req.target_version).await?;
    Ok(Json(resp))
}

/// Core per-contract replay engine, shared by the contracts-scoped route
/// (`replay_handler`) and the org-scoped top-level route (RFC-081
/// `replay_all_handler`, which groups event ids by contract and calls this
/// once per contract). Behavior is identical to the pre-RFC-081 handler.
pub async fn replay_for_contract(
    state: &Arc<AppState>,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    ids: Vec<Uuid>,
    target_version: Option<String>,
) -> AppResult<ReplayResponse> {
    // RFC-047 completion: require authenticated org context in production.
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }

    ReplayRequest::validate_ids_bounds(&ids)?;

    // Verify the contract identity exists and belongs to this org (404 on mismatch).
    let identity = storage::get_contract_identity(&state.db, contract_id, org_id).await?;

    // ----- 1. Resolve target version --------------------------------------
    let (target_version, target_source, target_is_draft) =
        resolve_replay_target(state, contract_id, target_version.as_deref(), org_id).await?;

    // ----- 2. Load source rows + categorize -------------------------------
    let rows = storage::list_quarantine_by_ids(&state.db, &ids).await?;
    let mut by_id: std::collections::HashMap<Uuid, storage::QuarantineRow> =
        rows.into_iter().map(|r| (r.id, r)).collect();

    // Allocations downstream are bounded by `validate_ids_bounds()` (≤ MAX_REPLAY_IDS)
    // but CodeQL's taint tracker can't follow that early-return guard.  Use the
    // constant directly in `with_capacity` so the allocation size is provably
    // independent of request input — the const reservation upper-bounds memory
    // at MAX_REPLAY_IDS * sizeof(T) regardless of what the client sent.
    const MAX_REPLAY_IDS: usize = 1_000;
    let bounded_ids_len = ids.len().min(MAX_REPLAY_IDS);
    let mut results: Vec<ReplayItemResult> = Vec::with_capacity(MAX_REPLAY_IDS);
    // Collected for the eligible (need-to-validate) fork below.
    struct Eligible {
        id: Uuid,
        payload: serde_json::Value,
        source_ip: Option<String>,
    }
    let mut eligible: Vec<Eligible> = Vec::new();
    // Reserve a slot per input id so we can fill in the outcome after
    // the validation pass completes.  Initially every slot is `None`;
    // non-eligible items fill their slot immediately below.
    let mut slot: Vec<Option<ReplayItemOutcome>> = (0..bounded_ids_len).map(|_| None).collect();
    let mut ordinal_for_id: std::collections::HashMap<Uuid, usize> =
        std::collections::HashMap::with_capacity(bounded_ids_len);

    for (idx, id) in ids.iter().enumerate() {
        ordinal_for_id.insert(*id, idx);
        match by_id.remove(id) {
            None => slot[idx] = Some(ReplayItemOutcome::NotFound),
            Some(r) if r.contract_id != contract_id => {
                slot[idx] = Some(ReplayItemOutcome::WrongContract);
            }
            Some(r) if r.status == "purged" => {
                slot[idx] = Some(ReplayItemOutcome::Purged);
            }
            Some(r) if r.status == "replayed" || r.replayed_at.is_some() => {
                slot[idx] = Some(ReplayItemOutcome::AlreadyReplayed);
            }
            // RFC-086: body not stored / purged → nothing to replay.
            // Same predicate the org-wide route reports via `replay_item_view`.
            Some(r) if source_body_missing(r.payload_redacted, r.payload.is_none()) => {
                slot[idx] = Some(ReplayItemOutcome::Redacted);
            }
            Some(r) => {
                eligible.push(Eligible {
                    id: r.id,
                    payload: r
                        .payload
                        .expect("payload present: prior arm handles redacted/None"),
                    source_ip: r.source_ip,
                });
            }
        }
    }

    // ----- 3. Validate eligible payloads ----------------------------------
    //
    // We mirror ingest's validation strategy: parallel-validate against the
    // target version, and (only when the caller did NOT pin a version and
    // the contract opts into fallback) retry failures across other stables.
    let target_compiled = state.get_compiled(contract_id, &target_version).await?;

    let use_fallback = target_source == "default_stable"
        && identity.multi_stable_resolution == MultiStableResolution::Fallback
        && !eligible.is_empty();

    let eligible_payloads: Vec<serde_json::Value> =
        eligible.iter().map(|e| e.payload.clone()).collect();
    let initial_results: Vec<ValidationResult> = tokio::task::spawn_blocking({
        let compiled = Arc::clone(&target_compiled);
        let payloads = eligible_payloads.clone();
        move || {
            payloads
                .par_iter()
                .map(|p| validate(&compiled, p))
                .collect()
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("replay validate join error: {e}")))?;

    // Per-event (matched_version, result).
    let mut per_event: Vec<(String, ValidationResult)> = initial_results
        .into_iter()
        .map(|vr| (target_version.clone(), vr))
        .collect();

    if use_fallback {
        let stables = storage::list_stable_versions(&state.db, contract_id).await?;
        let other_stables: Vec<_> = stables
            .into_iter()
            .filter(|v| v.version != target_version)
            .collect();
        if !other_stables.is_empty() {
            let mut compiled_fallbacks: Vec<(String, Arc<CompiledContract>)> =
                Vec::with_capacity(other_stables.len());
            for v in &other_stables {
                let cc = state.get_compiled(contract_id, &v.version).await?;
                compiled_fallbacks.push((v.version.clone(), cc));
            }
            for (idx, (matched_ver, vr)) in per_event.iter_mut().enumerate() {
                if vr.passed {
                    continue;
                }
                let event = &eligible_payloads[idx];
                let candidates: Vec<(String, ValidationResult)> = compiled_fallbacks
                    .par_iter()
                    .map(|(ver, cc)| (ver.clone(), validate(cc, event)))
                    .collect();
                if let Some((win_ver, win_vr)) = candidates.into_iter().find(|(_, vr)| vr.passed) {
                    *matched_ver = win_ver;
                    *vr = win_vr;
                }
            }
        }
    }

    // ----- 4. Build audit + quarantine inserts ----------------------------
    //
    // We pre-assign UUIDs to the pass audit rows so we can atomically link
    // each source row to the exact audit row it produced.  Failures get a
    // fresh quarantine row with `replay_of_quarantine_id` pointing back.
    let mut audit_inserts: Vec<storage::AuditEntryInsert> = Vec::new();
    let mut quar_inserts: Vec<storage::QuarantineEventInsert> = Vec::new();
    let mut forward_inserts: Vec<storage::ForwardEventInsert> = Vec::new();
    // (source_quar_id, new_audit_id) for the mark-replayed UPDATE.
    let mut replay_pairs: Vec<(Uuid, Uuid)> = Vec::new();
    // Track pass/fail outcomes we still need to record for each eligible
    // item so we can land them in the response `slot` after the DB writes.
    //
    // PassPending carries the pre-assigned audit UUID, which becomes the
    // `replayed_into_audit_id` in the response on success (or is
    // downgraded to `AlreadyReplayed` if the race guard rejected it).
    enum Pending {
        Pass {
            source_id: Uuid,
            audit_id: Uuid,
            matched_version: String,
        },
        Fail {
            source_id: Uuid,
            new_quar_id: Uuid,
            attempted_version: String,
            violations: Vec<Violation>,
        },
    }
    let mut pending: Vec<Pending> = Vec::with_capacity(eligible.len());

    for (idx, e) in eligible.iter().enumerate() {
        let (matched_ver, vr) = &per_event[idx];
        let event = &eligible_payloads[idx];
        if vr.passed {
            let audit_id = Uuid::new_v4();
            // RFC-004 §Replay: the stored quarantine payload is already in
            // post-transform form ("once transformed, forever transformed" —
            // re-running `apply_transforms` on a format-preserving mask would
            // reshuffle it).  `from_stored` is the sanctioned escape hatch
            // for this exact case.
            let stored_payload = TransformedPayload::from_stored(event.clone());
            audit_inserts.push(storage::AuditEntryInsert {
                contract_id,
                // GET /audit filters on the caller org. NULL hides the row.
                org_id: replay_pass_audit_org(org_id),
                contract_version: matched_ver.clone(),
                passed: true,
                violation_count: 0,
                violation_details: serde_json::Value::Array(vec![]),
                raw_event: stored_payload.clone(),
                validation_us: vr.validation_us as i64,
                source_ip: e.source_ip.clone(),
                source: "http".to_string(),
                pre_assigned_id: Some(audit_id),
                replay_of_quarantine_id: Some(e.id),
                direction: "ingress".to_string(),
            });
            // Q3=A: forward replay-passes, same as fresh ingest.
            forward_inserts.push(storage::ForwardEventInsert {
                contract_id,
                contract_version: matched_ver.clone(),
                payload: stored_payload,
            });
            replay_pairs.push((e.id, audit_id));
            pending.push(Pending::Pass {
                source_id: e.id,
                audit_id,
                matched_version: matched_ver.clone(),
            });
        } else {
            // Failed replay: write a fresh quarantine row.  We don't know
            // the ID Postgres will assign, so we can't echo it in the
            // response verbatim — pre-assign instead to keep the API
            // useful for subsequent lookups.  The batch insert honors
            // NULLIF-sentinel pattern, but pre-assigned IDs aren't wired
            // in for quarantine_events; so we fall back to "new UUID not
            // known until after INSERT".  Simpler: also add pre_assigned
            // behavior to quarantine_events.  For now we pre-gen a UUID
            // and use a small separate INSERT path.  (The simpler batch
            // helper does fine — we INSERT-SELECT-RETURNING id and
            // correlate by the known replay_of + row index.)
            //
            // Simplest approach that keeps the bulk INSERT: use the
            // pre-assigned UUID via an app-generated id column.  We add
            // that to QuarantineEventInsert in a follow-up (see
            // storage.rs); until then, we emit the failure row without
            // a pre-known id and fetch it back by (replay_of, created_at).
            //
            // For v1, use a RETURNING-style per-failure INSERT call
            // inline to keep the linkage atomic and well-defined.
            let violation_count = vr.violations.len();
            let violation_details = serde_json::to_value(&vr.violations)
                .unwrap_or_else(|_| serde_json::Value::Array(vec![]));
            quar_inserts.push(storage::QuarantineEventInsert {
                contract_id,
                contract_version: matched_ver.clone(),
                // RFC-004 §Replay: same reasoning as the pass-path above —
                // the source quarantine row is already post-transform, we
                // just carry it forward verbatim.
                payload: TransformedPayload::from_stored(event.clone()),
                violation_count: violation_count as i32,
                violation_details,
                validation_us: vr.validation_us as i64,
                source_ip: e.source_ip.clone(),
                replay_of_quarantine_id: Some(e.id),
                pre_assigned_id: None,
                direction: "ingress".to_string(),
            });
            // Response-side: we don't have the new row's DB id since the
            // batch helper uses uuid_generate_v4() server-side.  To keep
            // the response useful, we report the source id as the anchor
            // and omit `new_quarantine_id` when unknown.  Downstream
            // tools can query by replay_of_quarantine_id to find it.
            let new_quar_id = Uuid::nil(); // sentinel — "see replay-history"
            pending.push(Pending::Fail {
                source_id: e.id,
                new_quar_id,
                attempted_version: matched_ver.clone(),
                violations: vr.violations.clone(),
            });
        }
    }

    // ----- 5. Execute DB writes -------------------------------------------
    // Audit body and forwarded_events follow ingest's store_payloads bit.
    // The failed-replay quarantine row always keeps its body: see
    // [`replay_write_plan`]. The audit insert is awaited, so the passing
    // row exists before this function returns. forwarded_events is spawned
    // so a sink failure does not fail the replay.
    let store_payloads =
        crate::ingest::resolve_store_payloads(state, org_id, identity.store_event_payloads).await;
    let write_plan = replay_write_plan(store_payloads);
    storage::log_audit_entries_batch(&state.db, &audit_inserts, write_plan.store_audit_body)
        .await?;
    storage::quarantine_events_batch(&state.db, &quar_inserts, write_plan.store_quarantine_body)
        .await?;
    if !forward_inserts.is_empty() {
        let pool = state.db.clone();
        let to_forward = forward_inserts.clone();
        let store_forwarded = write_plan.store_forwarded_body;
        tokio::spawn(async move {
            if let Err(e) = storage::forward_events_batch(&pool, &to_forward, store_forwarded).await
            {
                tracing::warn!("replay forward insert failed: {e}");
            }
        });
    }

    // ----- 6. Atomically stamp source rows --------------------------------
    //
    // The conditional UPDATE in storage::mark_quarantine_replayed_batch only
    // stamps rows whose status is still (pending|reviewed) AND whose
    // replayed_at is NULL.  If two replay calls race for the same source
    // row, exactly one wins; the loser's source_id will be missing from
    // the returned `stamped` set and we downgrade its response slot to
    // `AlreadyReplayed`.
    let stamped: std::collections::HashSet<Uuid> =
        storage::mark_quarantine_replayed_batch(&state.db, &replay_pairs, chrono::Utc::now())
            .await?
            .into_iter()
            .collect();

    // ----- 7. Fold per-item outcomes into the response slots --------------
    for p in pending {
        match p {
            Pending::Pass {
                source_id,
                audit_id,
                matched_version,
            } => {
                let Some(&idx) = ordinal_for_id.get(&source_id) else {
                    continue;
                };
                if stamped.contains(&source_id) {
                    slot[idx] = Some(ReplayItemOutcome::Replayed {
                        replayed_into_audit_id: audit_id,
                        contract_version_matched: matched_version,
                    });
                } else {
                    // Lost the race to a concurrent replay.
                    slot[idx] = Some(ReplayItemOutcome::AlreadyReplayed);
                }
            }
            Pending::Fail {
                source_id,
                new_quar_id,
                attempted_version,
                violations,
            } => {
                let Some(&idx) = ordinal_for_id.get(&source_id) else {
                    continue;
                };
                slot[idx] = Some(ReplayItemOutcome::StillQuarantined {
                    new_quarantine_id: new_quar_id,
                    contract_version_attempted: attempted_version,
                    violation_count: violations.len(),
                    violations,
                });
            }
        }
    }

    // ----- 8. Assemble response ------------------------------------------
    for (idx, id) in ids.iter().enumerate() {
        let outcome = slot[idx].take().unwrap_or(ReplayItemOutcome::NotFound);
        results.push(ReplayItemResult {
            quarantine_id: *id,
            outcome,
        });
    }

    let counts = tally(&results);
    Ok(ReplayResponse {
        total: results.len(),
        replayed: counts.replayed,
        still_quarantined: counts.still_quarantined,
        already_replayed: counts.already_replayed,
        not_found: counts.not_found,
        wrong_contract: counts.wrong_contract,
        purged: counts.purged,
        redacted: counts.redacted,
        target_version,
        target_version_source: target_source,
        target_is_draft,
        results,
    })
}

/// `GET /contracts/:id/quarantine/:quar_id/replay-history`
pub async fn replay_history_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, quar_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<Vec<storage::ReplayHistoryEntry>>> {
    // RFC-047 completion: require authenticated org context in production.
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    // Verify contract belongs to this org before returning history (404 on mismatch).
    storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let chain = storage::replay_history_for(&state.db, contract_id, quar_id).await?;
    Ok(Json(chain))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True when a quarantine row has no body to re-validate.
///
/// `payload_redacted` is the RFC-086 marker (never stored, or purged). A NULL
/// payload without the marker is the same situation: there is nothing to
/// replay. The per-contract engine classifies those rows as
/// [`ReplayItemOutcome::Redacted`]; [`replay_item_view`] is what the org-wide
/// route uses so both responses flag them.
pub(crate) fn source_body_missing(payload_redacted: bool, payload_is_none: bool) -> bool {
    payload_redacted || payload_is_none
}

/// Fields both replay routes agree on for one engine outcome.
///
/// Per-contract `POST /contracts/{id}/quarantine/replay` serializes
/// [`ReplayItemOutcome::Redacted`] as `"outcome": "redacted"`. Org-wide
/// `POST /quarantine/replay` sets [`Self::redacted`] from this view of the
/// same outcome, instead of collapsing it into `passed: false` with an empty
/// violations list.
#[derive(Debug, Clone)]
pub struct ReplayItemView {
    pub passed: bool,
    pub version: String,
    pub violations: Vec<Violation>,
    pub redacted: bool,
}

/// Map one engine outcome onto the shared replay view.
///
/// `target_version` is the version the engine resolved for the contract. Pass
/// and still-quarantined outcomes carry their own version; every other
/// outcome (including redacted) echoes `target_version`.
pub fn replay_item_view(outcome: &ReplayItemOutcome, target_version: &str) -> ReplayItemView {
    match outcome {
        ReplayItemOutcome::Replayed {
            contract_version_matched,
            ..
        } => ReplayItemView {
            passed: true,
            version: contract_version_matched.clone(),
            violations: Vec::new(),
            redacted: false,
        },
        ReplayItemOutcome::StillQuarantined {
            contract_version_attempted,
            violations,
            ..
        } => ReplayItemView {
            passed: false,
            version: contract_version_attempted.clone(),
            violations: violations.clone(),
            redacted: false,
        },
        ReplayItemOutcome::Redacted => ReplayItemView {
            passed: false,
            version: target_version.to_string(),
            violations: Vec::new(),
            redacted: true,
        },
        ReplayItemOutcome::AlreadyReplayed
        | ReplayItemOutcome::NotFound
        | ReplayItemOutcome::WrongContract
        | ReplayItemOutcome::Purged => ReplayItemView {
            passed: false,
            version: target_version.to_string(),
            violations: Vec::new(),
            redacted: false,
        },
    }
}

/// Resolve the target version for a replay call.
///
/// Returns `(version, source, is_draft)`:
/// - `source` is `"explicit"` or `"default_stable"`.
/// - `is_draft` is `true` when the resolved version is in `draft` state.
/// - `org_id` is forwarded to `get_version` for BOLA protection (RFC-047).
pub async fn resolve_replay_target(
    state: &AppState,
    contract_id: Uuid,
    requested: Option<&str>,
    org_id: Option<Uuid>,
) -> AppResult<(String, &'static str, bool)> {
    match requested {
        Some(v) => {
            // Verify it exists and belongs to this org — 404 on mismatch.
            let row = storage::get_version(&state.db, contract_id, v, org_id).await?;
            Ok((row.version, "explicit", row.state == VersionState::Draft))
        }
        None => {
            // Default: latest stable.
            match storage::get_latest_stable_version(&state.db, contract_id).await? {
                Some(v) => Ok((v.version, "default_stable", false)),
                None => Err(AppError::NoStableVersion { contract_id }),
            }
        }
    }
}

/// Per-outcome count rollup for the response.
pub(crate) struct Tally {
    pub replayed: usize,
    pub still_quarantined: usize,
    pub already_replayed: usize,
    pub not_found: usize,
    pub wrong_contract: usize,
    pub purged: usize,
    pub redacted: usize,
}

pub(crate) fn tally(results: &[ReplayItemResult]) -> Tally {
    let mut t = Tally {
        replayed: 0,
        still_quarantined: 0,
        already_replayed: 0,
        not_found: 0,
        wrong_contract: 0,
        purged: 0,
        redacted: 0,
    };
    for r in results {
        match r.outcome {
            ReplayItemOutcome::Replayed { .. } => t.replayed += 1,
            ReplayItemOutcome::StillQuarantined { .. } => t.still_quarantined += 1,
            ReplayItemOutcome::AlreadyReplayed => t.already_replayed += 1,
            ReplayItemOutcome::NotFound => t.not_found += 1,
            ReplayItemOutcome::WrongContract => t.wrong_contract += 1,
            ReplayItemOutcome::Purged => t.purged += 1,
            ReplayItemOutcome::Redacted => t.redacted += 1,
        }
    }
    t
}

/// What a replay attempt writes for each body column.
///
/// `store_payloads` is the same bit v0/v1 ingest passes to
/// [`storage::log_audit_entries_batch`]: paid plan and org switch and
/// per-contract override. When it is false, the audit body is `{}` and
/// `raw_event_redacted` is set.
///
/// Ingest quarantine uses that same bit (SQL `NULL` when off), and those
/// rows are not replayable. A failed replay is different: it only runs
/// because the source row already had a body, and the new quarantine row
/// has to keep that body so a later replay can run. That write stays on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayWritePlan {
    pub store_audit_body: bool,
    pub store_quarantine_body: bool,
    pub store_forwarded_body: bool,
}

pub(crate) fn replay_write_plan(store_payloads: bool) -> ReplayWritePlan {
    ReplayWritePlan {
        store_audit_body: store_payloads,
        store_quarantine_body: true,
        store_forwarded_body: store_payloads,
    }
}

/// Org written on a replay-pass `audit_log` row.
///
/// `GET /audit` selects `WHERE org_id = $caller`. The insert is awaited
/// before the replay response returns. A NULL org does not match that
/// filter, so the passing row never shows up on `/audit`.
pub(crate) fn replay_pass_audit_org(caller_org: Option<Uuid>) -> Option<Uuid> {
    caller_org
}

// ---------------------------------------------------------------------------
// Unit tests (RFC-003 §test plan, DB-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Q1: validate_bounds --------------------------------------------

    #[test]
    fn rejects_empty_id_list() {
        let r = ReplayRequest {
            ids: vec![],
            target_version: None,
        };
        let err = ReplayRequest::validate_ids_bounds(&r.ids).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn rejects_over_cap() {
        let r = ReplayRequest {
            ids: (0..1_001).map(|_| Uuid::new_v4()).collect(),
            target_version: None,
        };
        let err = ReplayRequest::validate_ids_bounds(&r.ids).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn accepts_at_cap() {
        let r = ReplayRequest {
            ids: (0..1_000).map(|_| Uuid::new_v4()).collect(),
            target_version: None,
        };
        ReplayRequest::validate_ids_bounds(&r.ids).unwrap();
    }

    #[test]
    fn accepts_single_id() {
        let r = ReplayRequest {
            ids: vec![Uuid::new_v4()],
            target_version: None,
        };
        ReplayRequest::validate_ids_bounds(&r.ids).unwrap();
    }

    // ---- Q2: tally consistency with ReplayItemOutcome variants ----------

    fn fake_item(outcome: ReplayItemOutcome) -> ReplayItemResult {
        ReplayItemResult {
            quarantine_id: Uuid::new_v4(),
            outcome,
        }
    }

    #[test]
    fn tally_sums_every_outcome_kind() {
        let results = vec![
            fake_item(ReplayItemOutcome::Replayed {
                replayed_into_audit_id: Uuid::new_v4(),
                contract_version_matched: "1.0.0".into(),
            }),
            fake_item(ReplayItemOutcome::Replayed {
                replayed_into_audit_id: Uuid::new_v4(),
                contract_version_matched: "1.1.0".into(),
            }),
            fake_item(ReplayItemOutcome::StillQuarantined {
                new_quarantine_id: Uuid::nil(),
                contract_version_attempted: "1.0.0".into(),
                violation_count: 1,
                violations: vec![],
            }),
            fake_item(ReplayItemOutcome::AlreadyReplayed),
            fake_item(ReplayItemOutcome::NotFound),
            fake_item(ReplayItemOutcome::WrongContract),
            fake_item(ReplayItemOutcome::Purged),
            fake_item(ReplayItemOutcome::Purged),
            fake_item(ReplayItemOutcome::Redacted),
        ];
        let t = tally(&results);
        assert_eq!(t.replayed, 2);
        assert_eq!(t.still_quarantined, 1);
        assert_eq!(t.already_replayed, 1);
        assert_eq!(t.not_found, 1);
        assert_eq!(t.wrong_contract, 1);
        assert_eq!(t.purged, 2);
        assert_eq!(t.redacted, 1);
        // Counts roll up to input length — fundamental invariant.
        let sum = t.replayed
            + t.still_quarantined
            + t.already_replayed
            + t.not_found
            + t.wrong_contract
            + t.purged
            + t.redacted;
        assert_eq!(sum, results.len());
    }

    // ---- Q3: response shape / serde round-trip --------------------------

    #[test]
    fn replay_request_deserializes_with_target_version() {
        let body = r#"{"ids":["00000000-0000-0000-0000-000000000001"],"target_version":"2.1.0"}"#;
        let req: ReplayRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.ids.len(), 1);
        assert_eq!(req.target_version.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn replay_request_deserializes_without_target_version() {
        let body = r#"{"ids":["00000000-0000-0000-0000-000000000001"]}"#;
        let req: ReplayRequest = serde_json::from_str(body).unwrap();
        assert!(req.target_version.is_none());
    }

    #[test]
    fn replay_item_outcome_serialization_tags_correctly() {
        let o = ReplayItemOutcome::Replayed {
            replayed_into_audit_id: Uuid::nil(),
            contract_version_matched: "1.0.0".into(),
        };
        let s = serde_json::to_string(&o).unwrap();
        assert!(s.contains("\"outcome\":\"replayed\""));
        assert!(s.contains("\"contract_version_matched\":\"1.0.0\""));

        let o = ReplayItemOutcome::AlreadyReplayed;
        let s = serde_json::to_string(&o).unwrap();
        assert!(s.contains("\"outcome\":\"already_replayed\""));

        let o = ReplayItemOutcome::NotFound;
        let s = serde_json::to_string(&o).unwrap();
        assert!(s.contains("\"outcome\":\"not_found\""));

        // Per-contract flag for a row with no stored body.
        let redacted = ReplayItemResult {
            quarantine_id: Uuid::nil(),
            outcome: ReplayItemOutcome::Redacted,
        };
        let wire = serde_json::to_value(&redacted).unwrap();
        assert_eq!(wire["outcome"], "redacted");

        // Org-wide view of that same outcome. `redacted: true`, not a bare
        // `passed: false` with an empty violations list (that is what purged /
        // not-found look like).
        let view = replay_item_view(&redacted.outcome, "1.0.0");
        assert!(view.redacted);
        assert!(!view.passed);
        assert!(view.violations.is_empty());
        assert_eq!(view.version, "1.0.0");

        let purged = replay_item_view(&ReplayItemOutcome::Purged, "1.0.0");
        assert!(!purged.redacted);
        assert!(!purged.passed);
        assert!(source_body_missing(true, false));
        assert!(source_body_missing(false, true));
        assert!(!source_body_missing(false, false));
    }

    #[test]
    fn replay_storage_follows_ingest_except_requarantine() {
        use crate::plan::payloads_stored;

        // Free, both switches on. Audit and forward omit the body.
        // The new quarantine row still keeps it.
        let free = replay_write_plan(payloads_stored("free", true, true));
        assert!(!free.store_audit_body, "free audit body");
        assert!(free.store_quarantine_body, "free re-quarantine keeps body");
        assert!(!free.store_forwarded_body, "free forward body");
        assert!(!replay_write_plan(payloads_stored("aisg_free", true, true)).store_audit_body);

        // Paid, org master switch off.
        let org_off = replay_write_plan(payloads_stored("growth", false, true));
        assert!(!org_off.store_audit_body);
        assert!(org_off.store_quarantine_body);
        assert!(!org_off.store_forwarded_body);
        assert!(!replay_write_plan(payloads_stored("aisg_pro", false, true)).store_audit_body);

        // Paid, per-contract override off.
        let contract_off = replay_write_plan(payloads_stored("enterprise", true, false));
        assert!(!contract_off.store_audit_body);
        assert!(contract_off.store_quarantine_body);
        assert!(!contract_off.store_forwarded_body);

        // Paid, both switches on. All three bodies stored.
        let on = replay_write_plan(payloads_stored("growth", true, true));
        assert!(on.store_audit_body);
        assert!(on.store_quarantine_body);
        assert!(on.store_forwarded_body);
        assert!(replay_write_plan(payloads_stored("aisg_pro", true, true)).store_audit_body);
        assert!(replay_write_plan(payloads_stored("enterprise", true, true)).store_forwarded_body);
    }

    #[test]
    fn replay_pass_audit_org_matches_audit_list() {
        let org = Uuid::new_v4();
        let stamped = replay_pass_audit_org(Some(org));
        // GET /audit: WHERE org_id = $caller. NULL does not match.
        assert_eq!(stamped, Some(org));
        assert_ne!(replay_pass_audit_org(None), Some(org));
    }
}
