//! Pending quarantine counts, review, CSV rows, and bulk-replay jobs.
//!
//! Pending means `status = 'pending'`. Reviewed, replayed, and purged rows
//! do not count, so marking a row reviewed frees cap space on the next read.
//! Ingest and egress share the org count. Direction is not filtered.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{AppResult, DbOpContext};

pub const PENDING_QUARANTINE_COUNTS_SQL: &str = r#"
SELECT
    (SELECT count(*)::bigint FROM quarantine_events
      WHERE org_id = $1
        AND status = 'pending'
        AND replay_of_quarantine_id IS NULL) AS org_pending,
    (SELECT count(*)::bigint FROM quarantine_events
      WHERE contract_id = $2
        AND status = 'pending'
        AND replay_of_quarantine_id IS NULL) AS contract_pending
"#;

pub async fn pending_quarantine_counts(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
) -> AppResult<(i64, i64)> {
    let row: (i64, i64) = sqlx::query_as(PENDING_QUARANTINE_COUNTS_SQL)
        .bind(org_id)
        .bind(contract_id)
        .fetch_one(pool)
        .await
        .db_op("pending_quarantine_counts")?;
    Ok(row)
}

pub async fn org_pending_quarantine_count(pool: &PgPool, org_id: Uuid) -> AppResult<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*)::bigint FROM quarantine_events
        WHERE org_id = $1
          AND status = 'pending'
          AND replay_of_quarantine_id IS NULL
        "#,
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .db_op("org_pending_quarantine_count")?;
    Ok(count)
}

pub async fn mark_quarantine_reviewed(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    ids: &[Uuid],
) -> AppResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query(
        r#"
        UPDATE quarantine_events qe
        SET status = 'reviewed',
            reviewed_at = now()
        FROM contracts c
        WHERE qe.contract_id = c.id
          AND qe.contract_id = $1
          AND ($2::uuid IS NULL OR c.org_id = $2)
          AND qe.id = ANY($3)
          AND qe.status = 'pending'
          AND c.deleted_at IS NULL
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(ids)
    .execute(pool)
    .await
    .db_op("mark_quarantine_reviewed")?;
    Ok(result.rows_affected())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuarantineExportRow {
    pub created_at: DateTime<Utc>,
    pub contract_version: Option<String>,
    pub status: String,
    pub violation_details: serde_json::Value,
}

pub async fn list_quarantine_export(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    limit: i64,
) -> AppResult<Vec<QuarantineExportRow>> {
    let rows = sqlx::query_as(
        r#"
        SELECT qe.created_at, qe.contract_version, qe.status, qe.violation_details
        FROM quarantine_events qe
        JOIN contracts c ON c.id = qe.contract_id
        WHERE qe.contract_id = $1
          AND ($2::uuid IS NULL OR c.org_id = $2)
          AND qe.replay_of_quarantine_id IS NULL
          AND c.deleted_at IS NULL
        ORDER BY qe.created_at DESC
        LIMIT $3
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .db_op("list_quarantine_export")?;
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct ReplayJobRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub contract_id: Uuid,
    pub target_version: Option<String>,
    pub filter_version: Option<String>,
    pub filter_reason: Option<String>,
    pub status: String,
    pub record_limit: i32,
    pub selected: i32,
    pub replayed: i32,
    pub failed: i32,
    pub skipped: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn insert_replay_job(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    target_version: Option<&str>,
    filter_version: Option<&str>,
    filter_reason: Option<&str>,
    record_limit: i32,
) -> AppResult<ReplayJobRow> {
    let row = sqlx::query_as(
        r#"
        INSERT INTO quarantine_replay_jobs (
            org_id, contract_id, target_version, filter_version, filter_reason, record_limit
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, org_id, contract_id, target_version, filter_version, filter_reason,
                  status, record_limit, selected, replayed, failed, skipped, error,
                  created_at, updated_at
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(target_version)
    .bind(filter_version)
    .bind(filter_reason)
    .bind(record_limit)
    .fetch_one(pool)
    .await
    .db_op("insert_replay_job")?;
    Ok(row)
}

pub async fn get_replay_job(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    job_id: Uuid,
) -> AppResult<Option<ReplayJobRow>> {
    let row = sqlx::query_as(
        r#"
        SELECT id, org_id, contract_id, target_version, filter_version, filter_reason,
               status, record_limit, selected, replayed, failed, skipped, error,
               created_at, updated_at
        FROM quarantine_replay_jobs
        WHERE id = $1
          AND contract_id = $2
          AND ($3::uuid IS NULL OR org_id = $3)
        "#,
    )
    .bind(job_id)
    .bind(contract_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("get_replay_job")?;
    Ok(row)
}

pub struct ReplayJobProgress<'a> {
    pub status: &'a str,
    pub selected: i32,
    pub replayed: i32,
    pub failed: i32,
    pub skipped: i32,
    pub error: Option<&'a str>,
}

pub async fn update_replay_job(
    pool: &PgPool,
    job_id: Uuid,
    progress: ReplayJobProgress<'_>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        UPDATE quarantine_replay_jobs
        SET status = $2,
            selected = $3,
            replayed = $4,
            failed = $5,
            skipped = $6,
            error = $7,
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .bind(progress.status)
    .bind(progress.selected)
    .bind(progress.replayed)
    .bind(progress.failed)
    .bind(progress.skipped)
    .bind(progress.error)
    .execute(pool)
    .await
    .db_op("update_replay_job")?;
    Ok(())
}

pub async fn select_bulk_replay_ids(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    filter_version: Option<&str>,
    filter_reason: Option<&str>,
    limit: i64,
) -> AppResult<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT qe.id
        FROM quarantine_events qe
        JOIN contracts c ON c.id = qe.contract_id
        WHERE qe.contract_id = $1
          AND c.org_id = $2
          AND c.deleted_at IS NULL
          AND qe.replay_of_quarantine_id IS NULL
          AND qe.status IN ('pending', 'reviewed')
          AND qe.payload_redacted = false
          AND qe.payload IS NOT NULL
          AND ($3::text IS NULL OR qe.contract_version = $3)
          AND (
              $4::text IS NULL
              OR EXISTS (
                  SELECT 1
                  FROM jsonb_array_elements(
                      CASE
                          WHEN jsonb_typeof(qe.violation_details) = 'array'
                          THEN qe.violation_details
                          ELSE '[]'::jsonb
                      END
                  ) AS v
                  WHERE v->>'kind' = $4
              )
          )
        ORDER BY qe.created_at ASC
        LIMIT $5
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(filter_version)
    .bind(filter_reason)
    .bind(limit)
    .fetch_all(pool)
    .await
    .db_op("select_bulk_replay_ids")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
