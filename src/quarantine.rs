//! RFC-081 — org-scoped top-level quarantine endpoints backing the dashboard
//! Quarantine tab.
//!
//! These wrap the tested per-contract replay engine
//! (`replay::replay_for_contract`) and the org-scoped storage queries; they add
//! no new validation logic. Endpoints:
//!
//! * `GET  /quarantine`                — list source quarantine rows (org-scoped)
//! * `GET  /quarantine/summary`        — counts by status and direction for one run
//! * `POST /quarantine/replay`         — replay by event id across contracts
//! * `GET  /quarantine/replay-history` — attempt history for one event
//!
//! Org scope: every query joins `quarantine_events → contracts` and filters on
//! the caller's org; cross-org ids are silently dropped (never leaked). In
//! production (`auth_configured()`) a missing org → 401.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::replay::{replay_for_contract, replay_item_view, ReplayRequest};
use crate::storage;
use crate::validation::Violation;
use crate::{AppState, OrgId};

// ---------------------------------------------------------------------------
// GET /quarantine
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct QuarantineListQuery {
    pub contract_id: Option<Uuid>,
    /// Run id stored on the row. Alias of `run_id`.
    #[serde(default)]
    pub batch_id: Option<String>,
    /// Alias of `batch_id`.
    #[serde(default)]
    pub run_id: Option<String>,
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
    /// RFC-086: body was never stored or was purged; `raw_event` is `null` and
    /// the row is non-replayable.
    pub payload_redacted: bool,
    pub violation_details: serde_json::Value,
    pub violation_count: i32,
    pub source_ip: Option<String>,
    pub quarantined_at: chrono::DateTime<chrono::Utc>,
    pub replay_count: i64,
    pub last_replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_replay_passed: Option<bool>,
    pub status: String,
    /// Run id supplied on the ingest or egress call. Null when omitted.
    pub batch_id: Option<String>,
    pub replayable: bool,
    /// `plan`, `body_not_stored`, `purged`, or null when replayable.
    pub replay_unavailable_reason: Option<&'static str>,
}

