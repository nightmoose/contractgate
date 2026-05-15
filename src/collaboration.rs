//! Provider-Consumer Collaboration handlers (RFC-033).
//!
//! ## Routes (wired in main.rs — all protected)
//!
//! | Method | Path | Min role | Description |
//! |--------|------|----------|-------------|
//! | GET    | /contracts/{name}/collaborators                 | viewer  | List collaborators |
//! | POST   | /contracts/{name}/collaborators                 | owner   | Grant a role |
//! | PATCH  | /contracts/{name}/collaborators/{org_id}        | owner   | Change a role |
//! | DELETE | /contracts/{name}/collaborators/{org_id}        | owner   | Revoke a role |
//! | GET    | /contracts/{name}/comments                      | viewer  | List comments |
//! | POST   | /contracts/{name}/comments                      | viewer  | Add a comment |
//! | POST   | /contracts/{name}/comments/{id}/resolve         | viewer  | Resolve a comment |
//! | GET    | /contracts/{name}/proposals                     | viewer  | List proposals |
//! | POST   | /contracts/{name}/proposals                     | editor  | Open a proposal |
//! | POST   | /contracts/{name}/proposals/{id}/decide         | reviewer| Approve / reject |
//! | POST   | /contracts/{name}/proposals/{id}/apply          | owner   | Apply an approved proposal |
//!
//! ## Role hierarchy
//!
//! owner > reviewer > editor > viewer
//!
//! "owner" is implicit — the org whose `org_id` matches `contracts.org_id`.
//! It is never stored in `contract_collaborators`.

