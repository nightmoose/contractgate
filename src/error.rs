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

    /// Request body declared a media type the route does not read.  415.
    #[error("Unsupported media type: {0}")]
    UnsupportedMediaType(String),

    /// RFC-021: request body or per-event size limit exceeded.  413.
    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Unauthorized: missing or invalid API key")]
    Unauthorized,

    /// An API key sent as `Authorization: Bearer cg_...`. Bearer is for
    /// dashboard session tokens; say where the key goes instead of the
    /// generic 401.
    #[error("Unauthorized: send API keys in the x-api-key header, not Authorization: Bearer")]
    ApiKeyInBearer,

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

    /// Request conflicts with the resource's current state.  409.
    #[error("{0}")]
    Conflict(String),

    /// Authenticated, but not allowed to act on this resource.  403.
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// Link/org-visibility publication fetched or imported without the right
    /// `link_token` (S-P2). 403, distinct from the API-key 401.
    #[error("{0}")]
    LinkToken(String),

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
            AppError::UnsupportedMediaType(_) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, self.to_string())
            }
            AppError::Unauthorized | AppError::ApiKeyInBearer => {
                (StatusCode::UNAUTHORIZED, self.to_string())
            }
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
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            AppError::LinkToken(_) => (StatusCode::FORBIDDEN, self.to_string()),

            // An unknown org id (only reachable via the dev/no-auth
            // `x-org-id` header) is a client error, not a 500.
            AppError::Database(e)
                if e.as_database_error().and_then(|d| d.constraint())
                    == Some("contracts_org_id_fkey") =>
            {
                (
                    StatusCode::BAD_REQUEST,
                    "unknown org: the `x-org-id` org does not exist".into(),
                )
            }
            AppError::Database(e) => database_failure(e),
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

        let body = if matches!(self, AppError::DeprecatedVersionPinned { .. }) {
            Json(json!({
                "error": message,
                "status": status.as_u16(),
                "code": "contract_version_deprecated",
            }))
        } else {
            Json(json!({
                "error": message,
                "status": status.as_u16(),
            }))
        };

        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ---------------------------------------------------------------------------
// DB error context helper
// ---------------------------------------------------------------------------

/// Pool, lock, and statement timeouts are 504 and log the route plus the
/// stage. Every other database error stays a 500.
fn database_failure(e: &sqlx::Error) -> (StatusCode, String) {
    if let Some(wait) = crate::request_limits::db_timeout(e) {
        tracing::error!(
            route = %crate::request_limits::current_route(),
            stage = wait.stage,
            error = %e,
            "database wait timed out"
        );
        return (StatusCode::GATEWAY_TIMEOUT, wait.message.to_string());
    }
    tracing::error!(
        route = %crate::request_limits::current_route(),
        stage = %crate::request_limits::current_stage(),
        "Database error: {:?}",
        e
    );
    (StatusCode::INTERNAL_SERVER_ERROR, database_error_message(e))
}

fn database_error_message(e: &sqlx::Error) -> String {
    let Some(db) = e.as_database_error() else {
        return "Database error".into();
    };
    let kind = match db.code().as_deref() {
        Some("23503") => "a referenced record is missing (foreign key violation)",
        Some("23505") => "a record with the same key already exists (unique violation)",
        Some("23514") => "a value failed a database check (check violation)",
        Some("23502") => "a required value is missing (not-null violation)",
        _ => return "Database error".into(),
    };
    match db.constraint() {
        Some(c) => format!("Database error: {kind} on constraint {c}"),
        None => format!("Database error: {kind}"),
    }
}

