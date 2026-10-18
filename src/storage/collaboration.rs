//! Contract collaboration: collaborator roles, comments, edit proposals
//! (RFC-033).
//!
//! Split out of the original monolithic `storage.rs` (2026-07-10, RFC/worklist
//! item 3).

use crate::error::{AppError, AppResult, DbOpContext};
use sqlx::PgPool;
use uuid::Uuid;

// =============================================================================
// RFC-033 — Provider-Consumer Collaboration storage
// =============================================================================

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A collaborator grant row from `contract_collaborators`.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct CollaboratorRow {
    pub contract_name: String,
    pub org_id: Uuid,
    pub role: String,
    pub granted_by: Uuid,
    pub granted_at: chrono::DateTime<chrono::Utc>,
}

/// A comment row from `contract_comments`.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct CommentRow {
    pub id: Uuid,
    pub contract_name: String,
    pub field: Option<String>,
    pub org_id: Uuid,
    pub author: String,
    pub body: String,
    pub resolved: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A change proposal row from `contract_change_proposals`.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ProposalRow {
    pub id: Uuid,
    pub contract_name: String,
    pub proposed_by: Uuid,
    pub proposed_yaml: String,
    pub status: String,
    pub decided_by: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Name → contract id resolution (migration 042: everything keys on id)
// ---------------------------------------------------------------------------

/// Ids of the caller's own live contracts named `contract_name`.
pub async fn owned_contract_ids_by_name(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
) -> AppResult<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id FROM contracts
        WHERE  name = $1 AND org_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .fetch_all(pool)
    .await
    .db_op("owned_contract_ids_by_name")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Live contracts named `contract_name` on which `org_id` holds a grant,
/// with the granted role.
pub async fn granted_contracts_by_name(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
) -> AppResult<Vec<(Uuid, String)>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        r#"
        SELECT c.id, cc.role
        FROM   contract_collaborators cc
        JOIN   contracts c ON c.id = cc.contract_id
        WHERE  c.name = $1 AND c.deleted_at IS NULL AND cc.org_id = $2
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .fetch_all(pool)
    .await
    .db_op("granted_contracts_by_name")?;
    Ok(rows)
}

/// True if any org has a live contract named `contract_name`.  Used only to
/// choose between 403 (exists, caller lacks the role) and 404.
pub async fn contract_name_exists(pool: &PgPool, contract_name: &str) -> AppResult<bool> {
    let row: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM contracts WHERE name = $1 AND deleted_at IS NULL LIMIT 1")
            .bind(contract_name)
            .fetch_optional(pool)
            .await
            .db_op("contract_name_exists")?;
    Ok(row.is_some())
}

/// Active grant of `caller_org` on the live contract `contract_id`:
/// `(owner_org, role)`.  `None` = no grant.
pub async fn collaborator_grant_by_id(
    pool: &PgPool,
    contract_id: Uuid,
    caller_org: Uuid,
) -> AppResult<Option<(Uuid, String)>> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        r#"
        SELECT c.org_id, cc.role
        FROM   contracts c
        JOIN   contract_collaborators cc ON cc.contract_id = c.id
        WHERE  c.id = $1 AND c.deleted_at IS NULL AND cc.org_id = $2
        "#,
    )
    .bind(contract_id)
    .bind(caller_org)
    .fetch_optional(pool)
    .await
    .db_op("collaborator_grant_by_id")?;
    Ok(row)
}

/// True if `org_id` owns the live contract `contract_id`.
pub async fn org_owns_contract_id(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
) -> AppResult<bool> {
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM contracts WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL",
    )
    .bind(contract_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("org_owns_contract_id")?;
    Ok(row.is_some())
}

// ---------------------------------------------------------------------------
// Collaborator grants (by contract id)
// ---------------------------------------------------------------------------

const COLLAB_COLS: &str = "c.name AS contract_name, x.org_id, x.role, x.granted_by, x.granted_at";

