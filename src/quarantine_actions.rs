//! Free CSV export, mark-reviewed, cap summary, and Pro bulk replay.
//!
//! CSV rows carry the failure time, version, reason, and field path. They
//! do not carry the record body. Bulk replay is a background job bounded by
//! [`crate::plan::bulk_replay_job_limit`].

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::plan;
use crate::replay::{replay_for_contract, MAX_REPLAY_IDS};
use crate::storage::{self, QuarantineExportRow};
use crate::{AppState, OrgId};

const JOB_STOPPED: &str = "The replay job stopped. Try again.";

#[derive(Debug, Deserialize)]
pub struct ReviewRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ReviewResponse {
    pub reviewed: u64,
}

#[derive(Debug, Serialize)]
pub struct QuarantineSummary {
    pub org_pending: i64,
    pub org_cap: Option<i64>,
    pub contract_pending: i64,
    pub contract_cap: Option<i64>,
    pub state: &'static str,
    pub scope: Option<&'static str>,
    pub retention_days: i32,
    pub replay_allowed: bool,
}

#[derive(Debug, Deserialize)]
pub struct BulkReplayRequest {
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default)]
    pub contract_version: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReplayJobOut {
    pub job_id: Uuid,
    pub status: String,
    pub record_limit: i32,
    pub selected: i32,
    pub replayed: i32,
    pub failed: i32,
    pub skipped: i32,
    pub error: Option<String>,
}

/// `GET /contracts/{id}/quarantine/export`
pub async fn export_quarantine_csv(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let rows = storage::list_quarantine_export(
        &state.db,
        org_id,
        contract_id,
        plan::FREE_QUARANTINE_ORG_CAP,
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().expect("csv content type"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"quarantine-failures.csv\""
            .parse()
            .expect("csv disposition"),
    );
    Ok((headers, quarantine_csv(&rows)))
}

/// `POST /contracts/{id}/quarantine/review`
pub async fn review_quarantine_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<ReviewRequest>,
) -> AppResult<Json<ReviewResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    if req.ids.is_empty() {
        return Err(AppError::BadRequest(
            "review request must include at least one id".into(),
        ));
    }
    if req.ids.len() > MAX_REPLAY_IDS {
        return Err(AppError::BadRequest(format!(
            "review request exceeds cap of {MAX_REPLAY_IDS} ids (got {})",
            req.ids.len()
        )));
    }
    storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let reviewed =
        storage::mark_quarantine_reviewed(&state.db, org_id, contract_id, &req.ids).await?;
    if let Some(org_id) = org_id {
        crate::quarantine_cap::note_pending_delta(org_id, contract_id, -(reviewed as i64));
    }
    Ok(Json(ReviewResponse { reviewed }))
}

/// `GET /contracts/{id}/quarantine/summary`
pub async fn quarantine_summary_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<QuarantineSummary>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let Some(org_id) = org_id else {
        let days = if state.self_hosted {
            plan::log_retention_days("enterprise")
        } else {
            plan::log_retention_days("free")
        };
        return Ok(Json(uncapped_summary(true, days)));
    };
    let plan_name = storage::get_org_plan(&state.db, org_id)
        .await?
        .unwrap_or_else(|| "free".to_string());
    let replay_allowed = plan::gate_paid(state.self_hosted, &plan_name);
    let retention = plan::gate_retention_days(state.self_hosted, &plan_name);
    let org_cap = plan::quarantine_row_cap_for(state.self_hosted, &plan_name);
    let contract_cap = plan::per_contract_quarantine_cap_for(state.self_hosted, &plan_name);
    let (org_pending, contract_pending) =
        storage::pending_quarantine_counts(&state.db, org_id, contract_id).await?;
    let view = crate::drop_mode::quarantine_cap_view(crate::drop_mode::PlanCapCounts {
        org_pending,
        org_limit: org_cap,
        contract_pending,
        contract_limit: contract_cap,
        near_percent: plan::QUARANTINE_CAP_NEAR_PERCENT,
    });
    Ok(Json(QuarantineSummary {
        org_pending,
        org_cap,
        contract_pending,
        contract_cap,
        state: view.map(|view| view.state).unwrap_or("ok"),
        scope: view.map(|view| view.scope),
        retention_days: retention,
        replay_allowed,
    }))
}

