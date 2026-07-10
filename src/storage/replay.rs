//! Quarantine replay history + batch replay-marking.
//!
//! Split out of the original monolithic `storage.rs` (2026-07-10, RFC/worklist
//! item 3).

use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;


// ---------------------------------------------------------------------------
// Replay (RFC-003) — manual Replay Quarantine
// ---------------------------------------------------------------------------

/// A quarantine row as loaded by the replay handler.  Carries just enough to
/// categorize the row (not_found / wrong_contract / purged / already_replayed
/// / eligible) and re-validate the payload under a target version.
///
/// `contract_version`, `replayed_into_audit_id`, and `created_at` are read
/// back for future use (dashboard drawer, audit export) even though the
/// current replay handler doesn't dispatch on them.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct QuarantineRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub replayed_into_audit_id: Option<Uuid>,
    pub source_ip: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct QuarantineRowRaw {
    id: Uuid,
    contract_id: Uuid,
    contract_version: String,
    payload: serde_json::Value,
    status: String,
    replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    replayed_into_audit_id: Option<Uuid>,
    source_ip: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<QuarantineRowRaw> for QuarantineRow {
    fn from(r: QuarantineRowRaw) -> Self {
        Self {
            id: r.id,
            contract_id: r.contract_id,
            contract_version: r.contract_version,
            payload: r.payload,
            status: r.status,
            replayed_at: r.replayed_at,
            replayed_into_audit_id: r.replayed_into_audit_id,
            source_ip: r.source_ip,
            created_at: r.created_at,
        }
    }
}

/// Load quarantine rows by ID.  Rows missing from the result set are
/// surfaced by the caller as `not_found` — this helper simply returns the
/// subset that exists.
///
/// Preserves no particular order; the handler re-keys by ID.
pub async fn list_quarantine_by_ids(pool: &PgPool, ids: &[Uuid]) -> AppResult<Vec<QuarantineRow>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<QuarantineRowRaw> = sqlx::query_as(
        r#"
        SELECT
            id, contract_id, contract_version, payload, status,
            replayed_at, replayed_into_audit_id, source_ip, created_at
        FROM quarantine_events
        WHERE id = ANY($1::uuid[])
        "#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(QuarantineRow::from).collect())
}

/// Mark a batch of quarantine rows as replayed, each linked to the specific
/// audit_log row its payload produced on success.
///
/// The UPDATE is **conditional**: it only stamps rows whose `replayed_at` is
/// still NULL and whose status is `pending` or `reviewed`.  This is the race
/// guard — if two concurrent replay calls target the same source row, at
/// most one UPDATE touches it; the other sees the stamp on a re-read and
/// surfaces `already_replayed` to its caller.
///
/// Returns the set of source IDs that were successfully marked.  Any IDs in
/// `pairs` missing from the returned vec lost the race and should be
/// re-categorized by the caller.
pub async fn mark_quarantine_replayed_batch(
    pool: &PgPool,
    pairs: &[(Uuid, Uuid)], // (source_quarantine_id, new_audit_id)
    replayed_at: chrono::DateTime<chrono::Utc>,
) -> AppResult<Vec<Uuid>> {
    if pairs.is_empty() {
        return Ok(vec![]);
    }

    let source_ids: Vec<Uuid> = pairs.iter().map(|(s, _)| *s).collect();
    let new_audit_ids: Vec<Uuid> = pairs.iter().map(|(_, a)| *a).collect();

    let updated: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE quarantine_events qe
        SET status = 'replayed',
            replayed_at = $3,
            replayed_into_audit_id = t.new_audit_id
        FROM UNNEST($1::uuid[], $2::uuid[]) AS t(source_id, new_audit_id)
        WHERE qe.id = t.source_id
          AND qe.status IN ('pending', 'reviewed')
          AND qe.replayed_at IS NULL
        RETURNING qe.id
        "#,
    )
    .bind(&source_ids)
    .bind(&new_audit_ids)
    .bind(replayed_at)
    .fetch_all(pool)
    .await?;

    Ok(updated.into_iter().map(|(id,)| id).collect())
}

/// One entry in the replay-history chain returned by
/// `GET /contracts/:id/quarantine/:quar_id/replay-history`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayHistoryEntry {
    /// The original quarantine row the replay chain starts from.
    Source {
        id: Uuid,
        contract_version: String,
        status: String,
        violation_count: i32,
        replayed_at: Option<chrono::DateTime<chrono::Utc>>,
        replayed_into_audit_id: Option<Uuid>,
        created_at: chrono::DateTime<chrono::Utc>,
    },
    /// A quarantine row created by a failed replay attempt.
    FailedReplay {
        id: Uuid,
        contract_version: String,
        violation_count: i32,
        created_at: chrono::DateTime<chrono::Utc>,
    },
    /// The audit_log row a successful replay attempt produced.
    PassedReplay {
        id: Uuid,
        contract_version: String,
        created_at: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(sqlx::FromRow)]
struct SourceHistoryRow {
    id: Uuid,
    contract_version: String,
    status: String,
    violation_count: i32,
    replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    replayed_into_audit_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct FailedReplayRow {
    id: Uuid,
    contract_version: String,
    violation_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct PassedReplayRow {
    id: Uuid,
    contract_version: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Return the full replay history for a given quarantine row: the source row,
/// every failed-replay child linking back to it, and the terminal audit_log
/// row if a replay eventually passed.
///
/// The caller is expected to have already verified that `source_id` belongs
/// to `contract_id` (or to return 404 if not).
pub async fn replay_history_for(
    pool: &PgPool,
    contract_id: Uuid,
    source_id: Uuid,
) -> AppResult<Vec<ReplayHistoryEntry>> {
    let source: Option<SourceHistoryRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, status, violation_count, replayed_at,
               replayed_into_audit_id, created_at
        FROM quarantine_events
        WHERE id = $1 AND contract_id = $2
        "#,
    )
    .bind(source_id)
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;

    let source = match source {
        Some(r) => r,
        None => {
            return Err(AppError::BadRequest(format!(
                "quarantine row {source_id} not found on contract {contract_id}"
            )));
        }
    };

    // Failed-replay children.
    let failed: Vec<FailedReplayRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, violation_count, created_at
        FROM quarantine_events
        WHERE replay_of_quarantine_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    // Passed-replay audit row (if any).
    let passed: Vec<PassedReplayRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, created_at
        FROM audit_log
        WHERE replay_of_quarantine_id = $1 AND passed = true
        ORDER BY created_at ASC
        "#,
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    let mut out: Vec<ReplayHistoryEntry> = Vec::with_capacity(1 + failed.len() + passed.len());
    out.push(ReplayHistoryEntry::Source {
        id: source.id,
        contract_version: source.contract_version,
        status: source.status,
        violation_count: source.violation_count,
        replayed_at: source.replayed_at,
        replayed_into_audit_id: source.replayed_into_audit_id,
        created_at: source.created_at,
    });
    for r in failed {
        out.push(ReplayHistoryEntry::FailedReplay {
            id: r.id,
            contract_version: r.contract_version,
            violation_count: r.violation_count,
            created_at: r.created_at,
        });
    }
    for r in passed {
        out.push(ReplayHistoryEntry::PassedReplay {
            id: r.id,
            contract_version: r.contract_version,
            created_at: r.created_at,
        });
    }
    Ok(out)
}