/// `GET /quarantine?contract_id=&limit=&offset=`
pub async fn list_quarantine_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    headers: HeaderMap,
    Query(q): Query<QuarantineListQuery>,
) -> AppResult<Json<Vec<QuarantinedEventOut>>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    let batch_id =
        crate::batch_id::resolve_batch_id(q.batch_id.as_deref(), q.run_id.as_deref(), &headers)?;
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);
    let rows = storage::list_quarantine_events(
        &state.db,
        org_id,
        q.contract_id,
        batch_id.as_deref(),
        limit,
        offset,
    )
    .await?;

    let replay_allowed = replay_allowed(&state, org_id).await?;
    let out = rows
        .into_iter()
        .map(|r| {
            let missing = r.payload.is_none();
            let (replayable, reason) =
                replay_availability(replay_allowed, &r.status, r.payload_redacted, missing);
            QuarantinedEventOut {
                id: r.id,
                contract_id: r.contract_id,
                contract_version: r.contract_version,
                raw_event: r.payload.unwrap_or(serde_json::Value::Null),
                payload_redacted: r.payload_redacted,
                violation_details: r.violation_details,
                violation_count: r.violation_count,
                source_ip: r.source_ip,
                quarantined_at: r.created_at,
                replay_count: r.replay_count,
                last_replayed_at: r.last_replayed_at,
                last_replay_passed: r.last_replay_passed,
                status: r.status,
                batch_id: r.batch_id,
                replayable,
                replay_unavailable_reason: reason,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn replay_allowed(state: &AppState, org_id: Option<Uuid>) -> AppResult<bool> {
    if state.self_hosted || org_id.is_none() {
        return Ok(true);
    }
    match storage::get_org_plan(&state.db, org_id.expect("org checked")).await {
        Ok(plan) => Ok(crate::plan::gate_paid(
            false,
            plan.as_deref().unwrap_or("free"),
        )),
        Err(err) => {
            tracing::warn!("quarantine list plan lookup failed: {err}");
            Ok(true)
        }
    }
}

pub fn replay_availability(
    replay_allowed: bool,
    status: &str,
    payload_redacted: bool,
    payload_missing: bool,
) -> (bool, Option<&'static str>) {
    if !replay_allowed {
        return (false, Some("plan"));
    }
    if status == "purged" {
        return (false, Some("purged"));
    }
    if payload_redacted || payload_missing {
        return (false, Some("body_not_stored"));
    }
    (true, None)
}

// ---------------------------------------------------------------------------
// GET /quarantine/summary
// ---------------------------------------------------------------------------

const QUARANTINE_STATUSES: [&str; 4] = ["pending", "reviewed", "replayed", "purged"];
const QUARANTINE_DIRECTIONS: [&str; 2] = ["ingress", "egress"];

#[derive(Debug, Deserialize)]
pub struct RunSummaryQuery {
    pub contract_id: Option<Uuid>,
    /// Run id stored on the row. Alias of `run_id`.
    #[serde(default)]
    pub batch_id: Option<String>,
    /// Alias of `batch_id`.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Counts for one lifecycle status. Missing rows are zero.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusCounts {
    pub pending: i64,
    pub reviewed: i64,
    pub replayed: i64,
    pub purged: i64,
}

/// Counts for one traffic direction. Missing rows are zero.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DirectionCounts {
    pub ingress: i64,
    pub egress: i64,
}

/// One status × direction cell. The known eight cells are always present.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusDirectionCount {
    pub status: String,
    pub direction: String,
    pub count: i64,
}

/// `GET /quarantine/summary` body. `pending` is `by_status.pending`, the number
/// an orchestrator compares with zero.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QuarantineRunSummary {
    pub contract_id: Option<Uuid>,
    pub batch_id: Option<String>,
    pub by_status: StatusCounts,
    pub by_direction: DirectionCounts,
    pub by_status_and_direction: Vec<StatusDirectionCount>,
    pub total: i64,
    pub pending: i64,
}

/// Fold raw `(status, direction, count)` rows into a stable summary.
///
/// The four statuses and two directions are always present, even at zero.
/// A status or direction outside that set is kept in `by_status_and_direction`
/// and in `total`.
pub fn assemble_run_summary(
    contract_id: Option<Uuid>,
    batch_id: Option<String>,
    rows: impl IntoIterator<Item = (String, String, i64)>,
) -> QuarantineRunSummary {
    let mut cells: BTreeMap<(String, String), i64> = BTreeMap::new();
    for (status, direction, count) in rows {
        *cells.entry((status, direction)).or_insert(0) += count;
    }

    let mut by_status = StatusCounts {
        pending: 0,
        reviewed: 0,
        replayed: 0,
        purged: 0,
    };
    let mut by_direction = DirectionCounts {
        ingress: 0,
        egress: 0,
    };
    let mut total = 0i64;
    for ((status, direction), count) in &cells {
        total += count;
        match status.as_str() {
            "pending" => by_status.pending += count,
            "reviewed" => by_status.reviewed += count,
            "replayed" => by_status.replayed += count,
            "purged" => by_status.purged += count,
            _ => {}
        }
        match direction.as_str() {
            "ingress" => by_direction.ingress += count,
            "egress" => by_direction.egress += count,
            _ => {}
        }
    }

    let mut breakdown = Vec::with_capacity(cells.len());
    for status in QUARANTINE_STATUSES {
        for direction in QUARANTINE_DIRECTIONS {
            let count = cells
                .get(&(status.to_string(), direction.to_string()))
                .copied()
                .unwrap_or(0);
            breakdown.push(StatusDirectionCount {
                status: status.to_string(),
                direction: direction.to_string(),
                count,
            });
        }
    }
    for ((status, direction), count) in &cells {
        let known_status = QUARANTINE_STATUSES.contains(&status.as_str());
        let known_direction = QUARANTINE_DIRECTIONS.contains(&direction.as_str());
        if known_status && known_direction {
            continue;
        }
        breakdown.push(StatusDirectionCount {
            status: status.clone(),
            direction: direction.clone(),
            count: *count,
        });
    }

    let pending = by_status.pending;
    QuarantineRunSummary {
        contract_id,
        batch_id,
        by_status,
        by_direction,
        by_status_and_direction: breakdown,
        total,
        pending,
    }
}

