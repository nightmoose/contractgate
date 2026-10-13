//! Application-level error types for DataContractGate.
//!
//! Uses `thiserror` to define structured errors that map cleanly to HTTP responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

use crate::rate_limit::RateLimitOutcome;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Contract not found: {0}")]
    ContractNotFound(uuid::Uuid),

    #[error("Invalid contract YAML: {0}")]
    InvalidContractYaml(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    /// RFC-021: request body or per-event size limit exceeded.  413.
    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Unauthorized: missing or invalid API key")]
    Unauthorized,

    /// P1-1: per-key token-bucket exhausted.  429.
    /// RFC-103: carries the `RateLimitOutcome` so `IntoResponse` can attach
    /// `X-RateLimit-*` + `Retry-After` headers on the 429.
    #[error("Rate limit exceeded — too many requests")]
    RateLimitExceeded(RateLimitOutcome),

    /// RFC-083 Phase 2: org monthly event plan cap reached.  429.
    #[error("Plan event limit exceeded for plan '{plan}' ({used}/{limit} in {period})")]
    PlanLimitExceeded {
        plan: String,
        limit: i64,
        used: i64,
        period: String,
    },

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
    // ODCS import
    // -----------------------------------------------------------------------
    /// Attempt to promote a version that was imported from a stripped ODCS
    /// document (no `x-contractgate-*` extensions).  PII transforms or
    /// validation constraints may be missing.  Clear the flag via
    /// `POST /contracts/{id}/versions/{version}/approve-import`.  409.
    #[error(
        "Version {version} was imported from ODCS without DataContractGate extensions and requires \
         review before promotion. Use \
         POST /contracts/{contract_id}/versions/{version}/approve-import to clear this flag."
    )]
    OdcsReviewRequired {
        contract_id: uuid::Uuid,
        version: String,
    },

    // -----------------------------------------------------------------------
    // Generic
    // -----------------------------------------------------------------------
    /// Generic 404 for resource types that don't have a dedicated variant.
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Free-form internal error, used for invariants broken in storage row
    /// parsing, bad enum values read back from the DB, etc.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Upstream HTTP fetch timed out (infer/url, public-catalog export).  504.
    #[error("Gateway timeout: {0}")]
    GatewayTimeout(String),

    /// Response is structurally valid but contains no inferable records.  422.
    #[error("Unprocessable: {0}")]
    UnprocessableEntity(String),
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
            AppError::InvalidContractYaml(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
            }
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            // RFC-103: emit the IETF-draft rate-limit headers + RFC 6585
            // Retry-After so clients can back off intelligently.
            AppError::RateLimitExceeded(outcome) => {
                let body = Json(json!({
                    "error": self.to_string(),
                    "status": 429,
                    "retry_after_ms": outcome.retry_after_ms,
                }));
                return (StatusCode::TOO_MANY_REQUESTS, outcome.headers(), body).into_response();
            }
            // Plan-limit body is richer; handled specially below.
            AppError::PlanLimitExceeded { .. } => {
                let body = match &self {
                    AppError::PlanLimitExceeded {
                        plan,
                        limit,
                        used,
                        period,
                    } => Json(json!({
                        "error": "plan_limit_exceeded",
                        "detail": self.to_string(),
                        "plan": plan,
                        "limit": limit,
                        "used": used,
                        "period": period,
                        "upgrade_url": "https://app.datacontractgate.com/pricing",
                        "status": 429,
                    })),
                    _ => unreachable!(),
                };
                return (StatusCode::TOO_MANY_REQUESTS, body).into_response();
            }

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
            AppError::OdcsReviewRequired { .. } => (StatusCode::CONFLICT, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),

            AppError::Database(e) => {
                tracing::error!("Database error: {:?}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".into())
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            AppError::GatewayTimeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
            AppError::UnprocessableEntity(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
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

// ---------------------------------------------------------------------------
// DB error context helper
// ---------------------------------------------------------------------------

/// Extension trait to attach an operation label to a `sqlx::Error` before it
/// is propagated as `AppError::Database`.  Bare `?` on a `sqlx::Result`
/// preserves the underlying SQL detail (constraint, query, etc.) but loses
/// the application context — operators reading prod logs see "Database
/// error" without knowing whether it came from a contract create, an ingest
/// read, or an audit write.
///
/// Use:
///
/// ```ignore
/// let row = sqlx::query_as::<_, Row>("...")
///     .fetch_one(pool)
///     .await
///     .db_op("get_contract_identity")?;
/// ```
///
/// The HTTP status mapping is unchanged (still `AppError::Database` → 500);
/// only the `tracing::error!` line in the logs gains the operation label.
pub trait DbOpContext<T> {
    fn db_op(self, op: &'static str) -> AppResult<T>;
}

impl<T> DbOpContext<T> for Result<T, sqlx::Error> {
    fn db_op(self, op: &'static str) -> AppResult<T> {
        self.map_err(|e| {
            tracing::error!("Database error during {}: {:?}", op, e);
            AppError::Database(e)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    /// RFC-103: every `AppError::RateLimitExceeded` 429 must carry the full
    /// IETF-draft `X-RateLimit-*` trio **and** an RFC 6585 `Retry-After` in
    /// seconds, so clients can back off intelligently. Regression guard
    /// against the pre-RFC-103 bug where the middleware path discarded the
    /// `RateLimitOutcome` and emitted a header-less 429.
    #[tokio::test]
    async fn rate_limit_exceeded_response_has_all_four_headers() {
        let outcome = RateLimitOutcome {
            allowed: false,
            limit: 10,
            remaining: 0,
            reset_unix: 1_700_000_000,
            retry_after_ms: 1_250,
        };
        let resp = AppError::RateLimitExceeded(outcome).into_response();

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let headers = resp.headers();
        assert_eq!(headers.get("X-RateLimit-Limit").unwrap(), "10");
        assert_eq!(headers.get("X-RateLimit-Remaining").unwrap(), "0");
        assert_eq!(headers.get("X-RateLimit-Reset").unwrap(), "1700000000");
        // 1250 ms → ceil to 2 seconds, per RFC 6585.
        assert_eq!(headers.get("Retry-After").unwrap(), "2");

        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["status"], 429);
        assert_eq!(parsed["retry_after_ms"], 1_250);
    }
}
