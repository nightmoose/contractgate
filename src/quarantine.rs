//! RFC-081 — org-scoped top-level quarantine endpoints backing the dashboard
//! Quarantine tab.
//!
//! These wrap the tested per-contract replay engine
//! (`replay::replay_for_contract`) and the org-scoped storage queries; they add
//! no new validation logic. Endpoints:
//!
//! * `GET  /quarantine`                — list source quarantine rows (org-scoped)
//! * `POST /quarantine/replay`         — replay by event id across contracts
//! * `GET  /quarantine/replay-history` — attempt history for one event
//!
//! Org scope: every query joins `quarantine_events → contracts` and filters on
//! the caller's org; cross-org ids are silently dropped (never leaked). In
//! production (`auth_configured()`) a missing org → 401.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::replay::{replay_for_contract, ReplayItemOutcome, ReplayRequest};
use crate::storage;
use crate::validation::Violation;
use crate::{AppState, OrgId};

// ---------------------------------------------------------------------------
// GET /quarantine
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct QuarantineListQuery {
    pub contract_id: Option<Uuid>,
    #[serde(default = "default_list_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_list_limit() -> i64 {
    100
}

/// One quarantined event as the dashboard expects it (matches
/// `dashboard/lib/api.ts::QuarantinedEvent`).
#[derive(Debug, Serialize)]
pub struct QuarantinedEventOut {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_version: Option<String>,
    pub raw_event: serde_json::Value,
    pub violation_details: serde_json::Value,
    pub violation_count: i32,
    pub source_ip: Option<String>,
    pub quarantined_at: chrono::DateTime<chrono::Utc>,
    pub replay_count: i64,
    pub last_replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_replay_passed: Option<bool>,
}

/// `GET /quarantine?contract_id=&limit=&offset=`
pub async fn list_quarantine_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Query(q): Query<QuarantineListQuery>,
) -> AppResult<Json<Vec<QuarantinedEventOut>>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let rows =
        storage::list_quarantine_events(&state.db, org_id, q.contract_id, limit, offset).await?;

    let out = rows
        .into_iter()
        .map(|r| QuarantinedEventOut {
            id: r.id,
            contract_id: r.contract_id,
            contract_version: r.contract_version,
            raw_event: r.payload,
            violation_details: r.violation_details,
            violation_count: r.violation_count,
            source_ip: r.source_ip,
            quarantined_at: r.created_at,
            replay_count: r.replay_count,
            last_replayed_at: r.last_replayed_at,
            last_replay_passed: r.last_replay_passed,
        })
        .collect();
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// POST /quarantine/replay
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReplayAllRequest {
    pub event_ids: Vec<Uuid>,
    #[serde(default)]
    pub version: Option<String>,
    /// Optional assertion: every event must belong to this contract (400 if not).
    #[serde(default)]
    pub contract_id: Option<Uuid>,
}

/// One replay outcome (matches `dashboard/lib/api.ts::ReplayOutcome`).
#[derive(Debug, Serialize)]
pub struct ReplayOutcomeOut {
    pub event_id: Uuid,
    pub version: String,
    pub passed: bool,
    pub violations: Vec<Violation>,
    pub replayed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReplayAllResponse {
    pub replayed: usize,
    pub outcomes: Vec<ReplayOutcomeOut>,
}

/// `POST /quarantine/replay`  body `{event_ids, version?, contract_id?}`
///
/// The replay engine resolves one target version per contract, so ids are
/// grouped by their owning contract and replayed one group at a time, then the
/// per-event results are flattened back into input order.
pub async fn replay_all_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Json(req): Json<ReplayAllRequest>,
) -> AppResult<Json<ReplayAllResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    ReplayRequest::validate_ids_bounds(&req.event_ids)?;

    let now = chrono::Utc::now();
    let requested_version = req.version.clone().unwrap_or_default();

    // Resolve ids → contract, org-scoped. Unresolved = cross-org or nonexistent.
    let contract_of: HashMap<Uuid, Uuid> =
        storage::resolve_quarantine_contracts(&state.db, org_id, &req.event_ids)
            .await?
            .into_iter()
            .collect();

    // Optional contract_id assertion.
    if let Some(cid) = req.contract_id {
        if contract_of.values().any(|c| *c != cid) {
            return Err(AppError::BadRequest(
                "event_ids span multiple contracts or a contract other than contract_id".into(),
            ));
        }
    }

    // Group by contract.
    let mut by_contract: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    for id in &req.event_ids {
        if let Some(cid) = contract_of.get(id) {
            by_contract.entry(*cid).or_default().push(*id);
        }
    }

    // Replay each contract group and index outcomes by event id.
    let mut outcome_for: HashMap<Uuid, ReplayOutcomeOut> = HashMap::new();
    for (cid, ids) in by_contract {
        let resp = replay_for_contract(&state, org_id, cid, ids, req.version.clone()).await?;
        let target = resp.target_version.clone();
        for item in resp.results {
            let (passed, version, violations) = match item.outcome {
                ReplayItemOutcome::Replayed {
                    contract_version_matched,
                    ..
                } => (true, contract_version_matched, Vec::new()),
                ReplayItemOutcome::StillQuarantined {
                    contract_version_attempted,
                    violations,
                    ..
                } => (false, contract_version_attempted, violations),
                ReplayItemOutcome::AlreadyReplayed
                | ReplayItemOutcome::NotFound
                | ReplayItemOutcome::WrongContract
                | ReplayItemOutcome::Purged => (false, target.clone(), Vec::new()),
            };
            outcome_for.insert(
                item.quarantine_id,
                ReplayOutcomeOut {
                    event_id: item.quarantine_id,
                    version,
                    passed,
                    violations,
                    replayed_at: now,
                },
            );
        }
    }

    // Emit in input order; unresolved ids get a not-found-style outcome.
    let mut outcomes = Vec::with_capacity(req.event_ids.len());
    for id in &req.event_ids {
        match outcome_for.remove(id) {
            Some(o) => outcomes.push(o),
            None => outcomes.push(ReplayOutcomeOut {
                event_id: *id,
                version: requested_version.clone(),
                passed: false,
                violations: Vec::new(),
                replayed_at: now,
            }),
        }
    }
    let replayed = outcomes.iter().filter(|o| o.passed).count();
    Ok(Json(ReplayAllResponse { replayed, outcomes }))
}