/// `GET /quarantine/summary?contract_id=&batch_id=`
///
/// Counts source quarantine rows for the caller's org. `batch_id` (alias
/// `run_id`, or the `X-Batch-Id` / `X-Run-Id` headers) limits the count to one
/// pipeline run. Omit it to count every source row in the contract or org.
pub async fn run_summary_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    headers: HeaderMap,
    Query(q): Query<RunSummaryQuery>,
) -> AppResult<Json<QuarantineRunSummary>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    let batch_id =
        crate::batch_id::resolve_batch_id(q.batch_id.as_deref(), q.run_id.as_deref(), &headers)?;
    if let Some(contract_id) = q.contract_id {
        storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    }
    let rows = storage::summarize_quarantine(&state.db, org_id, q.contract_id, batch_id.as_deref())
        .await?;
    let summary = assemble_run_summary(
        q.contract_id,
        batch_id,
        rows.into_iter()
            .map(|row| (row.status, row.direction, row.count)),
    );
    Ok(Json(summary))
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
    /// True when the source body was never stored or was purged. Same
    /// classification as the per-contract route's `"outcome": "redacted"`.
    /// `passed` is false and `violations` is empty; the row is not a
    /// validation failure.
    pub redacted: bool,
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
    // Unresolved ids never enter the per-contract engine, which is where the
    // plan check lives. Reject a free plan here so this route still returns
    // 403 plan_required.
    crate::plan::require_plan_feature(&state, org_id, "quarantine_replay").await?;

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
            // Shared with the per-contract route: Redacted stays redacted
            // instead of collapsing into passed:false / violations:[].
            let view = replay_item_view(&item.outcome, &target);
            outcome_for.insert(
                item.quarantine_id,
                ReplayOutcomeOut {
                    event_id: item.quarantine_id,
                    version: view.version,
                    passed: view.passed,
                    violations: view.violations,
                    replayed_at: now,
                    redacted: view.redacted,
                },
            );
        }
    }

    // Emit in input order; unresolved ids get a not-found-style outcome.
    // No pre-sizing from the request body: ids are capped at MAX_REPLAY_IDS
    // by validate_ids_bounds, so growing the Vec on demand is cheap.
    let mut outcomes = Vec::new();
    for id in &req.event_ids {
        match outcome_for.remove(id) {
            Some(o) => outcomes.push(o),
            None => outcomes.push(ReplayOutcomeOut {
                event_id: *id,
                version: requested_version.clone(),
                passed: false,
                violations: Vec::new(),
                replayed_at: now,
                redacted: false,
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
    let resolved = storage::resolve_quarantine_contracts(&state.db, org_id, &[q.event_id]).await?;
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
                redacted: false,
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
                redacted: false,
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
            payload_redacted: false,
            violation_details: serde_json::json!([]),
            violation_count: 0,
            source_ip: None,
            quarantined_at: chrono::Utc::now(),
            replay_count: 0,
            last_replayed_at: None,
            last_replay_passed: None,
            status: "pending".into(),
            batch_id: None,
            replayable: true,
            replay_unavailable_reason: None,
        };
        let v = serde_json::to_value(&ev).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "id",
            "contract_id",
            "contract_version",
            "raw_event",
            "payload_redacted",
            "violation_details",
            "violation_count",
            "source_ip",
            "quarantined_at",
            "replay_count",
            "last_replayed_at",
            "last_replay_passed",
            "status",
            "batch_id",
            "replayable",
            "replay_unavailable_reason",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected, "QuarantinedEvent wire shape drifted");
    }

    #[test]
    fn run_summary_zero_fills_and_sums_one_batch() {
        let empty = assemble_run_summary(None, None, []);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.pending, 0);
        assert_eq!(empty.by_status.pending, 0);
        assert_eq!(empty.by_status.reviewed, 0);
        assert_eq!(empty.by_direction.ingress, 0);
        assert_eq!(empty.by_direction.egress, 0);
        assert_eq!(empty.by_status_and_direction.len(), 8);

        let summary = assemble_run_summary(
            Some(Uuid::nil()),
            Some("run-1".into()),
            [
                ("pending".into(), "ingress".into(), 2),
                ("pending".into(), "egress".into(), 1),
                ("reviewed".into(), "ingress".into(), 4),
            ],
        );
        assert_eq!(summary.batch_id.as_deref(), Some("run-1"));
        assert_eq!(summary.pending, 3);
        assert_eq!(summary.by_status.pending, 3);
        assert_eq!(summary.by_status.reviewed, 4);
        assert_eq!(summary.by_status.replayed, 0);
        assert_eq!(summary.by_status.purged, 0);
        assert_eq!(summary.by_direction.ingress, 6);
        assert_eq!(summary.by_direction.egress, 1);
        assert_eq!(summary.total, 7);
        let pending_ingress = summary
            .by_status_and_direction
            .iter()
            .find(|cell| cell.status == "pending" && cell.direction == "ingress")
            .unwrap();
        assert_eq!(pending_ingress.count, 2);
        let purged_egress = summary
            .by_status_and_direction
            .iter()
            .find(|cell| cell.status == "purged" && cell.direction == "egress")
            .unwrap();
        assert_eq!(purged_egress.count, 0);
    }

    #[test]
    fn replay_availability_reasons() {
        assert_eq!(
            replay_availability(false, "pending", true, true),
            (false, Some("plan"))
        );
        assert_eq!(
            replay_availability(true, "purged", false, false),
            (false, Some("purged"))
        );
        assert_eq!(
            replay_availability(true, "pending", true, false),
            (false, Some("body_not_stored"))
        );
        assert_eq!(
            replay_availability(true, "pending", false, false),
            (true, None)
        );
        assert_eq!(
            replay_availability(true, "reviewed", false, false),
            (true, None)
        );
    }

    /// `POST /quarantine/replay` used to resolve ids first. Unknown ids never
    /// reached `replay_for_contract`, so a Free plan got 200 instead of
    /// 403 `plan_required`.
    #[test]
    fn org_replay_checks_the_plan_before_resolving_ids() {
        let src = include_str!("quarantine.rs");
        let start = src
            .find("pub async fn replay_all_handler")
            .expect("handler");
        let end = src[start..]
            .find("pub async fn replay_history_all_handler")
            .expect("next handler")
            + start;
        let body = &src[start..end];
        let plan = body
            .find("require_plan_feature")
            .expect("org replay must reject a free plan");
        let resolve = body.find("resolve_quarantine_contracts").expect("resolve");
        assert!(
            plan < resolve,
            "unresolved ids must not skip the plan check"
        );
        assert!(body.contains("\"quarantine_replay\""));
    }

    #[test]
    fn replay_outcome_out_matches_frontend_shape() {
        let o = ReplayOutcomeOut {
            event_id: Uuid::nil(),
            version: "1.0.0".into(),
            passed: true,
            violations: Vec::new(),
            replayed_at: chrono::Utc::now(),
            redacted: false,
        };
        let v = serde_json::to_value(&o).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "event_id",
            "version",
            "passed",
            "violations",
            "replayed_at",
            "redacted",
        ]
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
