//! Tests for RFC-016 Observability v1 metrics endpoint.
//!
//! ## What these tests cover (no live DB required)
//!
//! 1. `all_metric_names_present` — install a global Prometheus recorder, emit
//!    one sample of every contractgate metric, render, assert every expected
//!    name appears.
//!
//! 2. `validation_histogram_moves` — call `validation::validate` ten times
//!    in-process, record a histogram observation each time, assert the
//!    `_count` line increases by ≥ 10.
//!
//! 3. `metrics_endpoint_open` — spin up the `/metrics` handler via
//!    `axum-test`, assert 200 + `text/plain` content-type (no auth).
//!
//! 4. `metrics_endpoint_bearer_auth` — assert 401 on missing/wrong token,
//!    200 on correct `METRICS_AUTH_TOKEN`.
//!
//! None of these require `DATABASE_URL`.
//!
//! ## Integration tests (require live server)
//!
//! Tagged `#[ignore]`; run with:
//!   cargo test --test metrics -- --include-ignored

use contractgate::contract::Contract;
use contractgate::validation::{validate, CompiledContract};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use serde_json::json;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a tiny compiled contract for in-process validation tests.
fn simple_contract() -> CompiledContract {
    let raw = r#"
version: "1.0"
name: "test_contract"
description: "used by metrics tests"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
    - name: amount
      type: number
      required: false
"#;
    let parsed: Contract = serde_yaml::from_str(raw).expect("valid contract yaml");
    CompiledContract::compile(parsed).expect("compiled ok")
}

/// Install a recorder as the *global* recorder (required for `metrics::*!` macros).
///
/// Uses `OnceLock` so repeated calls from different tests in the same process
/// are no-ops that return the same handle.
fn install_global_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    static HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Full("contractgate_validation_duration_seconds".to_string()),
                    &[0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0],
                )
                .expect("valid buckets")
                .install_recorder()
                .expect("install global recorder")
        })
        .clone()
}

// ---------------------------------------------------------------------------
// 1. All expected metric names present
// ---------------------------------------------------------------------------

