//! RFC-086 — event-payload storage settings + purge endpoints.
//!
//! Backs the dashboard's org master toggle, per-contract override, and the
//! per-contract "Purge body history" action. All routes are org-scoped: in
//! production a missing org → 401; ids belonging to another org 404.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::{storage, AppState, OrgId};

use crate::plan::gate_paid;

#[derive(Debug, Serialize)]
pub struct PayloadStorageStatus {
    /// The org-level master switch.
    pub enabled: bool,
    pub plan: String,
    /// Whether the plan is paid, so the switch is meaningful (else it is inert).
    pub eligible: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetEnabled {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct PurgeResult {
    pub quarantine_bodies_redacted: u64,
    pub audit_bodies_redacted: u64,
    /// `forwarded_events` rows whose payload was set to JSON null.
    pub forwarded_bodies_redacted: u64,
}

fn ensure_org(state: &AppState, org_id: Option<Uuid>) -> AppResult<()> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

/// `GET /settings/payload-storage` — the org master switch + plan eligibility.
pub async fn get_payload_storage_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
) -> AppResult<Json<PayloadStorageStatus>> {
    ensure_org(&state, org_id)?;
    // Self-host / dev (no org): bodies are always stored, not plan-gated.
    let Some(oid) = org_id else {
        return Ok(Json(PayloadStorageStatus {
            enabled: true,
            plan: "self_host".into(),
            eligible: true,
        }));
    };
    let (plan, enabled) = storage::get_org_payload_policy(&state.db, oid)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let eligible = gate_paid(state.self_hosted, &plan);
    Ok(Json(PayloadStorageStatus {
        enabled,
        plan,
        eligible,
    }))
}

/// `PUT /settings/payload-storage` `{enabled}` — flip the org master switch.
/// Enabling requires a paid plan. Disabling purges all stored bodies org-wide
/// (RFC-086) — the org is choosing to hold no source data.
pub async fn set_payload_storage_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Json(body): Json<SetEnabled>,
) -> AppResult<Json<PayloadStorageStatus>> {
    ensure_org(&state, org_id)?;
    let Some(oid) = org_id else {
        return Err(AppError::BadRequest(
            "payload storage settings require an org context".into(),
        ));
    };
    let (plan, _current) = storage::get_org_payload_policy(&state.db, oid)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let eligible = gate_paid(state.self_hosted, &plan);
    if body.enabled && !eligible {
        return Err(AppError::BadRequest(
            crate::plan::payload_storage_upgrade_message(),
        ));
    }

    storage::set_org_store_payloads(&state.db, oid, body.enabled).await?;
    // Off → purge every stored body for the org, including forwarded_events.
    if !body.enabled {
        storage::purge_bodies(&state.db, Some(oid), None).await?;
    }

    Ok(Json(PayloadStorageStatus {
        enabled: body.enabled,
        plan,
        eligible,
    }))
}

/// `PATCH /contracts/{id}/payload-storage` `{enabled}` — per-contract override.
/// Only effective when the org master switch is on. Write-forward only: this
/// does **not** purge audit, quarantine, or `forwarded_events` history (use
/// the purge endpoint for that).
pub async fn set_contract_payload_storage_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(body): Json<SetEnabled>,
) -> AppResult<Json<SetEnabled>> {
    ensure_org(&state, org_id)?;
    let updated =
        storage::set_contract_store_payloads(&state.db, contract_id, org_id, body.enabled).await?;
    if !updated {
        return Err(AppError::ContractNotFound(contract_id));
    }
    Ok(Json(body))
}

/// `POST /contracts/{id}/purge-bodies` — redact stored bodies for one contract,
/// retaining every audit, quarantine, and forwarded row and its metadata.
/// Independent of the toggles. Never deletes a row.
pub async fn purge_contract_bodies_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<PurgeResult>> {
    ensure_org(&state, org_id)?;
    // 404 if the contract isn't owned by / visible to this org.
    storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let (q, a, f) = storage::purge_bodies(&state.db, org_id, Some(contract_id)).await?;
    Ok(Json(PurgeResult {
        quarantine_bodies_redacted: q,
        audit_bodies_redacted: a,
        forwarded_bodies_redacted: f,
    }))
}

#[cfg(test)]
mod tests {
    use super::PurgeResult;

    #[test]
    fn purge_result_includes_forwarded_count() {
        let body = serde_json::to_value(PurgeResult {
            quarantine_bodies_redacted: 12,
            audit_bodies_redacted: 480,
            forwarded_bodies_redacted: 90,
        })
        .unwrap();
        assert_eq!(body["quarantine_bodies_redacted"], 12);
        assert_eq!(body["audit_bodies_redacted"], 480);
        assert_eq!(body["forwarded_bodies_redacted"], 90);
    }
}
