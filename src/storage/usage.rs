//! RFC-083 — usage metering (Phase 1 live counts + Phase 2 cached counter).

use crate::error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

/// The org's plan string (`free`/`growth`/`enterprise`), or `None` if the org
/// row is missing (dev / self-hosted).
pub async fn get_org_plan(pool: &PgPool, org_id: Uuid) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT plan FROM orgs WHERE id = $1")
        .bind(org_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(p,)| p))
}

/// Count of `audit_log` events for an org since `since` (inclusive). When
/// `org_id` is `None` (dev-no-auth) counts across all orgs. Backed by
/// `audit_log_org_id_created_idx` (migration 007).
///
/// Still used for bootstrap of the Phase 2 counter and as a diagnostic
/// fallback — hot-path enforcement uses [`get_monthly_usage_events`].
pub async fn monthly_event_count(
    pool: &PgPool,
    org_id: Option<Uuid>,
    since: chrono::DateTime<chrono::Utc>,
) -> AppResult<i64> {
    let (count,): (i64,) = match org_id {
        Some(o) => {
            sqlx::query_as("SELECT count(*) FROM audit_log WHERE org_id = $1 AND created_at >= $2")
                .bind(o)
                .bind(since)
                .fetch_one(pool)
                .await?
        }
        None => {
            sqlx::query_as("SELECT count(*) FROM audit_log WHERE created_at >= $1")
                .bind(since)
                .fetch_one(pool)
                .await?
        }
    };
    Ok(count)
}

/// O(1) read of the Phase 2 counter for `(org_id, period)`.
/// Returns `None` if no row exists yet (caller may bootstrap).
pub async fn get_monthly_usage_events(
    pool: &PgPool,
    org_id: Uuid,
    period: &str,
) -> AppResult<Option<i64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT events FROM org_monthly_usage WHERE org_id = $1 AND period = $2")
            .bind(org_id)
            .bind(period)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(e,)| e))
}

/// One-round-trip read of the org's plan **and** current-period counter, for
/// the hot-path enforcement check (RFC-083 Phase 2). `plan` is `None` when the
/// org row is missing; `events` is `None` when no counter row exists yet for
/// this period (caller bootstraps). Collapses the previous two reads (plan +
/// counter) into a single indexed query.
pub async fn get_org_plan_and_usage(
    pool: &PgPool,
    org_id: Uuid,
    period: &str,
) -> AppResult<(Option<String>, Option<i64>)> {
    let row: Option<(String, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT o.plan, u.events
        FROM public.orgs o
        LEFT JOIN public.org_monthly_usage u
          ON u.org_id = o.id AND u.period = $2
        WHERE o.id = $1
        "#,
    )
    .bind(org_id)
    .bind(period)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some((plan, events)) => (Some(plan), events),
        None => (None, None),
    })
}

/// Ensure a counter row exists for this org/period. If missing, seed from a
/// live `audit_log` count since `period_start` (one-time cost per org/month).
/// Returns the current event count.
pub async fn ensure_monthly_usage(
    pool: &PgPool,
    org_id: Uuid,
    period: &str,
    period_start: chrono::DateTime<chrono::Utc>,
) -> AppResult<i64> {
    if let Some(n) = get_monthly_usage_events(pool, org_id, period).await? {
        return Ok(n);
    }
    let seed = monthly_event_count(pool, Some(org_id), period_start).await?;
    // Race-safe insert: if another request seeds first, keep the larger of the two.
    let (events,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO org_monthly_usage (org_id, period, events)
        VALUES ($1, $2, $3)
        ON CONFLICT (org_id, period) DO UPDATE
          SET events = GREATEST(org_monthly_usage.events, EXCLUDED.events),
              updated_at = now()
        RETURNING events
        "#,
    )
    .bind(org_id)
    .bind(period)
    .bind(seed)
    .fetch_one(pool)
    .await?;
    Ok(events)
}

/// Atomically add `delta` events to the org's monthly counter. Creates the row
/// if missing. Returns the new total.
pub async fn increment_monthly_usage(
    pool: &PgPool,
    org_id: Uuid,
    period: &str,
    delta: i64,
) -> AppResult<i64> {
    if delta <= 0 {
        return get_monthly_usage_events(pool, org_id, period)
            .await
            .map(|o| o.unwrap_or(0));
    }
    let (events,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO org_monthly_usage (org_id, period, events)
        VALUES ($1, $2, $3)
        ON CONFLICT (org_id, period) DO UPDATE
          SET events = org_monthly_usage.events + EXCLUDED.events,
              updated_at = now()
        RETURNING events
        "#,
    )
    .bind(org_id)
    .bind(period)
    .bind(delta)
    .fetch_one(pool)
    .await?;
    Ok(events)
}