/// Attach an operation label to a `sqlx::Error` before it becomes
/// `AppError::Database`. The error line includes the operation, the route,
/// and the stage (`pool_acquire`, `lock_wait`, `db_statement`, or `handler`).
///
/// ```ignore
/// let row = sqlx::query_as::<_, Row>("...")
///     .fetch_one(pool)
///     .await
///     .db_op("get_contract_identity")?;
/// ```
pub trait DbOpContext<T> {
    fn db_op(self, op: &'static str) -> AppResult<T>;
}

impl<T> DbOpContext<T> for Result<T, sqlx::Error> {
    fn db_op(self, op: &'static str) -> AppResult<T> {
        self.map_err(|e| {
            let stage = crate::request_limits::db_timeout(&e)
                .map(|wait| wait.stage)
                .unwrap_or_else(crate::request_limits::current_stage);
            tracing::error!(
                route = %crate::request_limits::current_route(),
                stage,
                "Database error during {op}: {e:?}"
            );
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
            rate: 10,
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

    #[tokio::test]
    async fn pool_acquire_timeout_is_504() {
        let resp = AppError::Database(sqlx::Error::PoolTimedOut).into_response();
        assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["status"], 504);
        assert_eq!(parsed["error"], "timed out acquiring a database connection");
    }
}

// ---------------------------------------------------------------------------
// Uniform JSON error bodies
// ---------------------------------------------------------------------------

/// Largest non-JSON error body copied into the JSON `error` message.
const PLAIN_ERROR_BODY_MAX: usize = 4 * 1024;

/// Middleware: give every error response the `{"error": ..., "status": N}`
/// shape that `AppError` produces.
///
/// Errors raised outside our handlers bypass `AppError`: tower-http's body
/// limit answers 413 with plain text ("length limit exceeded"), axum's
/// extractors answer 400/415 with plain text, and unmatched routes answer
/// 404/405 with an empty body (E2E retest 2026-10-13). Responses that
/// already carry JSON, and every non-error response, pass through untouched.
pub async fn json_error_bodies(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let resp = next.run(req).await;

    let status = resp.status();
    let is_json = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !(status.is_client_error() || status.is_server_error()) || is_json {
        return resp;
    }

    let (mut parts, body) = resp.into_parts();
    let text = axum::body::to_bytes(body, PLAIN_ERROR_BODY_MAX)
        .await
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|s| !s.is_empty());
    let message = match (status, text) {
        (_, Some(text)) => text,
        (StatusCode::NOT_FOUND, None) => format!("No route for {method} {path}"),
        (StatusCode::METHOD_NOT_ALLOWED, None) => {
            format!("Method {method} is not allowed on {path}")
        }
        (_, None) => status
            .canonical_reason()
            .unwrap_or("Request failed")
            .to_string(),
    };

    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    let body = json!({ "error": message, "status": status.as_u16() }).to_string();
    Response::from_parts(parts, axum::body::Body::from(body))
}

#[cfg(test)]
mod json_error_body_tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::post, Router};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/echo", post(|body: String| async move { body }))
            .route(
                "/json-err",
                post(|| async { AppError::BadRequest("bad".into()) }),
            )
            .layer(tower_http::limit::RequestBodyLimitLayer::new(16))
            .layer(axum::middleware::from_fn(json_error_bodies))
    }

    async fn call(req: Request<Body>) -> (StatusCode, String, serde_json::Value) {
        let resp = app().oneshot(req).await.unwrap();
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            ct,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn body_limit_413_becomes_json() {
        let req = Request::post("/echo")
            .body(Body::from("x".repeat(64)))
            .unwrap();
        let (status, ct, body) = call(req).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(ct, "application/json");
        assert_eq!(body["status"], 413);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("length limit exceeded"));
    }

    #[tokio::test]
    async fn unknown_route_404_becomes_json() {
        let req = Request::get("/nope/").body(Body::empty()).unwrap();
        let (status, ct, body) = call(req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(ct, "application/json");
        assert_eq!(
            body,
            json!({"error": "No route for GET /nope/", "status": 404})
        );
    }

    #[tokio::test]
    async fn wrong_method_405_becomes_json() {
        let req = Request::get("/echo").body(Body::empty()).unwrap();
        let (status, _, body) = call(req).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["status"], 405);
    }

    #[tokio::test]
    async fn json_errors_and_successes_pass_through() {
        let (status, _, body) = call(Request::post("/json-err").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "Invalid request: bad");

        let resp = app()
            .oneshot(Request::post("/echo").body(Body::from("hi")).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"hi");
    }
}

