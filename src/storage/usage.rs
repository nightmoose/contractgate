//! RFC-083 — usage metering reads (Phase 1: live counts, no counter table yet).

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