// ---------------------------------------------------------------------------
// GET /quarantine/replay-history
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReplayHistoryQuery {
    pub event_id: Uuid,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
}

fn default_history_limit() -> i64 {
    100
}

/// `GET /quarantine/replay-history?event_id=&limit=`
pub async fn replay_history_all_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Query(q): Query<ReplayHistoryQuery>,
) -> AppResult<Json<Vec<ReplayOutcomeOut>>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    // Resolve the event's contract, org-scoped. Unknown/cross-org → empty.
    let resolved =
        storage::resolve_quarantine_contracts(&state.db, org_id, &[q.event_id]).await?;
    let Some((_, contract_id)) = resolved.into_iter().next() else {
        return Ok(Json(vec![]));
    };

    let chain = storage::replay_history_for(&state.db, contract_id, q.event_id).await?;
    let mut out: Vec<ReplayOutcomeOut> = Vec::new();
    for entry in chain {
        use storage::ReplayHistoryEntry::{FailedReplay, PassedReplay, Source};
        match entry {
            // The source row is the origin, not a replay attempt.
            Source { .. } => {}
            FailedReplay {
                contract_version,
                created_at,
                ..
            } => out.push(ReplayOutcomeOut {
                event_id: q.event_id,
                version: contract_version,
                passed: false,
                violations: Vec::new(),
                replayed_at: created_at,
            }),
            PassedReplay {
                contract_version,
                created_at,
                ..
            } => out.push(ReplayOutcomeOut {
                event_id: q.event_id,
                version: contract_version,
                passed: true,
                violations: Vec::new(),
                replayed_at: created_at,
            }),
        }
    }
    out.truncate(q.limit.clamp(1, 500) as usize);
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the wire field names to `dashboard/lib/api.ts`. A rename on either
    /// side (the exact bug RFC-081 fixes) fails here instead of silently 404ing
    /// or dropping fields in the UI.
    #[test]
    fn quarantined_event_out_matches_frontend_shape() {
        let ev = QuarantinedEventOut {
            id: Uuid::nil(),
            contract_id: Uuid::nil(),
            contract_version: Some("1.0.0".into()),
            raw_event: serde_json::json!({"k": "v"}),
            violation_details: serde_json::json!([]),
            violation_count: 0,
            source_ip: None,
            quarantined_at: chrono::Utc::now(),
            replay_count: 0,
            last_replayed_at: None,
            last_replay_passed: None,
        };
        let v = serde_json::to_value(&ev).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "id",
            "contract_id",
            "contract_version",
            "raw_event",
            "violation_details",
            "violation_count",
            "source_ip",
            "quarantined_at",
            "replay_count",
            "last_replayed_at",
            "last_replay_passed",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected, "QuarantinedEvent wire shape drifted");
    }

    #[test]
    fn replay_outcome_out_matches_frontend_shape() {
        let o = ReplayOutcomeOut {
            event_id: Uuid::nil(),
            version: "1.0.0".into(),
            passed: true,
            violations: Vec::new(),
            replayed_at: chrono::Utc::now(),
        };
        let v = serde_json::to_value(&o).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> =
            ["event_id", "version", "passed", "violations", "replayed_at"]
                .into_iter()
                .collect();
        assert_eq!(keys, expected, "ReplayOutcome wire shape drifted");

        // ReplayAllResponse envelope: { replayed, outcomes }.
        let resp = ReplayAllResponse {
            replayed: 1,
            outcomes: vec![o],
        };
        let rv = serde_json::to_value(&resp).unwrap();
        let rkeys: std::collections::BTreeSet<&str> =
            rv.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            rkeys,
            ["replayed", "outcomes"].into_iter().collect(),
            "ReplayResponse envelope drifted"
        );
    }
}
