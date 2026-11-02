//! RFC-083 — `GET /usage`.
//!
//! Org-scoped current-month event usage against the org's plan limit.
//! Phase 2 prefers the O(1) `org_monthly_usage` counter (bootstrapped from
//! audit_log once per org/month); falls back cleanly for unlimited plans.

use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::metering::{current_month_start, current_period_key};
use crate::plan::monthly_event_limit_for;
use crate::storage;
use crate::{AppState, OrgId};

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub plan: String,
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub used: i64,
    /// Monthly cap; `null` for unlimited (Enterprise).
    pub limit: Option<i64>,
    /// `max(limit - used, 0)`; `null` when unlimited.
    pub remaining: Option<i64>,
    /// Percent of the cap used; `null` when unlimited.
    pub pct: Option<f64>,
    pub unlimited: bool,
    /// Pending quarantine rows for this org. Ingest and egress share the count.
    pub quarantine: QuarantineUsage,
}

#[derive(Debug, Serialize)]
pub struct QuarantineUsage {
    pub pending: i64,
    /// Org cap. `null` when the plan has no cap.
    pub cap: Option<i64>,
    /// Per-contract cap. `null` when the plan has no cap.
    pub contract_cap: Option<i64>,
    /// `ok`, `near`, or `reached` against the org cap.
    pub state: &'static str,
}

/// `GET /usage`
pub async fn usage_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
) -> AppResult<Json<UsageResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }

    let period_start = current_month_start();
    let period = current_period_key();

    let used = match org_id {
        Some(o) => storage::ensure_monthly_usage(&state.db, o, &period, period_start).await?,
        // Dev-no-auth: live count across all orgs (unchanged Phase 1 behaviour).
        None => storage::monthly_event_count(&state.db, None, period_start).await?,
    };

    let plan = match org_id {
        Some(o) => storage::get_org_plan(&state.db, o)
            .await?
            .unwrap_or_else(|| "free".to_string()),
        None => "free".to_string(),
    };

    let limit = monthly_event_limit_for(state.self_hosted, &plan);
    let unlimited = limit.is_none();
    let remaining = limit.map(|l| (l - used).max(0));
    let pct = limit.map(|l| {
        if l > 0 {
            used as f64 / l as f64 * 100.0
        } else {
            0.0
        }
    });

    let quarantine = quarantine_usage(&state, org_id, &plan).await;

    Ok(Json(UsageResponse {
        plan,
        period_start,
        used,
        limit,
        remaining,
        pct,
        unlimited,
        quarantine,
    }))
}

async fn quarantine_usage(
    state: &AppState,
    org_id: Option<uuid::Uuid>,
    plan: &str,
) -> QuarantineUsage {
    let cap = crate::plan::quarantine_row_cap_for(state.self_hosted, plan);
    let contract_cap = crate::plan::per_contract_quarantine_cap_for(state.self_hosted, plan);
    let pending = match org_id {
        Some(org_id) => match storage::org_pending_quarantine_count(&state.db, org_id).await {
            Ok(count) => count,
            Err(err) => {
                tracing::warn!("usage quarantine count failed: {err}");
                0
            }
        },
        None => 0,
    };
    let state_name = match cap {
        Some(limit) if limit > 0 && pending >= limit => "reached",
        Some(limit)
            if limit > 0
                && pending.saturating_mul(100)
                    >= limit.saturating_mul(crate::plan::QUARANTINE_CAP_NEAR_PERCENT) =>
        {
            "near"
        }
        _ => "ok",
    };
    QuarantineUsage {
        pending,
        cap,
        contract_cap,
        state: state_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_wire_shape() {
        let resp = UsageResponse {
            plan: "free".into(),
            period_start: current_month_start(),
            used: 100,
            limit: Some(1_000_000),
            remaining: Some(999_900),
            pct: Some(0.01),
            unlimited: false,
            quarantine: QuarantineUsage {
                pending: 0,
                cap: Some(50_000),
                contract_cap: Some(10_000),
                state: "ok",
            },
        };
        let v = serde_json::to_value(&resp).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "plan",
            "period_start",
            "used",
            "limit",
            "remaining",
            "pct",
            "unlimited",
            "quarantine",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn month_start_is_first_at_midnight() {
        use chrono::{Datelike, Timelike};
        let s = current_month_start();
        assert_eq!(s.day(), 1);
        assert_eq!((s.hour(), s.minute(), s.second()), (0, 0, 0));
    }
}
