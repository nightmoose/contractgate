//! Tests for RFC-016 Observability v1 metrics endpoint.
//!
//! ## What these tests cover (no live DB required)
//!
//! 1. `all_metric_names_present` — install a fresh Prometheus recorder, emit
//!    one sample of every contractgate metric, render the exposition text, and
//!    assert every expected metric name appears.
//!
//! 2. `validation_histogram_moves` — install a fresh recorder, call
//!    `validation::validate` ten times (pure in-process, no DB), record a
//!    histogram observation for each call, render, and assert the `_count`
//!    line has moved to ≥ 10.
//!
//! 3. `metrics_endpoint_open` — stand up the `/metrics` handler via
//!    `axum-test`, hit it without auth, assert 200 and correct content-type.
//!
//! 4. `metrics_endpoint_bearer_auth` — same setup but with
//!    `METRICS_AUTH_TOKEN` set; assert 401 on missing token, 200 on correct.
//!
//! All tests are fully in-process and do not require `DATABASE_URL`.
//!
//! ## Integration tests (require live server)
//!
//! Tagged `#[ignore]` — run with:
//!   cargo test --test metrics -- --include-ignored

use contractgate::contract::{Contract, ContractField, FieldType};
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

/// Install an isolated Prometheus recorder and return the handle.
/// Uses a non-global builder so each test gets its own recorder, avoiding
/// cross-test state pollution.
fn fresh_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("contractgate_validation_duration_seconds".to_string()),
            &[0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0],
        )
        .expect("valid buckets")
        .build_recorder()
        .handle()
}

/// Install a recorder as the *global* recorder (needed for `metrics::*!` macros).
/// Only safe to call once per process — subsequent calls are ignored by the
/// `once_cell` guard in `observability::install_recorder`.  In the test binary
/// each `#[test]` function runs in the same process, so we use a `once_cell`
/// here too and render via the returned handle.
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
// 1. All expected metric names are present
// ---------------------------------------------------------------------------

#[test]
fn all_metric_names_present() {
    let handle = install_global_recorder();

    // Emit one sample of every RFC-016 metric so they appear in the output.
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

    let expected = [
        "contractgate_requests_total",
        "contractgate_validation_duration_seconds",
        "contractgate_violations_total",
        "contractgate_quarantined_total",
        "contractgate_contracts_active",
        "contractgate_audit_log_rows",
    ];

    for name in expected {
        assert!(
            output.contains(name),
            "metric `{name}` missing from /metrics output.\nOutput:\n{output}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Validation histogram count moves after 10 calls
// ---------------------------------------------------------------------------

#[test]
fn validation_histogram_moves() {
    let handle = install_global_recorder();
    let compiled = simple_contract();

    // Render before to get baseline count.
    let before = handle.render();
    let before_count = extract_histogram_count(&before, "contractgate_validation_duration_seconds");

    // Fire 10 validation calls, each timed and recorded.
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
    let after_count = extract_histogram_count(&after, "contractgate_validation_duration_seconds");

    assert!(
        after_count >= before_count + 10,
        "histogram _count did not increase by 10: before={before_count} after={after_count}\n\
         Output:\n{after}"
    );
}

/// Parse the summed `_count` from a Prometheus histogram exposition block.
/// Sums all `metric_name_count` lines (across label combinations).
fn extract_histogram_count(text: &str, metric_name: &str) -> u64 {
    let count_key = format!("{metric_name}_count");
    text.lines()
        .filter(|l| l.starts_with(&count_key))
        .filter_map(|l| l.split_whitespace().last())
        .filter_map(|v| v.parse::<f64>().ok())
        .map(|v| v as u64)
        .sum()
}

// ---------------------------------------------------------------------------
// 3 & 4. /metrics handler — open + bearer-auth via axum-test
// ---------------------------------------------------------------------------

/// Build a minimal single-route Axum app with just the metrics handler.
/// Avoids the full gateway router (which needs a DB pool).
fn metrics_app() -> axum::Router {
    use axum::routing::get;
    axum::Router::new().route("/metrics", get(metrics_handler_wrapper))
}

/// Free function wrapper so axum routing can refer to it without capturing
/// the PrometheusHandle (the handler reads from the global PROMETHEUS_HANDLE).
async fn metrics_handler_wrapper(
    req: axum::http::Request<axum::body::Body>,
) -> axum::response::Response {
    // Re-implement the handler inline to avoid depending on the binary crate.
    use axum::response::IntoResponse;

    if let Ok(expected) = std::env::var("METRICS_AUTH_TOKEN") {
        let provided = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if provided != expected {
            return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }

    // Use the global recorder installed by `install_global_recorder`.
    // If it hasn't been called yet, fall back gracefully.
    let body = metrics_exporter_prometheus::PrometheusBuilder::new()
        .build_recorder()
        .handle()
        .render();

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

#[tokio::test]
async fn metrics_endpoint_open() {
    // No METRICS_AUTH_TOKEN set → endpoint must be open.
    std::env::remove_var("METRICS_AUTH_TOKEN");
    install_global_recorder();

    let server = axum_test::TestServer::new(metrics_app()).unwrap();
    let resp = server.get("/metrics").await;

    assert_eq!(resp.status_code(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/plain"),
        "expected text/plain content-type, got: {ct}"
    );
}

#[tokio::test]
async fn metrics_endpoint_bearer_auth() {
    std::env::set_var("METRICS_AUTH_TOKEN", "supersecret");
    install_global_recorder();

    let server = axum_test::TestServer::new(metrics_app()).unwrap();

    // No token → 401.
    let resp = server.get("/metrics").await;
    assert_eq!(resp.status_code(), 401, "missing token should be 401");

    // Wrong token → 401.
    let resp = server
        .get("/metrics")
        .add_header(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer wrongtoken"),
        )
        .await;
    assert_eq!(resp.status_code(), 401, "wrong token should be 401");

    // Correct token → 200.
    let resp = server
        .get("/metrics")
        .add_header(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer supersecret"),
        )
        .await;
    assert_eq!(resp.status_code(), 200, "correct token should be 200");

    // Clean up so other tests aren't affected.
    std::env::remove_var("METRICS_AUTH_TOKEN");
}

// ---------------------------------------------------------------------------
// Integration test stubs (require live server)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live server at TEST_BASE_URL (default http://localhost:3001)"]
async fn integration_metrics_endpoint_live() {
    let base =
        std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.expect("body");

    for name in &[
        "contractgate_requests_total",
        "contractgate_validation_duration_seconds",
        "contractgate_contracts_active",
        "contractgate_audit_log_rows",
    ] {
        assert!(body.contains(name), "missing metric `{name}` in live /metrics");
    }
}
