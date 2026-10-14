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
// Collaborator functions
// ---------------------------------------------------------------------------

/// True if `org_id` owns a live (non-deleted) contract named `contract_name`.
pub async fn org_owns_contract_name(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
) -> AppResult<bool> {
    let row: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1 FROM contracts
        WHERE  name = $1 AND org_id = $2 AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("org_owns_contract_name")?;
    Ok(row.is_some())
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

/// Active collaborator role for `org_id` on `contract_name`.
///
/// A grant counts only while the granting org still owns a live contract of
/// that name, so grants on deleted contracts (or made by a non-owner) are void.
pub async fn get_collaborator_role(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT cc.role FROM contract_collaborators cc
        WHERE  cc.contract_name = $1 AND cc.org_id = $2
          AND  EXISTS (
                 SELECT 1 FROM contracts c
                 WHERE  c.name = cc.contract_name
                   AND  c.org_id = cc.granted_by
                   AND  c.deleted_at IS NULL
               )
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("get_collaborator_role")?;

    Ok(row.map(|(r,)| r))
}

/// C3: if `caller_org` holds an active collaborator grant (any role, viewer
/// and up) on the live contract `contract_id`, return the owner org id so the
/// caller can read it with the owner's scope.  `None` = no access.
pub async fn collaborator_read_owner(
    pool: &PgPool,
    contract_id: Uuid,
    caller_org: Uuid,
) -> AppResult<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT c.org_id
        FROM   contracts c
        JOIN   contract_collaborators cc
               ON cc.contract_name = c.name AND cc.granted_by = c.org_id
        WHERE  c.id = $1 AND c.deleted_at IS NULL
          AND  cc.org_id = $2
          AND  cc.role IN ('viewer', 'editor', 'reviewer')
        LIMIT 1
        "#,
    )
    .bind(contract_id)
    .bind(caller_org)
    .fetch_optional(pool)
    .await
    .db_op("collaborator_read_owner")?;
    Ok(row.map(|(id,)| id))
}

/// List all collaborator grants for a contract, ordered by granted_at.
pub async fn list_collaborators(
    pool: &PgPool,
    contract_name: &str,
) -> AppResult<Vec<CollaboratorRow>> {
    let rows = sqlx::query_as::<_, CollaboratorRow>(
        r#"
        SELECT contract_name, org_id, role, granted_by, granted_at
        FROM   contract_collaborators
        WHERE  contract_name = $1
        ORDER  BY granted_at ASC
        "#,
    )
    .bind(contract_name)
    .fetch_all(pool)
    .await
    .db_op("list_collaborators")?;

    Ok(rows)
}