// ---------------------------------------------------------------------------
// Legacy API host redirect
// ---------------------------------------------------------------------------

/// Hosts (comma-separated, e.g. `contractgate-api.fly.dev`) that should stop
/// serving the API and 308-redirect to [`CANONICAL_API_ORIGIN_ENV`] instead.
/// Unset (the default) disables the redirect, so it can be switched on with a
/// Fly secret once every client has moved off the old host.
pub const LEGACY_API_HOSTS_ENV: &str = "LEGACY_API_HOSTS";
/// Where legacy-host requests go. Defaults to the public API origin.
pub const CANONICAL_API_ORIGIN_ENV: &str = "CANONICAL_API_ORIGIN";

/// Middleware: 308 requests that arrive on a legacy host to the same path and
/// query on the canonical origin. 308 keeps the method and body, so old POST
/// clients that follow redirects keep working. `/health` and `/ready` stay
/// served on every host for platform probes.
pub async fn redirect_legacy_hosts(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Ok(hosts) = std::env::var(LEGACY_API_HOSTS_ENV) else {
        return next.run(req).await;
    };
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase());
    let is_legacy = host.is_some_and(|h| {
        hosts
            .split(',')
            .map(str::trim)
            .any(|l| !l.is_empty() && l.eq_ignore_ascii_case(&h))
    });
    // R-S3: /health and /ready redirect too on the legacy *public* host (they
    // returned 200 + ACAO * there). Fly's checks (fly.toml `[checks.ready]`)
    // hit the machine's internal address, whose Host is never the legacy
    // name, so they are unaffected.
    if !is_legacy {
        return next.run(req).await;
    }
    let origin = std::env::var(CANONICAL_API_ORIGIN_ENV)
        .unwrap_or_else(|_| "https://api.datacontractgate.com".to_string());
    let target = format!(
        "{}{}",
        origin.trim_end_matches('/'),
        req.uri()
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
    );
    (
        StatusCode::PERMANENT_REDIRECT,
        [(axum::http::header::LOCATION, target.clone())],
        Json(json!({
            "error": format!("This host is retired; use {origin}"),
            "status": 308,
            "location": target,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod legacy_host_tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use tower::ServiceExt;

    // One test so the process-wide env var isn't raced by parallel tests.
    #[tokio::test]
    async fn legacy_host_redirects_except_probes_and_only_when_configured() {
        let app = || {
            Router::new()
                .route("/health", get(|| async { "ok" }))
                .route("/contracts", get(|| async { "list" }))
                .layer(axum::middleware::from_fn(redirect_legacy_hosts))
        };
        let req = |host: &str, path: &str| {
            Request::get(path)
                .header("host", host)
                .body(Body::empty())
                .unwrap()
        };

        std::env::remove_var(LEGACY_API_HOSTS_ENV);
        let r = app()
            .oneshot(req("contractgate-api.fly.dev", "/contracts"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "off unless configured");

        std::env::set_var(LEGACY_API_HOSTS_ENV, "contractgate-api.fly.dev");
        std::env::remove_var(CANONICAL_API_ORIGIN_ENV);
        let r = app()
            .oneshot(req("contractgate-api.fly.dev:443", "/contracts?limit=5"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            r.headers()["location"],
            "https://api.datacontractgate.com/contracts?limit=5"
        );

        let r = app()
            .oneshot(req("contractgate-api.fly.dev", "/health"))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            StatusCode::PERMANENT_REDIRECT,
            "legacy public host redirects probes too (R-S3)"
        );

        // Fly's internal health check: Host is the machine address.
        let r = app()
            .oneshot(req("172.19.0.2:3001", "/health"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "internal checks stay served");

        let r = app()
            .oneshot(req("api.datacontractgate.com", "/contracts"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "canonical host untouched");
        std::env::remove_var(LEGACY_API_HOSTS_ENV);
    }
}
