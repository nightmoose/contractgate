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
//! | DELETE | /contracts/{name}/comments/{id}                 | author org or owner | Delete a comment |
//! | GET    | /contracts/{name}/proposals                     | viewer  | List proposals |
//! | POST   | /contracts/{name}/proposals                     | editor  | Open a proposal |
//! | DELETE | /contracts/{name}/proposals/{id}                | proposer org (open) or owner | Delete a proposal |
//! | POST   | /contracts/{name}/proposals/{id}/decide         | reviewer| Approve / reject |
//! | POST   | /contracts/{name}/proposals/{id}/apply          | owner   | Apply an approved proposal |
//!
//! ## Role hierarchy
//!
//! owner > reviewer > editor > viewer
//!
//! "owner" is implicit — the org whose `org_id` matches `contracts.org_id`.
//! It is never stored in `contract_collaborators`.
//!
//! ## Contract identity (migration 042)
//!
//! The URL carries a contract name; it is resolved to one contract id (the
//! caller's own live contract, else one it holds a grant on) and every query
//! runs by that id. Grants, comments and proposals point at a contract id.
//! Two live matches → 409. A UUID in the `{name}` slot is treated as a
//! contract id and checked against the caller's own contract or its grant on
//! that id (N2), so grantees that own a same-named contract can still reach
//! the shared one. Names keep resolving to the caller's own contract first.

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

impl CallerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            CallerRole::Owner => "owner",
            CallerRole::Reviewer => "reviewer",
            CallerRole::Editor => "editor",
            CallerRole::Viewer => "viewer",
        }
    }

    fn from_stored(s: Option<&str>) -> Option<Self> {
        match s {
            Some("editor") => Some(Self::Editor),
            Some("reviewer") => Some(Self::Reviewer),
            Some("viewer") => Some(Self::Viewer),
            _ => None,
        }
    }
}

/// C2: map the caller's (possibly absent) role to an access decision.
///
/// - role satisfies `min_role` → Ok
/// - contract doesn't exist anywhere → 404
/// - otherwise → 403 naming the role required (never the API-key 401)
pub(crate) fn decide_access(
    role: Option<CallerRole>,
    contract_exists: bool,
    min_role: CallerRole,
    contract_name: &str,
) -> AppResult<CallerRole> {
    match role {
        Some(r) if r.satisfies(min_role) => Ok(r),
        None if !contract_exists => Err(AppError::NotFound(format!(
            "contract '{contract_name}' not found"
        ))),
        _ => Err(forbidden_role(min_role, contract_name)),
    }
}

pub(crate) fn forbidden_role(min_role: CallerRole, contract_name: &str) -> AppError {
    let need = if min_role == CallerRole::Owner {
        "owner role".to_string()
    } else {
        format!("{} role or higher", min_role.as_str())
    };
    AppError::Forbidden(format!("requires {need} on contract '{contract_name}'"))
}

/// Caller's role on a contract id: owner, its active grant, or `None`.
pub(crate) async fn id_role(
    state: &AppState,
    contract_id: Uuid,
    caller_org: Uuid,
) -> AppResult<Option<CallerRole>> {
    if storage::org_owns_contract_id(&state.db, contract_id, caller_org).await? {
        return Ok(Some(CallerRole::Owner));
    }
    let grant = storage::collaborator_grant_by_id(&state.db, contract_id, caller_org).await?;
    Ok(grant.and_then(|(_, r)| CallerRole::from_stored(Some(r.as_str()))))
}

/// N2: decision for by-id routes.  No role at all → 404 (don't reveal the
/// contract); a role below `min_role` → 403 naming the role.
pub(crate) fn decide_id_access(
    role: Option<CallerRole>,
    min_role: CallerRole,
    contract_id: Uuid,
) -> AppResult<CallerRole> {
    match role {
        None => Err(AppError::ContractNotFound(contract_id)),
        Some(r) if r.satisfies(min_role) => Ok(r),
        Some(_) => Err(forbidden_role(min_role, &contract_id.to_string())),
    }
}

/// At most one match is allowed; several live contracts behind one URL name
/// is a 409 instead of silently picking one.
pub(crate) fn pick_one<T>(
    mut found: Vec<T>,
    contract_name: &str,
    whose: &str,
) -> AppResult<Option<T>> {
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        n => Err(AppError::Conflict(format!(
            "{n} live contracts named '{contract_name}' {whose}; rename or delete the extras"
        ))),
    }
}