fn uncapped_summary(replay_allowed: bool, retention_days: i32) -> QuarantineSummary {
    QuarantineSummary {
        org_pending: 0,
        org_cap: None,
        contract_pending: 0,
        contract_cap: None,
        state: "ok",
        scope: None,
        retention_days,
        replay_allowed,
    }
}

/// `POST /contracts/{id}/quarantine/replay-bulk`
pub async fn replay_bulk_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<BulkReplayRequest>,
) -> AppResult<(StatusCode, Json<ReplayJobOut>)> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    let Some(org_id) = org_id else {
        return Err(AppError::BadRequest(
            "bulk replay needs an organization".into(),
        ));
    };
    plan::require_plan_feature(&state, Some(org_id), "bulk_replay").await?;
    storage::get_contract_identity(&state.db, contract_id, Some(org_id)).await?;
    let record_limit = i32::try_from(plan::bulk_replay_job_limit(
        std::env::var(plan::BULK_REPLAY_JOB_LIMIT_ENV)
            .ok()
            .as_deref(),
    ))
    .unwrap_or(i32::MAX);
    let target = blank(req.target_version);
    let version = blank(req.contract_version);
    let reason = blank(req.reason);
    let job = storage::insert_replay_job(
        &state.db,
        org_id,
        contract_id,
        target.as_deref(),
        version.as_deref(),
        reason.as_deref(),
        record_limit,
    )
    .await?;
    let job_id = job.id;
    let state_for_job = Arc::clone(&state);
    let target_for_job = target.clone();
    tokio::spawn(async move {
        if let Err(err) = run_bulk_job(
            state_for_job.clone(),
            job_id,
            org_id,
            contract_id,
            target_for_job,
            version,
            reason,
            i64::from(record_limit),
        )
        .await
        {
            tracing::warn!("bulk replay job {job_id} stopped: {err}");
            let _ = storage::update_replay_job(
                &state_for_job.db,
                job_id,
                job_progress("failed", 0, 0, 0, 0, Some(JOB_STOPPED)),
            )
            .await;
        }
    });
    Ok((StatusCode::ACCEPTED, Json(job_out(&job))))
}

/// `GET /contracts/{id}/quarantine/replay-jobs/{job_id}`
pub async fn replay_job_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, job_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<ReplayJobOut>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    let job = storage::get_replay_job(&state.db, org_id, contract_id, job_id)
        .await?
        .ok_or_else(|| AppError::NotFound("replay job not found".into()))?;
    Ok(Json(job_out(&job)))
}