use axum::{
    extract::{FromRequest, Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::storage::{self, CollaboratorRow, CommentRow, ProposalRow};
use crate::AppState;

// ---------------------------------------------------------------------------
// Role helpers
// ---------------------------------------------------------------------------

/// Effective role of the caller on a given contract.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CallerRole {
    Owner,
    Editor,
    Reviewer,
    Viewer,
}

impl CallerRole {
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "editor" => Some(Self::Editor),
            "reviewer" => Some(Self::Reviewer),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Returns true if this role satisfies the minimum required role.
    ///
    /// Order: owner > reviewer > editor > viewer
    pub fn satisfies(self, min: CallerRole) -> bool {
        let rank = |r: CallerRole| match r {
            CallerRole::Owner => 4,
            CallerRole::Reviewer => 3,
            CallerRole::Editor => 2,
            CallerRole::Viewer => 1,
        };
        rank(self) >= rank(min)
    }
}

/// Resolve the caller's effective role on `contract_name`.
///
/// Returns `None` if the caller has no access at all.
async fn resolve_role(
    state: &AppState,
    contract_name: &str,
    caller_org: Option<Uuid>,
) -> AppResult<CallerRole> {
    let caller_org = caller_org.ok_or(AppError::Unauthorized)?;

    // Check if this org owns the contract.
    let owner_org = storage::get_contract_owner_org(&state.db, contract_name).await?;
    if owner_org == Some(caller_org) {
        return Ok(CallerRole::Owner);
    }

    // Check collaborator table.
    let role_str = storage::get_collaborator_role(&state.db, contract_name, caller_org).await?;
    match role_str.as_deref() {
        Some("editor") => Ok(CallerRole::Editor),
        Some("reviewer") => Ok(CallerRole::Reviewer),
        Some("viewer") => Ok(CallerRole::Viewer),
        _ => Err(AppError::Unauthorized),
    }
}

/// Asserts the caller meets `min_role`.  Returns the resolved role on success.
async fn require_role(
    state: &AppState,
    contract_name: &str,
    caller_org: Option<Uuid>,
    min_role: CallerRole,
) -> AppResult<CallerRole> {
    let role = resolve_role(state, contract_name, caller_org).await?;
    if role.satisfies(min_role) {
        Ok(role)
    } else {
        Err(AppError::Unauthorized)
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct GrantCollaboratorRequest {
    pub org_id: Uuid,
    pub role: String,
}

#[derive(Deserialize)]
pub struct PatchCollaboratorRequest {
    pub role: String,
}

#[derive(Deserialize)]
pub struct AddCommentRequest {
    /// Field name this comment is anchored to.  `null` = whole-contract comment.
    pub field: Option<String>,
    /// Display name or email of the author.
    pub author: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct CreateProposalRequest {
    pub proposed_yaml: String,
}

#[derive(Deserialize)]
pub struct DecideProposalRequest {
    /// `"approved"` or `"rejected"`.
    pub decision: String,
}

// ---------------------------------------------------------------------------
// GET /contracts/{name}/collaborators
// ---------------------------------------------------------------------------

pub async fn list_collaborators_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<Json<Vec<CollaboratorRow>>> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_collaborators(&state.db, &contract_name).await?;
    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/collaborators
// ---------------------------------------------------------------------------

pub async fn grant_collaborator_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<CollaboratorRow>)> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let Json(body): Json<GrantCollaboratorRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    validate_collaborator_role(&body.role)?;
    let granted_by = org_id.expect("owner check guarantees org_id is set");
    let row = storage::grant_collaborator(
        &state.db,
        &contract_name,
        body.org_id,
        &body.role,
        granted_by,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

// ---------------------------------------------------------------------------
// PATCH /contracts/{name}/collaborators/{org_id}
// ---------------------------------------------------------------------------

pub async fn patch_collaborator_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, target_org)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<Json<CollaboratorRow>> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let Json(body): Json<PatchCollaboratorRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    validate_collaborator_role(&body.role)?;
    let row = storage::update_collaborator_role(&state.db, &contract_name, target_org, &body.role)
        .await?;
    Ok(Json(row))
}

// ---------------------------------------------------------------------------
// DELETE /contracts/{name}/collaborators/{org_id}
// ---------------------------------------------------------------------------

pub async fn revoke_collaborator_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, target_org)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<StatusCode> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;
    storage::revoke_collaborator(&state.db, &contract_name, target_org).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /contracts/{name}/comments
// ---------------------------------------------------------------------------

pub async fn list_comments_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<Json<Vec<CommentRow>>> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_comments(&state.db, &contract_name).await?;
    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/comments
// ---------------------------------------------------------------------------

pub async fn add_comment_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<CommentRow>)> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;

    let caller_org = org_id.expect("viewer check guarantees org_id is set");
    let Json(body): Json<AddCommentRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    if body.body.trim().is_empty() {
        return Err(AppError::BadRequest("comment body cannot be empty".into()));
    }
    if body.author.trim().is_empty() {
        return Err(AppError::BadRequest("author cannot be empty".into()));
    }

    let row = storage::add_comment(
        &state.db,
        &contract_name,
        body.field.as_deref(),
        caller_org,
        &body.author,
        &body.body,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/comments/{id}/resolve
// ---------------------------------------------------------------------------

pub async fn resolve_comment_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, comment_id)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<Json<CommentRow>> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let row = storage::resolve_comment(&state.db, comment_id).await?;
    Ok(Json(row))
}

// ---------------------------------------------------------------------------
// GET /contracts/{name}/proposals
// ---------------------------------------------------------------------------

pub async fn list_proposals_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<Json<Vec<ProposalRow>>> {
    let org_id = crate::org_id_from_req(&req);
    require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_proposals(&state.db, &contract_name).await?;
    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/proposals
// ---------------------------------------------------------------------------

pub async fn create_proposal_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_name): Path<String>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ProposalRow>)> {
    let org_id = crate::org_id_from_req(&req);
    // Only editors (and owners) may open proposals.
    require_role(&state, &contract_name, org_id, CallerRole::Editor).await?;

    let caller_org = org_id.expect("editor check guarantees org_id is set");
    let Json(body): Json<CreateProposalRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    if body.proposed_yaml.trim().is_empty() {
        return Err(AppError::BadRequest("proposed_yaml cannot be empty".into()));
    }

    let row = storage::create_proposal(&state.db, &contract_name, caller_org, &body.proposed_yaml)
        .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/proposals/{id}/decide
// ---------------------------------------------------------------------------

pub async fn decide_proposal_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, proposal_id)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<Json<ProposalRow>> {
    let org_id = crate::org_id_from_req(&req);
    // Reviewers and owners may decide.
    require_role(&state, &contract_name, org_id, CallerRole::Reviewer).await?;

    let caller_org = org_id.expect("reviewer check guarantees org_id is set");
    let Json(body): Json<DecideProposalRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let new_status = match body.decision.as_str() {
        "approved" => "approved",
        "rejected" => "rejected",
        other => {
            return Err(AppError::BadRequest(format!(
                "decision must be 'approved' or 'rejected', got '{other}'"
            )));
        }
    };

    let row = storage::decide_proposal(&state.db, proposal_id, new_status, caller_org).await?;
    Ok(Json(row))
}