/// Resolve the URL contract name to a contract id for the caller (its own org
/// first, then a collaborator grant) and assert `min_role`.
/// 401 only when there is no authenticated org (real auth failure).
async fn require_role(
    state: &AppState,
    contract_name: &str,
    caller_org: Option<Uuid>,
    min_role: CallerRole,
) -> AppResult<(Uuid, CallerRole)> {
    let caller_org = caller_org.ok_or(AppError::Unauthorized)?;
    // N2: a UUID in the path is a contract id — resolve it directly, so a
    // grantee that owns a same-named contract can still reach the shared one.
    if let Ok(contract_id) = Uuid::parse_str(contract_name) {
        let role = id_role(state, contract_id, caller_org).await?;
        return decide_id_access(role, min_role, contract_id).map(|r| (contract_id, r));
    }
    let owned = storage::owned_contract_ids_by_name(&state.db, contract_name, caller_org).await?;
    let found = match pick_one(owned, contract_name, "in your org")? {
        Some(id) => Some((id, CallerRole::Owner)),
        None => {
            let granted =
                storage::granted_contracts_by_name(&state.db, contract_name, caller_org).await?;
            pick_one(granted, contract_name, "are shared with your org")?.and_then(|(id, role)| {
                CallerRole::from_stored(Some(role.as_str())).map(|r| (id, r))
            })
        }
    };
    let exists = found.is_some() || storage::contract_name_exists(&state.db, contract_name).await?;
    let role = decide_access(found.map(|(_, r)| r), exists, min_role, contract_name)?;
    Ok((found.expect("decide_access ok implies a role").0, role))
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_collaborators(&state.db, cid).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let Json(body): Json<GrantCollaboratorRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    validate_collaborator_role(&body.role)?;
    let row = storage::grant_collaborator(&state.db, cid, body.org_id, &body.role).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let Json(body): Json<PatchCollaboratorRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    validate_collaborator_role(&body.role)?;
    let row = storage::update_collaborator_role(&state.db, cid, target_org, &body.role).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;
    storage::revoke_collaborator(&state.db, cid, target_org).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_comments(&state.db, cid).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;

    let caller_org = org_id.ok_or(AppError::Unauthorized)?;
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
        cid,
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let row = storage::resolve_comment(&state.db, cid, comment_id).await?;
    Ok(Json(row))
}

// ---------------------------------------------------------------------------
// DELETE /contracts/{name}/comments/{id}
// ---------------------------------------------------------------------------

/// Delete a comment.  Allowed for the comment's author org or the contract
/// owner; any other collaborator gets 403.
pub async fn delete_comment_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, comment_id)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<StatusCode> {
    let org_id = crate::org_id_from_req(&req);
    let (cid, role) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let caller_org = org_id.ok_or(AppError::Unauthorized)?;
    let author_org = storage::get_comment_org(&state.db, cid, comment_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("comment '{comment_id}' not found")))?;
    check_comment_delete(role, caller_org, author_org, &contract_name)?;
    storage::delete_comment(&state.db, cid, comment_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn check_comment_delete(
    role: CallerRole,
    caller_org: Uuid,
    author_org: Uuid,
    contract_name: &str,
) -> AppResult<()> {
    if role == CallerRole::Owner || caller_org == author_org {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "requires owner role on contract '{contract_name}' or the comment's author org"
        )))
    }
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let rows = storage::list_proposals(&state.db, cid).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Editor).await?;

    let caller_org = org_id.ok_or(AppError::Unauthorized)?;
    let Json(body): Json<CreateProposalRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    if body.proposed_yaml.trim().is_empty() {
        return Err(AppError::BadRequest("proposed_yaml cannot be empty".into()));
    }

    let row = storage::create_proposal(&state.db, cid, caller_org, &body.proposed_yaml).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

// ---------------------------------------------------------------------------
// DELETE /contracts/{name}/proposals/{id}
// ---------------------------------------------------------------------------