fn job_out(job: &storage::ReplayJobRow) -> ReplayJobOut {
    ReplayJobOut {
        job_id: job.id,
        status: job.status.clone(),
        record_limit: job.record_limit,
        selected: job.selected,
        replayed: job.replayed,
        failed: job.failed,
        skipped: job.skipped,
        error: job.error.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_bulk_job(
    state: Arc<AppState>,
    job_id: Uuid,
    org_id: Uuid,
    contract_id: Uuid,
    target_version: Option<String>,
    filter_version: Option<String>,
    filter_reason: Option<String>,
    record_limit: i64,
) -> AppResult<()> {
    storage::update_replay_job(&state.db, job_id, job_progress("running", 0, 0, 0, 0, None))
        .await?;
    let ids = storage::select_bulk_replay_ids(
        &state.db,
        org_id,
        contract_id,
        filter_version.as_deref(),
        filter_reason.as_deref(),
        record_limit,
    )
    .await?;
    let selected = i32::try_from(ids.len()).unwrap_or(i32::MAX);
    if ids.is_empty() {
        storage::update_replay_job(
            &state.db,
            job_id,
            job_progress("completed", 0, 0, 0, 0, None),
        )
        .await?;
        return Ok(());
    }
    let mut replayed = 0i32;
    let mut failed = 0i32;
    let mut skipped = 0i32;
    for chunk in ids.chunks(MAX_REPLAY_IDS) {
        let response = match replay_for_contract(
            &state,
            Some(org_id),
            contract_id,
            chunk.to_vec(),
            target_version.clone(),
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!("bulk replay batch stopped: {err}");
                storage::update_replay_job(
                    &state.db,
                    job_id,
                    job_progress(
                        "failed",
                        selected,
                        replayed,
                        failed,
                        skipped,
                        Some(JOB_STOPPED),
                    ),
                )
                .await?;
                return Ok(());
            }
        };
        replayed = replayed.saturating_add(i32::try_from(response.replayed).unwrap_or(0));
        failed = failed.saturating_add(i32::try_from(response.still_quarantined).unwrap_or(0));
        let batch_skipped = response.already_replayed
            + response.not_found
            + response.wrong_contract
            + response.purged
            + response.redacted;
        skipped = skipped.saturating_add(i32::try_from(batch_skipped).unwrap_or(0));
        storage::update_replay_job(
            &state.db,
            job_id,
            job_progress("running", selected, replayed, failed, skipped, None),
        )
        .await?;
    }
    storage::update_replay_job(
        &state.db,
        job_id,
        job_progress("completed", selected, replayed, failed, skipped, None),
    )
    .await?;
    Ok(())
}

fn job_progress<'a>(
    status: &'a str,
    selected: i32,
    replayed: i32,
    failed: i32,
    skipped: i32,
    error: Option<&'a str>,
) -> storage::ReplayJobProgress<'a> {
    storage::ReplayJobProgress {
        status,
        selected,
        replayed,
        failed,
        skipped,
        error,
    }
}

fn blank(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn quarantine_csv(rows: &[QuarantineExportRow]) -> String {
    let mut out = String::from("quarantined_at,contract_version,reason,field,status\n");
    for row in rows {
        let (reason, field) = first_violation(&row.violation_details);
        let version = row.contract_version.as_deref().unwrap_or("");
        out.push_str(&csv_cell(&row.created_at.to_rfc3339()));
        out.push(',');
        out.push_str(&csv_cell(version));
        out.push(',');
        out.push_str(&csv_cell(reason));
        out.push(',');
        out.push_str(&csv_cell(field));
        out.push(',');
        out.push_str(&csv_cell(&row.status));
        out.push('\n');
    }
    out
}

fn first_violation(details: &serde_json::Value) -> (&str, &str) {
    let Some(first) = details.as_array().and_then(|items| items.first()) else {
        return ("", "");
    };
    let kind = first
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let field = first
        .get("field")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    (kind, field)
}

fn csv_cell(value: &str) -> String {
    let guarded = if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn csv_has_reasons_and_no_body() {
        let row = QuarantineExportRow {
            created_at: chrono::Utc
                .with_ymd_and_hms(2026, 10, 30, 12, 41, 0)
                .unwrap(),
            contract_version: Some("1.3.0".into()),
            status: "pending".into(),
            violation_details: serde_json::json!([
                {"kind": "type_mismatch", "field": "amount", "message": "secret-body-value"}
            ]),
        };
        let csv = quarantine_csv(&[row]);
        assert!(csv.starts_with("quarantined_at,contract_version,reason,field,status\n"));
        assert!(csv.contains("type_mismatch"));
        assert!(csv.contains("amount"));
        assert!(csv.contains("1.3.0"));
        assert!(!csv.contains("secret-body-value"));
        assert!(!csv.contains("payload"));
    }

    #[test]
    fn csv_escapes_formula_and_commas() {
        let cell = csv_cell("=cmd");
        assert!(cell.starts_with('\''));
        let quoted = csv_cell("a,b");
        assert_eq!(quoted, "\"a,b\"");
    }
}
