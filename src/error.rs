//! Application-level error types for ContractGate.
//!
//! Uses `thiserror` to define structured errors that map cleanly to HTTP responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Contract not found: {0}")]
    ContractNotFound(uuid::Uuid),

    #[error("Invalid contract YAML: {0}")]
    InvalidContractYaml(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: missing or invalid API key")]
    Unauthorized,

    // -----------------------------------------------------------------------
    // Versioning (RFC-002)
    // -----------------------------------------------------------------------
    /// A version with this (contract_id, version) tuple already exists.
    /// Emitted on `POST /contracts/{id}/versions` when the requested semver
    /// is a duplicate.  409.
    #[error("Version {version} already exists on contract {contract_id}")]
    VersionConflict {
        contract_id: uuid::Uuid,
        version: String,
    },

    /// Attempt to mutate a non-draft version (yaml patch, delete, illegal
    /// state change).  The Postgres trigger also enforces this; this error
    /// lets the app emit a clean 409 before hitting the DB.
    #[error("Version {version} is {state} and cannot be modified")]
    VersionImmutable { version: String, state: String },

    /// A pinned or referenced version doesn't exist on the contract.  404.
    #[error("Version {version} not found on contract {contract_id}")]
    VersionNotFound {
        contract_id: uuid::Uuid,
        version: String,
    },

    /// An illegal state transition was requested (e.g. stable → draft, or
    /// promoting a deprecated version).  409.
    #[error("Invalid state transition {from} → {to} for version {version}")]
    InvalidStateTransition {
        from: String,
        to: String,
        version: String,
    },

    /// Unpinned traffic arrived for a contract that has no stable version
    /// published yet.  404 — the client should either publish one or pin a
    /// draft explicitly via `X-Contract-Version`.
    #[error("Contract {contract_id} has no stable version yet")]
    NoStableVersion { contract_id: uuid::Uuid },

    /// Traffic arrived pinned to a deprecated version.  Per RFC-002 §3 the
    /// whole batch is quarantined wholesale against this version — callers
    /// should cut over to the named `latest_stable`.  410 Gone.
    #[error(
        "Version {version} on contract {contract_id} is deprecated; latest stable is {latest_stable:?}"
    )]
    DeprecatedVersionPinned {
        contract_id: uuid::Uuid,
        version: String,
        latest_stable: Option<String>,
    },

    // -----------------------------------------------------------------------
    // Generic
    // -----------------------------------------------------------------------
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Free-form internal error, used for invariants broken in storage row
    /// parsing, bad enum values read back from the DB, etc.
    #[error("Internal error: {0}")]
    Internal(String),
}

// Preserve ergonomic `?` conversion from anyhow::Error (previously handled by
// a `#[from]` on the old `Internal` variant that took `anyhow::Error`).
impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(format!("{e:#}"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::ContractNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::InvalidContractYaml(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),

            AppError::VersionConflict { .. } => (StatusCode::CONFLICT, self.to_string()),
            AppError::VersionImmutable { .. } => (StatusCode::CONFLICT, self.to_string()),
            AppError::VersionNotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::InvalidStateTransition { .. } => (StatusCode::CONFLICT, self.to_string()),
            // RFC-002 §8: NoStableVersion → 409 ("publish one or pin a draft").
            AppError::NoStableVersion { .. } => (StatusCode::CONFLICT, self.to_string()),
            // RFC-002 §8: DeprecatedVersionPinned → 422 when it reaches the
            // generic error responder.  The ingest handler short-circuits and
            // writes the wholesale-quarantine audit path before this variant
            // is ever returned; this mapping is the belt-and-braces case
            // (e.g. if a future handler propagates it directly).
            AppError::DeprecatedVersionPinned { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
            }

            AppError::Database(e) => {
                tracing::error!("Database error: {:?}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".into())
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
        };

        let body = Json(json!({
            "error": message,
            "status": status.as_u16(),
        }));

        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
