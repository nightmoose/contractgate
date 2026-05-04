//! Observability v1 — Prometheus metrics (RFC-016).
//!
//! # What lives here
//!
//! - [`install_recorder`]: install the global Prometheus recorder once at
//!   startup.  Must be called before any `metrics::counter!` / `histogram!`
//!   / `gauge!` macro invocations.
//!
//! - [`metrics_handler`]: Axum handler for `GET /metrics`.  Returns the
//!   Prometheus text exposition format.  Bearer-auth gated only when the
//!   `METRICS_AUTH_TOKEN` env var is set; otherwise open (RFC-016 §Q3/Q4).
//!
//! - [`track_requests`]: Axum middleware that increments
//!   `contractgate_requests_total{route,method,status}` after every response.
//!
//! - [`spawn_gauge_tasks`]: spawns two background Tokio tasks that periodically
//!   refresh the `contractgate_contracts_active` (30s) and
//!   `contractgate_audit_log_rows` (60s) gauges by querying the DB.
//!
//! # Histogram buckets (RFC-016 §Decisions Q2)
//!
//! 0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0 (sec).
//! Tight resolution under the 25ms validation budget so p95/p99 are accurate.

use axum::{
    extract::MatchedPath,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;
use sqlx::PgPool;
use tokio::time::{interval, Duration};

/// Histogram buckets (seconds) — RFC-016 §Decisions Q2.
const VALIDATION_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0,
];

/// Global handle to the installed Prometheus recorder.
/// Initialised exactly once by [`install_recorder`].
static PROMETHEUS_HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

// ---------------------------------------------------------------------------
// Recorder install
// ---------------------------------------------------------------------------

/// Install the global Prometheus recorder.  Must be called once before the
/// Axum server starts accepting connections.  Subsequent calls are no-ops.
pub fn install_recorder() {
    PROMETHEUS_HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Full(
                    "contractgate_validation_duration_seconds".to_string(),
                ),
                VALIDATION_BUCKETS,
            )
            .expect("valid bucket values")
            .install_recorder()
            .expect("failed to install Prometheus recorder")
    });
}

/// Return the handle installed by [`install_recorder`], or `None` if it was
/// never called (unit tests that skip recorder install).
pub fn handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

// ---------------------------------------------------------------------------
// /metrics HTTP handler
// ---------------------------------------------------------------------------

/// Axum handler: `GET /metrics`.
///
/// Auth rule (RFC-016 §Q3/Q4):
/// - If `METRICS_AUTH_TOKEN` env var is set, the request must carry
///   `Authorization: Bearer <token>`.
/// - Otherwise the endpoint is open (allows Prometheus to scrape without
///   per-org credentials).
pub async fn metrics_handler(req: Request<axum::body::Body>) -> Response {
    // Bearer auth gate — only active when the env var is set.
    if let Ok(expected) = std::env::var("METRICS_AUTH_TOKEN") {
        let provided = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");

        if provided != expected {
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }

    match PROMETHEUS_HANDLE.get() {
        Some(handle) => {
            let body = handle.render();
            (
                StatusCode::OK,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; version=0.0.4; charset=utf-8",
                )],
                body,
            )
                .into_response()
        }
        None => {
            tracing::warn!("metrics_handler called before install_recorder");
            (StatusCode::SERVICE_UNAVAILABLE, "metrics not initialised").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Request-tracking middleware
// ---------------------------------------------------------------------------

/// Axum middleware layer: increments `contractgate_requests_total` after
/// every response.
///
/// Labels:
/// - `route`  — Axum matched-path pattern (e.g. `/ingest/{raw_id}`), or the URI path
/// - `method` — HTTP method string (`GET`, `POST`, …).
/// - `status` — HTTP status code as a string (`200`, `404`, …).
pub async fn track_requests(req: Request<axum::body::Body>, next: Next) -> Response {
    // Capture route + method *before* consuming the request.
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());
    let method = req.method().as_str().to_owned();

    let resp = next.run(req).await;

    let status = resp.status().as_u16().to_string();

    metrics::counter!(
        "contractgate_requests_total",
        "route" => route,
        "method" => method,
        "status" => status,
    )
    .increment(1);

    resp
}

// ---------------------------------------------------------------------------
// Background gauge-refresh tasks (RFC-016 §Decisions Q5)
// ---------------------------------------------------------------------------

/// Spawn two long-running Tokio tasks that periodically refresh gauges:
///
/// - `contractgate_contracts_active` — count of non-soft-deleted contracts,
///   refreshed every **30 seconds**.
/// - `contractgate_audit_log_rows`   — total row count in `audit_log`,
///   refreshed every **60 seconds**.
///
/// Both queries are intentionally simple aggregate `COUNT(*)` calls that do
/// not touch `org_memberships`, so there is no risk of the PG 42P17
/// recursion (see `feedback_rls_helper_required.md`).
pub fn spawn_gauge_tasks(db: PgPool) {
    // --- contracts_active (30s) ---
    let db_30 = db.clone();
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            match sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM contracts WHERE deleted_at IS NULL",
            )
            .fetch_one(&db_30)
            .await
            {
                Ok(n) => {
                    metrics::gauge!("contractgate_contracts_active").set(n as f64);
                }
                Err(e) => {
                    tracing::warn!("gauge refresh (contracts_active) failed: {e}");
                }
            }
        }
    });

    // --- audit_log_rows (60s) ---
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_log")
                .fetch_one(&db)
                .await
            {
                Ok(n) => {
                    metrics::gauge!("contractgate_audit_log_rows").set(n as f64);
                }
                Err(e) => {
                    tracing::warn!("gauge refresh (audit_log_rows) failed: {e}");
                }
            }
        }
    });
}
