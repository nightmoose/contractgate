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
///
/// **Fails open on infrastructure errors.** Metering must never take down the
/// ingest hot path: if the plan lookup or the counter read errors (DB blip, or
/// `org_monthly_usage` not yet migrated), we log and *allow* the request rather
/// than 500. The only error this returns is the intentional `PlanLimitExceeded`.
/// This also de-risks deploy ordering — shipping the binary before migration 032
/// simply means "not yet enforcing", not "every ingest 500s".
pub async fn enforce_plan_limit(pool: &PgPool, org_id: Option<Uuid>) -> Result<(), AppError> {
    let Some(org_id) = org_id else {
        return Ok(());
    };
    let period = current_period_key();

    // One round-trip: plan + current-period counter (steady state is a single
    // indexed read; matches the Enterprise fast path).
    let (plan_opt, events_opt) = match storage::get_org_plan_and_usage(pool, org_id, &period).await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(org_id = %org_id, "RFC-083: plan/usage lookup failed, allowing ingest (fail-open): {e:?}");
            return Ok(());
        }
    };

    // Unknown org (no row) → unmetered; a missing org can't own a counter anyway.
    let Some(plan) = plan_opt else {
        return Ok(());
    };
    let Some(limit) = monthly_event_limit(&plan) else {
        return Ok(()); // Enterprise / unlimited
    };

    // Fast path: counter row present → no extra read. Otherwise bootstrap once
    // this month from audit_log.
    let used = match events_opt {
        Some(n) => n,
        None => {
            let period_start = current_month_start();
            match storage::ensure_monthly_usage(pool, org_id, &period, period_start).await {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!(org_id = %org_id, period = %period, "RFC-083: usage bootstrap failed, allowing ingest (fail-open): {e:?}");
                    return Ok(());
                }
            }
        }
    };

    if is_at_or_over_cap(used, Some(limit)) {
        return Err(AppError::PlanLimitExceeded {
            plan,
            limit,
            used,
            period,
        });
    }
    Ok(())
}

/// Schedule a fire-and-forget counter increment for this batch (own spawn).
/// Prefer [`record_batch_usage_blocking`] when already inside the audit spawn
/// so audit + meter stay ordered together.
/// Safe to call when `org_id` is None (no-op).
///
/// Kept for paths without an existing spawn (e.g. future stream metering).
/// HTTP ingest uses the blocking form inside the audit task.
#[allow(dead_code)]
pub fn record_batch_usage(pool: PgPool, org_id: Option<Uuid>, event_count: usize) {
    let Some(org_id) = org_id else {
        return;
    };
    if event_count == 0 {
        return;
    }
    tokio::spawn(async move {
        record_batch_usage_blocking(&pool, Some(org_id), event_count).await;
    });
}

/// Awaited increment (call from an existing spawn, e.g. after audit succeeds).
pub async fn record_batch_usage_blocking(pool: &PgPool, org_id: Option<Uuid>, event_count: usize) {
    let Some(org_id) = org_id else {
        return;
    };
    if event_count == 0 {
        return;
    }
    let period = current_period_key();
    let delta = event_count as i64;
    if let Err(e) = storage::increment_monthly_usage(pool, org_id, &period, delta).await {
        tracing::warn!(
            org_id = %org_id,
            period = %period,
            delta,
            "RFC-083: failed to increment org_monthly_usage: {e:?}"
        );
    }
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
