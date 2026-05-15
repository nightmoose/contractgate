//! Contract Sharing & Publication handlers (RFC-032).
//!
//! ## Routes (wired in main.rs)
//!
//! | Method | Path | Auth | Description |
//! |--------|------|------|-------------|
//! | POST | /contracts/{id}/versions/{v}/publish | required | Publish a version |
//! | DELETE | /contracts/publications/{ref} | required | Revoke a publication |
//! | GET | /published/{ref} | none | Fetch published contract |
//! | POST | /contracts/import-published | required | Import by ref |
//! | GET | /contracts/{id}/import-status | required | Check for updates |
//!
//! Visibility semantics:
//!   - `public`: anyone with the ref can fetch.
//!   - `link`:   requires the ref AND an unguessable link token (`?token=`).
//!   - `org`:    scoped to RFC-033 (not yet enforced; treated as `link` for now).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::contract::{ImportMode, PublicationVisibility};
use crate::error::{AppError, AppResult};
use crate::storage::{self, ImportStatusResult, PublicationRow};
use crate::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PublishRequest {
    /// `public` | `link` | `org`  (default: `link`)
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

fn default_visibility() -> String {
    "link".to_string()
}

#[derive(Serialize)]
pub struct PublishResponse {
    pub publication_ref: String,
    pub visibility: String,
    /// Only present when `visibility = "link"`.
    pub link_token: Option<String>,
    pub contract_name: String,
    pub contract_version: String,
    pub published_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct RevokeResponse {
    pub publication_ref: String,
    pub revoked_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct FetchedPublication {
    pub publication_ref: String,
    pub contract_name: String,
    pub contract_version: String,
    pub visibility: String,
    pub published_at: chrono::DateTime<chrono::Utc>,
    /// The locked YAML content of the published contract version.
    pub yaml_content: String,
}

#[derive(Deserialize)]
pub struct FetchQuery {
    /// Required when visibility = `link`.
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct ImportPublishedRequest {
    /// The publication ref returned by `POST .../publish`.
    pub publication_ref: String,
    /// Token required when visibility = `link`.
    pub link_token: Option<String>,
    /// `snapshot` (default) or `subscribe`.
    #[serde(default = "default_import_mode")]
    pub import_mode: String,
}

fn default_import_mode() -> String {
    "snapshot".to_string()
}

#[derive(Serialize)]
pub struct ImportPublishedResponse {
    /// The new contract_id in the consumer's org.
    pub contract_id: Uuid,
    pub version: String,
    pub import_mode: String,
    pub imported_from_ref: String,
}

// ---------------------------------------------------------------------------
// Helper: generate a link token (32 hex chars = 16 random bytes)
// ---------------------------------------------------------------------------

fn generate_link_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ---------------------------------------------------------------------------
// POST /contracts/{id}/versions/{version}/publish
// ---------------------------------------------------------------------------

pub async fn publish_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<PublishResponse>)> {
    let org_id = crate::org_id_from_req(&req);

    // Extract JSON body after reading extensions.
    let Json(body): Json<PublishRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let visibility: PublicationVisibility = body
        .visibility
        .parse()
        .map_err(|e: String| AppError::BadRequest(e))?;

    // Generate a link token only for `link` visibility.
    let link_token = if visibility == PublicationVisibility::Link {
        Some(generate_link_token())
    } else {
        None
    };

    let published_by = org_id.map(|id| id.to_string());

    let row = storage::publish_contract_version(
        &state.db,
        contract_id,
        &version,
        visibility,
        link_token.as_deref(),
        org_id,
        published_by.as_deref(),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(PublishResponse {
            publication_ref: row.publication_ref,
            visibility: row.visibility,
            link_token: row.link_token,
            contract_name: row.contract_name,
            contract_version: row.contract_version,
            published_at: row.published_at,
        }),
    ))
}

// ---------------------------------------------------------------------------
// DELETE /contracts/publications/{ref}
// ---------------------------------------------------------------------------

pub async fn revoke_handler(
    State(state): State<Arc<AppState>>,
    Path(publication_ref): Path<String>,
    req: axum::extract::Request,
) -> AppResult<Json<RevokeResponse>> {
    let org_id = crate::org_id_from_req(&req);
    let row = storage::revoke_publication(&state.db, &publication_ref, org_id).await?;

    Ok(Json(RevokeResponse {
        publication_ref: row.publication_ref,
        revoked_at: row.revoked_at.expect("revoke always sets revoked_at"),
    }))
}

// ---------------------------------------------------------------------------
// GET /published/{ref}  — public route, no auth middleware
// ---------------------------------------------------------------------------

pub async fn fetch_published_handler(
    State(state): State<Arc<AppState>>,
    Path(publication_ref): Path<String>,
    Query(q): Query<FetchQuery>,
) -> AppResult<Json<FetchedPublication>> {
    let row = storage::get_publication(&state.db, &publication_ref).await?;

    // 404 for revoked publications — don't reveal that it existed.
    if row.is_revoked() {
        return Err(AppError::BadRequest(format!(
            "publication '{}' not found",
            publication_ref
        )));
    }

    let vis = row
        .visibility_parsed()
        .ok_or_else(|| AppError::Internal(format!("unknown visibility '{}'", row.visibility)))?;

    // For `link` and `org` visibility, validate the token.
    match vis {
        PublicationVisibility::Public => {}
        PublicationVisibility::Link | PublicationVisibility::Org => {
            let provided = q.token.as_deref().unwrap_or("");
            let expected = row.link_token.as_deref().unwrap_or("");
            if provided.is_empty() {
                return Err(AppError::Unauthorized);
            }
            // Constant-time comparison to avoid timing attacks.
            if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(AppError::Unauthorized);
            }
        }
    }

    Ok(Json(FetchedPublication {
        publication_ref: row.publication_ref,
        contract_name: row.contract_name,
        contract_version: row.contract_version,
        visibility: row.visibility,
        published_at: row.published_at,
        yaml_content: row.yaml_content,
    }))
}

/// Constant-time byte slice comparison.  `pub(crate)` so tests in `tests.rs` can call it.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// POST /contracts/import-published
// ---------------------------------------------------------------------------

pub async fn import_published_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ImportPublishedResponse>)> {
    let org_id = crate::org_id_from_req(&req);

    let Json(body): Json<ImportPublishedRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let import_mode: ImportMode = body
        .import_mode
        .parse()
        .map_err(|e: String| AppError::BadRequest(e))?;

    // Fetch the publication and validate its state + token.
    let pub_row = storage::get_publication(&state.db, &body.publication_ref).await?;

    if pub_row.is_revoked() {
        return Err(AppError::BadRequest(format!(
            "publication '{}' has been revoked",
            body.publication_ref
        )));
    }

    // Validate link token when required.
    let vis = pub_row.visibility_parsed().ok_or_else(|| {
        AppError::Internal(format!("unknown visibility '{}'", pub_row.visibility))
    })?;

    match vis {
        PublicationVisibility::Public => {}
        PublicationVisibility::Link | PublicationVisibility::Org => {
            let provided = body.link_token.as_deref().unwrap_or("");
            let expected = pub_row.link_token.as_deref().unwrap_or("");
            if provided.is_empty() {
                return Err(AppError::BadRequest(
                    "link_token is required for this publication".to_string(),
                ));
            }
            if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(AppError::Unauthorized);
            }
        }
    }

    let cv = storage::import_published_contract(&state.db, &pub_row, import_mode, org_id).await?;

    Ok((
        StatusCode::CREATED,
        Json(ImportPublishedResponse {
            contract_id: cv.contract_id,
            version: cv.version,
            import_mode: import_mode.as_str().to_string(),
            imported_from_ref: pub_row.publication_ref,
        }),
    ))
}

// ---------------------------------------------------------------------------
// GET /contracts/{id}/import-status
// ---------------------------------------------------------------------------

pub async fn import_status_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<ImportStatusResult>> {
    let result = storage::check_import_status(&state.db, contract_id).await?;
    Ok(Json(result))
}