pub async fn list_collaborators(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Vec<CollaboratorRow>> {
    let sql = format!(
        "SELECT {COLLAB_COLS} FROM contract_collaborators x \
         JOIN contracts c ON c.id = x.contract_id \
         WHERE x.contract_id = $1 ORDER BY x.granted_at ASC"
    );
    sqlx::query_as::<_, CollaboratorRow>(&sql)
        .bind(contract_id)
        .fetch_all(pool)
        .await
        .db_op("list_collaborators")
}

/// Grant a role on a specific contract.  `granted_by` is the contract's owner
/// org (taken from the contract row, not the caller).  Idempotent upsert.
pub async fn grant_collaborator(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
    role: &str,
) -> AppResult<CollaboratorRow> {
    let sql = format!(
        "WITH x AS ( \
           INSERT INTO contract_collaborators \
               (contract_id, contract_name, org_id, role, granted_by, granted_at) \
           SELECT c.id, c.name, $2, $3, c.org_id, NOW() FROM contracts c \
           WHERE  c.id = $1 AND c.deleted_at IS NULL \
           ON CONFLICT (contract_id, org_id) \
           DO UPDATE SET role = EXCLUDED.role, granted_by = EXCLUDED.granted_by, \
                         granted_at = NOW() \
           RETURNING * ) \
         SELECT {COLLAB_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, CollaboratorRow>(&sql)
        .bind(contract_id)
        .bind(org_id)
        .bind(role)
        .fetch_optional(pool)
        .await
        .db_op("grant_collaborator")?
        .ok_or(AppError::ContractNotFound(contract_id))
}

pub async fn update_collaborator_role(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
    new_role: &str,
) -> AppResult<CollaboratorRow> {
    let sql = format!(
        "WITH x AS ( \
           UPDATE contract_collaborators SET role = $3 \
           WHERE contract_id = $1 AND org_id = $2 RETURNING * ) \
         SELECT {COLLAB_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, CollaboratorRow>(&sql)
        .bind(contract_id)
        .bind(org_id)
        .bind(new_role)
        .fetch_optional(pool)
        .await
        .db_op("update_collaborator_role")?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "collaborator org '{org_id}' not found on this contract"
            ))
        })
}

pub async fn revoke_collaborator(pool: &PgPool, contract_id: Uuid, org_id: Uuid) -> AppResult<()> {
    let result =
        sqlx::query("DELETE FROM contract_collaborators WHERE contract_id = $1 AND org_id = $2")
            .bind(contract_id)
            .bind(org_id)
            .execute(pool)
            .await
            .db_op("revoke_collaborator")?;
    if result.rows_affected() == 0 {
        return Err(AppError::BadRequest(format!(
            "collaborator org '{org_id}' not found on this contract"
        )));
    }
    Ok(())
}

/// Org-visibility import: grant the importer `viewer` on the published
/// contract.  `granted_by` is the contract's owner org.  Idempotent.
pub async fn ensure_viewer_collaborator(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Uuid,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO contract_collaborators
            (contract_id, contract_name, org_id, role, granted_by, granted_at)
        SELECT c.id, c.name, $2, 'viewer', c.org_id, NOW()
        FROM   contracts c
        WHERE  c.id = $1 AND c.deleted_at IS NULL AND c.org_id <> $2
        ON CONFLICT (contract_id, org_id) DO NOTHING
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .execute(pool)
    .await
    .db_op("ensure_viewer_collaborator")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Comments (by contract id)
// ---------------------------------------------------------------------------

const COMMENT_COLS: &str =
    "x.id, c.name AS contract_name, x.field, x.org_id, x.author, x.body, x.resolved, x.created_at";

pub async fn list_comments(pool: &PgPool, contract_id: Uuid) -> AppResult<Vec<CommentRow>> {
    let sql = format!(
        "SELECT {COMMENT_COLS} FROM contract_comments x \
         JOIN contracts c ON c.id = x.contract_id \
         WHERE x.contract_id = $1 ORDER BY x.created_at ASC"
    );
    sqlx::query_as::<_, CommentRow>(&sql)
        .bind(contract_id)
        .fetch_all(pool)
        .await
        .db_op("list_comments")
}

pub async fn add_comment(
    pool: &PgPool,
    contract_id: Uuid,
    field: Option<&str>,
    org_id: Uuid,
    author: &str,
    body: &str,
) -> AppResult<CommentRow> {
    let sql = format!(
        "WITH x AS ( \
           INSERT INTO contract_comments \
               (contract_id, contract_name, field, org_id, author, body, resolved, created_at) \
           SELECT c.id, c.name, $2, $3, $4, $5, false, NOW() FROM contracts c WHERE c.id = $1 \
           RETURNING * ) \
         SELECT {COMMENT_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, CommentRow>(&sql)
        .bind(contract_id)
        .bind(field)
        .bind(org_id)
        .bind(author)
        .bind(body)
        .fetch_optional(pool)
        .await
        .db_op("add_comment")?
        .ok_or(AppError::ContractNotFound(contract_id))
}

pub async fn resolve_comment(
    pool: &PgPool,
    contract_id: Uuid,
    comment_id: Uuid,
) -> AppResult<CommentRow> {
    let sql = format!(
        "WITH x AS ( \
           UPDATE contract_comments SET resolved = true \
           WHERE id = $1 AND contract_id = $2 RETURNING * ) \
         SELECT {COMMENT_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, CommentRow>(&sql)
        .bind(comment_id)
        .bind(contract_id)
        .fetch_optional(pool)
        .await
        .db_op("resolve_comment")?
        .ok_or_else(|| AppError::NotFound(format!("comment '{comment_id}' not found")))
}

/// Author org of one comment on `contract_id`.
pub async fn get_comment_org(
    pool: &PgPool,
    contract_id: Uuid,
    comment_id: Uuid,
) -> AppResult<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT org_id FROM contract_comments WHERE id = $1 AND contract_id = $2")
            .bind(comment_id)
            .bind(contract_id)
            .fetch_optional(pool)
            .await
            .db_op("get_comment_org")?;
    Ok(row.map(|(o,)| o))
}

pub async fn delete_comment(pool: &PgPool, contract_id: Uuid, comment_id: Uuid) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM contract_comments WHERE id = $1 AND contract_id = $2")
        .bind(comment_id)
        .bind(contract_id)
        .execute(pool)
        .await
        .db_op("delete_comment")?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "comment '{comment_id}' not found"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Proposals (by contract id)
// ---------------------------------------------------------------------------

const PROPOSAL_COLS: &str = "x.id, c.name AS contract_name, x.proposed_by, x.proposed_yaml, \
                             x.status, x.decided_by, x.created_at";

pub async fn list_proposals(pool: &PgPool, contract_id: Uuid) -> AppResult<Vec<ProposalRow>> {
    let sql = format!(
        "SELECT {PROPOSAL_COLS} FROM contract_change_proposals x \
         JOIN contracts c ON c.id = x.contract_id \
         WHERE x.contract_id = $1 ORDER BY x.created_at DESC"
    );
    sqlx::query_as::<_, ProposalRow>(&sql)
        .bind(contract_id)
        .fetch_all(pool)
        .await
        .db_op("list_proposals")
}

pub async fn create_proposal(
    pool: &PgPool,
    contract_id: Uuid,
    proposed_by: Uuid,
    proposed_yaml: &str,
) -> AppResult<ProposalRow> {
    let sql = format!(
        "WITH x AS ( \
           INSERT INTO contract_change_proposals \
               (contract_id, contract_name, proposed_by, proposed_yaml, status, created_at) \
           SELECT c.id, c.name, $2, $3, 'open', NOW() FROM contracts c WHERE c.id = $1 \
           RETURNING * ) \
         SELECT {PROPOSAL_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, ProposalRow>(&sql)
        .bind(contract_id)
        .bind(proposed_by)
        .bind(proposed_yaml)
        .fetch_optional(pool)
        .await
        .db_op("create_proposal")?
        .ok_or(AppError::ContractNotFound(contract_id))
}

/// Set an `open` proposal on `contract_id` to `approved` or `rejected`.
pub async fn decide_proposal(
    pool: &PgPool,
    contract_id: Uuid,
    proposal_id: Uuid,
    new_status: &str,
    decided_by: Uuid,
) -> AppResult<ProposalRow> {
    let sql = format!(
        "WITH x AS ( \
           UPDATE contract_change_proposals SET status = $2, decided_by = $3 \
           WHERE id = $1 AND status = 'open' AND contract_id = $4 RETURNING * ) \
         SELECT {PROPOSAL_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, ProposalRow>(&sql)
        .bind(proposal_id)
        .bind(new_status)
        .bind(decided_by)
        .bind(contract_id)
        .fetch_optional(pool)
        .await
        .db_op("decide_proposal")?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "proposal '{proposal_id}' not found or is not in 'open' status"
            ))
        })
}