// ---------------------------------------------------------------------------
// POST /contracts/{name}/proposals/{id}/apply
// ---------------------------------------------------------------------------

pub async fn apply_proposal_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, proposal_id)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<Json<ProposalRow>> {
    let org_id = crate::org_id_from_req(&req);
    // Only the owner may apply a proposal.
    require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let row = storage::apply_proposal(&state.db, proposal_id).await?;
    Ok(Json(row))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Validate that a role string is one of the stored roles (not "owner" — owner
/// is implicit).
fn validate_collaborator_role(role: &str) -> AppResult<()> {
    match role {
        "editor" | "reviewer" | "viewer" => Ok(()),
        "owner" => Err(AppError::BadRequest(
            "cannot grant 'owner' role — ownership is determined by the contract's org_id".into(),
        )),
        other => Err(AppError::BadRequest(format!(
            "invalid role '{other}': must be 'editor', 'reviewer', or 'viewer'"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_role_satisfies_owner_beats_all() {
        assert!(CallerRole::Owner.satisfies(CallerRole::Owner));
        assert!(CallerRole::Owner.satisfies(CallerRole::Reviewer));
        assert!(CallerRole::Owner.satisfies(CallerRole::Editor));
        assert!(CallerRole::Owner.satisfies(CallerRole::Viewer));
    }

    #[test]
    fn caller_role_satisfies_reviewer() {
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Reviewer));
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Editor));
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Reviewer.satisfies(CallerRole::Owner));
    }

    #[test]
    fn caller_role_satisfies_editor() {
        assert!(CallerRole::Editor.satisfies(CallerRole::Editor));
        assert!(CallerRole::Editor.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Editor.satisfies(CallerRole::Reviewer));
        assert!(!CallerRole::Editor.satisfies(CallerRole::Owner));
    }

    #[test]
    fn caller_role_satisfies_viewer_only_itself() {
        assert!(CallerRole::Viewer.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Editor));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Reviewer));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Owner));
    }

    #[test]
    fn caller_role_from_str_all_variants() {
        assert_eq!(CallerRole::from_str("owner"), Some(CallerRole::Owner));
        assert_eq!(CallerRole::from_str("editor"), Some(CallerRole::Editor));
        assert_eq!(CallerRole::from_str("reviewer"), Some(CallerRole::Reviewer));
        assert_eq!(CallerRole::from_str("viewer"), Some(CallerRole::Viewer));
        assert_eq!(CallerRole::from_str("admin"), None);
        assert_eq!(CallerRole::from_str(""), None);
    }

    #[test]
    fn validate_collaborator_role_accepts_valid() {
        assert!(validate_collaborator_role("editor").is_ok());
        assert!(validate_collaborator_role("reviewer").is_ok());
        assert!(validate_collaborator_role("viewer").is_ok());
    }

    #[test]
    fn validate_collaborator_role_rejects_owner() {
        let err = validate_collaborator_role("owner").unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("owner") || msg.to_lowercase().contains("owner"));
    }

    #[test]
    fn validate_collaborator_role_rejects_unknown() {
        assert!(validate_collaborator_role("admin").is_err());
        assert!(validate_collaborator_role("superuser").is_err());
        assert!(validate_collaborator_role("").is_err());
    }
}