/// Delete a proposal: the proposing org while it is still `open`, or the
/// contract owner at any time.
pub async fn delete_proposal_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_name, proposal_id)): Path<(String, Uuid)>,
    req: axum::extract::Request,
) -> AppResult<StatusCode> {
    let org_id = crate::org_id_from_req(&req);
    let (cid, role) = require_role(&state, &contract_name, org_id, CallerRole::Viewer).await?;
    let caller_org = org_id.ok_or(AppError::Unauthorized)?;
    let (proposed_by, status) = storage::get_proposal_meta(&state.db, cid, proposal_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal '{proposal_id}' not found")))?;
    check_proposal_delete(role, caller_org, proposed_by, &status, &contract_name)?;
    storage::delete_proposal(&state.db, cid, proposal_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn check_proposal_delete(
    role: CallerRole,
    caller_org: Uuid,
    proposed_by: Uuid,
    status: &str,
    contract_name: &str,
) -> AppResult<()> {
    if role == CallerRole::Owner {
        return Ok(());
    }
    if caller_org == proposed_by {
        if status == "open" {
            return Ok(());
        }
        return Err(AppError::Conflict(format!(
            "proposal is '{status}'; the proposing org can delete it only while it is open"
        )));
    }
    Err(AppError::Forbidden(format!(
        "requires owner role on contract '{contract_name}' or the proposing org"
    )))
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Reviewer).await?;

    let caller_org = org_id.ok_or(AppError::Unauthorized)?;
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

    let row = storage::decide_proposal(&state.db, cid, proposal_id, new_status, caller_org).await?;
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
    let (cid, _) = require_role(&state, &contract_name, org_id, CallerRole::Owner).await?;

    let row = storage::apply_proposal(&state.db, cid, proposal_id).await?;
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

    fn status(e: AppError) -> (axum::http::StatusCode, String) {
        use axum::response::IntoResponse;
        let msg = e.to_string();
        (e.into_response().status(), msg)
    }

    #[test]
    fn non_collaborator_gets_403_naming_role() {
        let (code, msg) =
            status(decide_access(None, true, CallerRole::Viewer, "orders").unwrap_err());
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert!(
            msg.contains("requires viewer role or higher on contract 'orders'"),
            "{msg}"
        );
        assert!(!msg.contains("API key"));
    }

    #[test]
    fn non_owner_grant_gets_403_owner_role() {
        for r in [CallerRole::Viewer, CallerRole::Editor, CallerRole::Reviewer] {
            let (code, msg) =
                status(decide_access(Some(r), true, CallerRole::Owner, "orders").unwrap_err());
            assert_eq!(code, StatusCode::FORBIDDEN);
            assert!(
                msg.contains("requires owner role on contract 'orders'"),
                "{msg}"
            );
        }
    }

    #[test]
    fn viewer_cannot_open_proposal_403() {
        let (code, msg) = status(
            decide_access(Some(CallerRole::Viewer), true, CallerRole::Editor, "x").unwrap_err(),
        );
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert!(msg.contains("editor role or higher"));
    }

    #[test]
    fn viewer_can_read() {
        assert_eq!(
            decide_access(Some(CallerRole::Viewer), true, CallerRole::Viewer, "x").unwrap(),
            CallerRole::Viewer
        );
    }

    #[test]
    fn unknown_contract_gets_404() {
        let (code, _) = status(decide_access(None, false, CallerRole::Viewer, "nope").unwrap_err());
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[test]
    fn pick_one_resolves_single_and_rejects_ambiguous() {
        assert_eq!(
            pick_one(Vec::<u8>::new(), "x", "in your org").unwrap(),
            None
        );
        assert_eq!(pick_one(vec![7u8], "x", "in your org").unwrap(), Some(7));
        let (code, msg) = status(pick_one(vec![1u8, 2], "orders", "in your org").unwrap_err());
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(msg.contains("2 live contracts named 'orders'"), "{msg}");
    }

    #[test]
    fn id_access_editor_writes_viewer_403_stranger_404() {
        let id = Uuid::new_v4();
        assert!(decide_id_access(Some(CallerRole::Editor), CallerRole::Editor, id).is_ok());
        assert!(decide_id_access(Some(CallerRole::Reviewer), CallerRole::Editor, id).is_ok());
        assert!(decide_id_access(Some(CallerRole::Owner), CallerRole::Owner, id).is_ok());
        let (code, msg) =
            status(decide_id_access(Some(CallerRole::Viewer), CallerRole::Editor, id).unwrap_err());
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert!(msg.contains("editor role or higher"), "{msg}");
        let (code, msg) =
            status(decide_id_access(Some(CallerRole::Editor), CallerRole::Owner, id).unwrap_err());
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert!(msg.contains("requires owner role"), "{msg}");
        let (code, _) = status(decide_id_access(None, CallerRole::Viewer, id).unwrap_err());
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[test]
    fn proposal_delete_rules() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(check_proposal_delete(CallerRole::Editor, a, a, "open", "x").is_ok());
        assert!(check_proposal_delete(CallerRole::Owner, a, b, "approved", "x").is_ok());
        let (code, _) =
            status(check_proposal_delete(CallerRole::Editor, a, a, "approved", "x").unwrap_err());
        assert_eq!(code, StatusCode::CONFLICT);
        let (code, msg) =
            status(check_proposal_delete(CallerRole::Reviewer, a, b, "open", "x").unwrap_err());
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert!(msg.contains("proposing org"));
    }

    #[test]
    fn real_auth_failure_stays_401() {
        let (code, _) = status(AppError::Unauthorized);
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn comment_delete_author_or_owner_only() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(check_comment_delete(CallerRole::Viewer, a, a, "x").is_ok());
        assert!(check_comment_delete(CallerRole::Owner, a, b, "x").is_ok());
        let (code, _) = status(check_comment_delete(CallerRole::Reviewer, a, b, "x").unwrap_err());
        assert_eq!(code, StatusCode::FORBIDDEN);
    }

    #[test]
    fn validate_collaborator_role_rejects_unknown() {
        assert!(validate_collaborator_role("admin").is_err());
        assert!(validate_collaborator_role("superuser").is_err());
        assert!(validate_collaborator_role("").is_err());
    }
}
