//! RFC-083 Phase 2 — plan-limit enforcement helpers.
//!
//! Hot-path design:
//! - **Check** is a single primary-key read (after optional one-time bootstrap).
//! - **Increment** is one UPSERT per batch, fired after audit write is scheduled
//!   (same fire-and-forget pattern as audit), so it does not block the response
//!   path beyond the pre-check.
//! - Self-hosted / no-org traffic is unmetered.
//! - Enterprise (`limit = None`) is never blocked.

use crate::error::AppError;
use crate::plan::monthly_event_limit;
use crate::storage;
use sqlx::PgPool;
use uuid::Uuid;

/// UTC calendar month key: `YYYY-MM`.
pub fn current_period_key() -> String {
    use chrono::{Datelike, Utc};
    let now = Utc::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

/// First instant of the current UTC calendar month.
pub fn current_month_start() -> chrono::DateTime<chrono::Utc> {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("first of month at midnight is always a valid UTC instant")
}

/// Pure check: if `used >= limit`, the org is already at/over cap (next batch blocked).
/// A batch that *crosses* the cap while `used < limit` is still allowed once.
#[inline]
pub fn is_at_or_over_cap(used: i64, limit: Option<i64>) -> bool {
    match limit {
        None => false,
        Some(l) => used >= l,
    }
}

/// Load plan + current usage and return `Err(PlanLimitExceeded)` when blocked.
/// No-op (Ok) when `org_id` is `None` (dev / self-host) or plan is unlimited.
pub async fn enforce_plan_limit(pool: &PgPool, org_id: Option<Uuid>) -> Result<(), AppError> {
    let Some(org_id) = org_id else {
        return Ok(());
    };

    let plan = storage::get_org_plan(pool, org_id)
        .await?
        .unwrap_or_else(|| "free".to_string());
    let limit = monthly_event_limit(&plan);
    if limit.is_none() {
        return Ok(());
    }

    let period = current_period_key();
    let period_start = current_month_start();
    let used = storage::ensure_monthly_usage(pool, org_id, &period, period_start).await?;

    if is_at_or_over_cap(used, limit) {
        return Err(AppError::PlanLimitExceeded {
            plan,
            limit: limit.unwrap_or(0),
            used,
            period,
        });
    }
    Ok(())
}

/// Schedule a fire-and-forget counter increment for this batch.
/// Safe to call when `org_id` is None (no-op).
pub fn record_batch_usage(pool: PgPool, org_id: Option<Uuid>, event_count: usize) {
    let Some(org_id) = org_id else {
        return;
    };
    if event_count == 0 {
        return;
    }
    let period = current_period_key();
    let delta = event_count as i64;
    tokio::spawn(async move {
        if let Err(e) = storage::increment_monthly_usage(&pool, org_id, &period, delta).await {
            tracing::warn!(
                org_id = %org_id,
                period = %period,
                delta,
                "RFC-083: failed to increment org_monthly_usage: {e:?}"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_blocked() {
        assert!(!is_at_or_over_cap(0, None));
        assert!(!is_at_or_over_cap(i64::MAX, None));
    }

    #[test]
    fn under_cap_allows() {
        assert!(!is_at_or_over_cap(999_999, Some(1_000_000)));
        assert!(!is_at_or_over_cap(0, Some(1_000_000)));
    }

    #[test]
    fn at_or_over_blocks() {
        assert!(is_at_or_over_cap(1_000_000, Some(1_000_000)));
        assert!(is_at_or_over_cap(1_000_001, Some(1_000_000)));
    }

    #[test]
    fn crossing_batch_still_allowed_if_under() {
        // used=999_990, batch of 20 would cross 1M — still allowed once
        // because check is `used >= limit` before the batch, not after.
        assert!(!is_at_or_over_cap(999_990, Some(1_000_000)));
    }

    #[test]
    fn period_key_format() {
        let k = current_period_key();
        assert_eq!(k.len(), 7);
        assert_eq!(&k[4..5], "-");
        let (y, m) = (&k[..4], &k[5..]);
        assert!(y.parse::<u16>().is_ok());
        let month: u8 = m.parse().unwrap();
        assert!((1..=12).contains(&month));
    }
}