/// Mark an `approved` proposal on `contract_id` as `applied`.
pub async fn apply_proposal(
    pool: &PgPool,
    contract_id: Uuid,
    proposal_id: Uuid,
) -> AppResult<ProposalRow> {
    let sql = format!(
        "WITH x AS ( \
           UPDATE contract_change_proposals SET status = 'applied' \
           WHERE id = $1 AND status = 'approved' AND contract_id = $2 RETURNING * ) \
         SELECT {PROPOSAL_COLS} FROM x JOIN contracts c ON c.id = x.contract_id"
    );
    sqlx::query_as::<_, ProposalRow>(&sql)
        .bind(proposal_id)
        .bind(contract_id)
        .fetch_optional(pool)
        .await
        .db_op("apply_proposal")?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "proposal '{proposal_id}' not found or is not in 'approved' status"
            ))
        })
}

/// `(proposed_by, status)` of one proposal on `contract_id`.
pub async fn get_proposal_meta(
    pool: &PgPool,
    contract_id: Uuid,
    proposal_id: Uuid,
) -> AppResult<Option<(Uuid, String)>> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT proposed_by, status FROM contract_change_proposals WHERE id = $1 AND contract_id = $2",
    )
    .bind(proposal_id)
    .bind(contract_id)
    .fetch_optional(pool)
    .await
    .db_op("get_proposal_meta")?;
    Ok(row)
}

/// Delete one proposal on `contract_id`.  Caller checks permission.
pub async fn delete_proposal(pool: &PgPool, contract_id: Uuid, proposal_id: Uuid) -> AppResult<()> {
    let result =
        sqlx::query("DELETE FROM contract_change_proposals WHERE id = $1 AND contract_id = $2")
            .bind(proposal_id)
            .bind(contract_id)
            .execute(pool)
            .await
            .db_op("delete_proposal")?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "proposal '{proposal_id}' not found"
        )));
    }
    Ok(())
}