/// Grant a role to an org on a contract.  Uses INSERT … ON CONFLICT UPDATE so
/// calling this twice just updates the role (idempotent from the caller's view).
pub async fn grant_collaborator(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
    role: &str,
    granted_by: Uuid,
) -> AppResult<CollaboratorRow> {
    let row = sqlx::query_as::<_, CollaboratorRow>(
        r#"
        INSERT INTO contract_collaborators
            (contract_name, org_id, role, granted_by, granted_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (contract_name, org_id)
        DO UPDATE SET role = EXCLUDED.role, granted_by = EXCLUDED.granted_by,
                      granted_at = NOW()
        RETURNING contract_name, org_id, role, granted_by, granted_at
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .bind(role)
    .bind(granted_by)
    .fetch_one(pool)
    .await
    .db_op("grant_collaborator")?;

    Ok(row)
}

/// Update an existing collaborator's role.  Returns NotFound if no row exists.
pub async fn update_collaborator_role(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
    new_role: &str,
) -> AppResult<CollaboratorRow> {
    let row = sqlx::query_as::<_, CollaboratorRow>(
        r#"
        UPDATE contract_collaborators
        SET    role = $3
        WHERE  contract_name = $1 AND org_id = $2
        RETURNING contract_name, org_id, role, granted_by, granted_at
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .bind(new_role)
    .fetch_optional(pool)
    .await
    .db_op("update_collaborator_role")?
    .ok_or_else(|| {
        AppError::BadRequest(format!(
            "collaborator org '{org_id}' not found on contract '{contract_name}'"
        ))
    })?;

    Ok(row)
}

/// Remove a collaborator grant entirely.
pub async fn revoke_collaborator(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
) -> AppResult<()> {
    let result = sqlx::query(
        r#"
        DELETE FROM contract_collaborators
        WHERE  contract_name = $1 AND org_id = $2
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .execute(pool)
    .await
    .db_op("revoke_collaborator")?;

    if result.rows_affected() == 0 {
        return Err(AppError::BadRequest(format!(
            "collaborator org '{org_id}' not found on contract '{contract_name}'"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Comment functions
// ---------------------------------------------------------------------------

/// List all comments for a contract, oldest first.
pub async fn list_comments(pool: &PgPool, contract_name: &str) -> AppResult<Vec<CommentRow>> {
    let rows = sqlx::query_as::<_, CommentRow>(
        r#"
        SELECT id, contract_name, field, org_id, author, body, resolved, created_at
        FROM   contract_comments
        WHERE  contract_name = $1
        ORDER  BY created_at ASC
        "#,
    )
    .bind(contract_name)
    .fetch_all(pool)
    .await
    .db_op("list_comments")?;

    Ok(rows)
}

/// Insert a new comment.
pub async fn add_comment(
    pool: &PgPool,
    contract_name: &str,
    field: Option<&str>,
    org_id: Uuid,
    author: &str,
    body: &str,
) -> AppResult<CommentRow> {
    let row = sqlx::query_as::<_, CommentRow>(
        r#"
        INSERT INTO contract_comments
            (contract_name, field, org_id, author, body, resolved, created_at)
        VALUES ($1, $2, $3, $4, $5, false, NOW())
        RETURNING id, contract_name, field, org_id, author, body, resolved, created_at
        "#,
    )
    .bind(contract_name)
    .bind(field)
    .bind(org_id)
    .bind(author)
    .bind(body)
    .fetch_one(pool)
    .await
    .db_op("add_comment")?;

    Ok(row)
}

/// Mark a comment as resolved.
pub async fn resolve_comment(
    pool: &PgPool,
    contract_name: &str,
    comment_id: Uuid,
) -> AppResult<CommentRow> {
    let row = sqlx::query_as::<_, CommentRow>(
        r#"
        UPDATE contract_comments
        SET    resolved = true
        WHERE  id = $1 AND contract_name = $2
        RETURNING id, contract_name, field, org_id, author, body, resolved, created_at
        "#,
    )
    .bind(comment_id)
    .bind(contract_name)
    .fetch_optional(pool)
    .await
    .db_op("resolve_comment")?
    .ok_or_else(|| AppError::NotFound(format!("comment '{comment_id}' not found")))?;

    Ok(row)
}

/// Fetch one comment's author org, scoped to its contract.
pub async fn get_comment_org(
    pool: &PgPool,
    contract_name: &str,
    comment_id: Uuid,
) -> AppResult<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT org_id FROM contract_comments WHERE id = $1 AND contract_name = $2")
            .bind(comment_id)
            .bind(contract_name)
            .fetch_optional(pool)
            .await
            .db_op("get_comment_org")?;
    Ok(row.map(|(o,)| o))
}

/// Delete one comment, scoped to its contract.  Caller checks permission.
pub async fn delete_comment(pool: &PgPool, contract_name: &str, comment_id: Uuid) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM contract_comments WHERE id = $1 AND contract_name = $2")
        .bind(comment_id)
        .bind(contract_name)
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
// Proposal functions
// ---------------------------------------------------------------------------

/// List all proposals for a contract, newest first.
pub async fn list_proposals(pool: &PgPool, contract_name: &str) -> AppResult<Vec<ProposalRow>> {
    let rows = sqlx::query_as::<_, ProposalRow>(
        r#"
        SELECT id, contract_name, proposed_by, proposed_yaml, status, decided_by, created_at
        FROM   contract_change_proposals
        WHERE  contract_name = $1
        ORDER  BY created_at DESC
        "#,
    )
    .bind(contract_name)
    .fetch_all(pool)
    .await
    .db_op("list_proposals")?;

    Ok(rows)
}

/// Open a new change proposal (status = 'open').
pub async fn create_proposal(
    pool: &PgPool,
    contract_name: &str,
    proposed_by: Uuid,
    proposed_yaml: &str,
) -> AppResult<ProposalRow> {
    let row = sqlx::query_as::<_, ProposalRow>(
        r#"
        INSERT INTO contract_change_proposals
            (contract_name, proposed_by, proposed_yaml, status, created_at)
        VALUES ($1, $2, $3, 'open', NOW())
        RETURNING id, contract_name, proposed_by, proposed_yaml, status, decided_by, created_at
        "#,
    )
    .bind(contract_name)
    .bind(proposed_by)
    .bind(proposed_yaml)
    .fetch_one(pool)
    .await
    .db_op("create_proposal")?;

    Ok(row)
}

/// Set a proposal status to `approved` or `rejected`.
/// Only operates on proposals that are currently `open`.
pub async fn decide_proposal(
    pool: &PgPool,
    contract_name: &str,
    proposal_id: Uuid,
    new_status: &str, // "approved" | "rejected"
    decided_by: Uuid,
) -> AppResult<ProposalRow> {
    let row = sqlx::query_as::<_, ProposalRow>(
        r#"
        UPDATE contract_change_proposals
        SET    status = $2, decided_by = $3
        WHERE  id = $1 AND status = 'open' AND contract_name = $4
        RETURNING id, contract_name, proposed_by, proposed_yaml, status, decided_by, created_at
        "#,
    )
    .bind(proposal_id)
    .bind(new_status)
    .bind(decided_by)
    .bind(contract_name)
    .fetch_optional(pool)
    .await
    .db_op("decide_proposal")?
    .ok_or_else(|| {
        AppError::BadRequest(format!(
            "proposal '{proposal_id}' not found or is not in 'open' status"
        ))
    })?;

    Ok(row)
}

/// Mark an `approved` proposal as `applied`.
/// Only the owner calls this; the handler has already enforced that.
/// Only operates on proposals that are currently `approved`.
pub async fn apply_proposal(
    pool: &PgPool,
    contract_name: &str,
    proposal_id: Uuid,
) -> AppResult<ProposalRow> {
    let row = sqlx::query_as::<_, ProposalRow>(
        r#"
        UPDATE contract_change_proposals
        SET    status = 'applied'
        WHERE  id = $1 AND status = 'approved' AND contract_name = $2
        RETURNING id, contract_name, proposed_by, proposed_yaml, status, decided_by, created_at
        "#,
    )
    .bind(proposal_id)
    .bind(contract_name)
    .fetch_optional(pool)
    .await
    .db_op("apply_proposal")?
    .ok_or_else(|| {
        AppError::BadRequest(format!(
            "proposal '{proposal_id}' not found or is not in 'approved' status"
        ))
    })?;

    Ok(row)
}

/// Grant a `viewer` collaborator role to the importing org when a publication
/// with `org` visibility is imported.  Idempotent — safe to call twice.
///
/// Called from `import_published_contract` when `pub_row.visibility == "org"`.
pub async fn ensure_viewer_collaborator(
    pool: &PgPool,
    contract_name: &str,
    org_id: Uuid,
    granted_by: Uuid,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO contract_collaborators
            (contract_name, org_id, role, granted_by, granted_at)
        VALUES ($1, $2, 'viewer', $3, NOW())
        ON CONFLICT (contract_name, org_id) DO NOTHING
        "#,
    )
    .bind(contract_name)
    .bind(org_id)
    .bind(granted_by)
    .execute(pool)
    .await
    .db_op("ensure_viewer_collaborator")?;

    Ok(())
}