#[test]
fn all_metric_names_present() {
    let handle = install_global_recorder();

    metrics::counter!("contractgate_requests_total",
        "route" => "/health", "method" => "GET", "status" => "200"
    ).increment(1);

    metrics::histogram!("contractgate_validation_duration_seconds",
        "contract_id" => "test-id", "outcome" => "passed"
    ).record(0.002);

    metrics::counter!("contractgate_violations_total",
        "contract_id" => "test-id", "kind" => "missing_required_field"
    ).increment(1);

    metrics::counter!("contractgate_quarantined_total",
        "contract_id" => "test-id"
    ).increment(1);

    metrics::gauge!("contractgate_contracts_active").set(5.0);
    metrics::gauge!("contractgate_audit_log_rows").set(42.0);

    let output = handle.render();

    for name in [
        "contractgate_requests_total",
        "contractgate_validation_duration_seconds",
        "contractgate_violations_total",
        "contractgate_quarantined_total",
        "contractgate_contracts_active",
        "contractgate_audit_log_rows",
    ] {
        assert!(
            output.contains(name),
            "metric `{name}` missing from /metrics output.\nOutput:\n{output}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Histogram count moves after 10 validation calls
// ---------------------------------------------------------------------------

#[test]
fn validation_histogram_moves() {
    let handle = install_global_recorder();
    let compiled = simple_contract();

    let before = handle.render();
    let before_count =
        extract_histogram_count(&before, "contractgate_validation_duration_seconds");

    let good_event = json!({ "user_id": "u1", "amount": 9.99 });
    for _ in 0..10 {
        let t = Instant::now();
        let result = validate(&compiled, &good_event);
        let elapsed = t.elapsed().as_secs_f64();
        let outcome = if result.passed { "passed" } else { "failed" };
        metrics::histogram!(
            "contractgate_validation_duration_seconds",
            "contract_id" => "histogram-test",
            "outcome" => outcome,
        )
        .record(elapsed);
    }

    let after = handle.render();
    let after_count =
        extract_histogram_count(&after, "contractgate_validation_duration_seconds");

    assert!(
        after_count >= before_count + 10,
        "histogram _count did not increase by ≥10: before={before_count} after={after_count}"
    );
}

/// Sum all `<metric_name>_count` lines (across label combos) in the text.
fn extract_histogram_count(text: &str, metric_name: &str) -> u64 {
    let key = format!("{metric_name}_count");
    text.lines()
        .filter(|l| l.starts_with(&key))
        .filter_map(|l| l.split_whitespace().last())
        .filter_map(|v| v.parse::<f64>().ok())
        .map(|v| v as u64)
        .sum()
}

// ---------------------------------------------------------------------------
// Minimal metrics-only Axum app (no DB needed)
// ---------------------------------------------------------------------------

fn metrics_app() -> axum::Router {
    axum::Router::new().route("/metrics", axum::routing::get(metrics_handler_test))
}

/// Inline re-implementation of the metrics handler for tests.
/// Cannot import from the binary crate; mirrors the logic in observability.rs.
async fn metrics_handler_test(req: axum::http::Request<axum::body::Body>) -> axum::response::Response {
    use axum::response::IntoResponse;

    if let Ok(expected) = std::env::var("METRICS_AUTH_TOKEN") {
        let provided = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if provided != expected {
            return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }

    // Render via the global recorder installed by install_global_recorder().
    // If tests run in isolation and install_global_recorder() hasn't been
    // called, build a fresh recorder just for this render (output will be empty
    // but the endpoint will still return 200).
    let body = install_global_recorder().render();

    (
        axum::http::StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// 3. /metrics endpoint — open (no auth)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_endpoint_open() {
    std::env::remove_var("METRICS_AUTH_TOKEN");
    install_global_recorder();

    // axum-test v20: TestServer::new returns TestServer directly (not Result).
    let server = axum_test::TestServer::new(metrics_app());
    let resp: axum_test::TestResponse = server.get("/metrics").await;

    assert_eq!(resp.status_code(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/plain"), "expected text/plain, got: {ct}");
}

// ---------------------------------------------------------------------------
// 4. /metrics endpoint — bearer-auth gating
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_endpoint_bearer_auth() {
    std::env::set_var("METRICS_AUTH_TOKEN", "supersecret");
    install_global_recorder();

    let server = axum_test::TestServer::new(metrics_app());

    // No token → 401.
    let resp: axum_test::TestResponse = server.get("/metrics").await;
    assert_eq!(resp.status_code(), 401, "missing token should be 401");

    // Wrong token → 401.
    let resp: axum_test::TestResponse = server
        .get("/metrics")
        .add_header(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer wrongtoken"),
        )
        .await;
    assert_eq!(resp.status_code(), 401, "wrong token should be 401");

    // Correct token → 200.
    let resp: axum_test::TestResponse = server
        .get("/metrics")
        .add_header(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer supersecret"),
        )
        .await;
    assert_eq!(resp.status_code(), 200, "correct token should be 200");

    std::env::remove_var("METRICS_AUTH_TOKEN");
}

// ---------------------------------------------------------------------------
// Integration test stubs (require live server at TEST_BASE_URL)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live server at TEST_BASE_URL (default http://localhost:3001)"]
async fn integration_metrics_endpoint_live() {
    let base =
        std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
    let resp = reqwest::Client::new()
        .get(format!("{base}/metrics"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.expect("body");

    for name in [
        "contractgate_requests_total",
        "contractgate_validation_duration_seconds",
        "contractgate_contracts_active",
        "contractgate_audit_log_rows",
    ] {
        assert!(body.contains(name), "missing metric `{name}` in live /metrics");
    }
}
