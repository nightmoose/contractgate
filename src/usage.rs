//! RFC-083 Phase 1 — `GET /usage`.
//!
//! Org-scoped current-month event usage against the org's plan limit. Read-only,
//! live count over `audit_log` — no counter table and no hot-path change (that
//! arrives with Phase 2 enforcement).

use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::plan::monthly_event_limit;
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
    let used = storage::monthly_event_count(&state.db, org_id, period_start).await?;

    let plan = match org_id {
        Some(o) => storage::get_org_plan(&state.db, o)
            .await?
            .unwrap_or_else(|| "free".to_string()),
        None => "free".to_string(),
    };

    let limit = monthly_event_limit(&plan);
    let unlimited = limit.is_none();
    let remaining = limit.map(|l| (l - used).max(0));
    let pct = limit.map(|l| {
        if l > 0 {
            used as f64 / l as f64 * 100.0
        } else {
            0.0
        }
    });

    Ok(Json(UsageResponse {
        plan,
        period_start,
        used,
        limit,
        remaining,
        pct,
        unlimited,
    }))
}

/// First instant of the current UTC calendar month.
fn current_month_start() -> chrono::DateTime<chrono::Utc> {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("first of month at midnight is always a valid, unambiguous UTC instant")
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
