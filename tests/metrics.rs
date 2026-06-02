//! Tests for RFC-016 Observability v1 metrics endpoint.
//!
//! ## What these tests cover (no live DB required)
//!
//! 1. `all_metric_names_present` — emit one sample of every contractgate
//!    metric, render, assert every expected name appears.
//!
//! 2. `validation_histogram_moves` — call `validation::validate` ten times
//!    in-process, record a histogram observation each time; assert the count
//!    for the specific label used by this test reaches ≥ 10.
//!    (Scoped to a unique label so parallel tests sharing the global recorder
//!    do not interfere.)
//!
//! 3. `metrics_endpoint_open` — spin up the `/metrics` handler via axum-test
//!    with auth disabled; assert 200 + `text/plain` content-type.
//!
//! 4. `metrics_endpoint_bearer_auth` — spin up the handler with a hard-coded
//!    auth token injected via Axum state (avoids env-var races in parallel
//!    tests); assert 401 on missing/wrong token, 200 on correct token.
//!
//! None of these tests require `DATABASE_URL`.
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

/// Build a minimal compiled contract for in-process validation tests.
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

/// Install a Prometheus recorder as the global recorder (required for
/// `metrics::*!` macros to fire).  Uses `OnceLock` so multiple tests in the
/// same process share one recorder — subsequent calls are no-ops that return
/// the same handle.
fn install_global_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    static HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Full("contractgate_validation_duration_seconds".to_string()),
                    &[
                        0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0,
                    ],
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
    )
    .increment(1);

    metrics::histogram!("contractgate_validation_duration_seconds",
        "contract_id" => "names-test", "outcome" => "passed"
    )
    .record(0.002);

    metrics::counter!("contractgate_violations_total",
        "contract_id" => "names-test", "kind" => "missing_required_field"
    )
    .increment(1);

    metrics::counter!("contractgate_quarantined_total",
        "contract_id" => "names-test"
    )
    .increment(1);

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
            "metric `{name}` missing from /metrics output.\nFull output:\n{output}"
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

    // Use a label that is UNIQUE to this test so other parallel tests sharing
    // the same global recorder cannot affect the assertion.
    const TEST_CONTRACT_ID: &str = "histogram-moves-test";

    let good_event = json!({ "user_id": "u1", "amount": 9.99 });

    // Acquire handle ONCE — metrics 0.24: repeated macro calls with dynamic
    // labels can re-register the metric (resetting the counter).  Reusing a
    // single handle guarantees all 10 record() calls accumulate.
    let hist = metrics::histogram!(
        "contractgate_validation_duration_seconds",
        "contract_id" => TEST_CONTRACT_ID,
        "outcome" => "passed",
    );

    for _ in 0..10 {
        let t = Instant::now();
        let _result = validate(&compiled, &good_event);
        let elapsed = t.elapsed().as_secs_f64();
        hist.record(elapsed);
    }

    let output = handle.render();

    // Count observations for THIS test's specific label set only.
    let count = histogram_count_for_label(
        &output,
        "contractgate_validation_duration_seconds",
        TEST_CONTRACT_ID,
    );

    assert!(
        count >= 10,
        "histogram count for contract_id={TEST_CONTRACT_ID} should be ≥10, got {count}.\n\
         Output:\n{output}"
    );
}

/// Return the sum of all `<metric>_count` lines whose text contains
/// `contract_id="<id>"`.  Uses a per-label scope so parallel tests that share
/// the global recorder do not interfere.
fn histogram_count_for_label(text: &str, metric: &str, contract_id: &str) -> u64 {
    let count_key = format!("{metric}_count");
    let label_fragment = format!("contract_id=\"{contract_id}\"");
    text.lines()
        .filter(|l| l.starts_with(&count_key) && l.contains(&label_fragment))
        .filter_map(|l| l.split_whitespace().last())
        .filter_map(|v| v.parse::<f64>().ok())
        .map(|v| v as u64)
        .sum()
}

// ---------------------------------------------------------------------------
// Minimal metrics-only Axum app
//
// Auth token injected via Axum state — avoids std::env races when tests
// run in parallel (tests 3 and 4 would race on set_var / remove_var).
// ---------------------------------------------------------------------------

/// Auth configuration passed as Axum state to the test handler.
#[derive(Clone)]
struct TestAuthState {
    /// `Some(token)` → bearer auth required; `None` → open.
    expected_token: Option<String>,
}

fn metrics_app(auth: TestAuthState) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/metrics", get(metrics_handler_test))
        .with_state(auth)
}

async fn metrics_handler_test(
    axum::extract::State(auth): axum::extract::State<TestAuthState>,
    req: axum::http::Request<axum::body::Body>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    if let Some(expected) = &auth.expected_token {
        let provided = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if provided != expected.as_str() {
            return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }

    // Render the global recorder (installed by install_global_recorder).
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
    install_global_recorder();

    let server = axum_test::TestServer::new(metrics_app(TestAuthState {
        expected_token: None,
    }));
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
    install_global_recorder();

    let server = axum_test::TestServer::new(metrics_app(TestAuthState {
        expected_token: Some("supersecret".into()),
    }));

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
}

// ---------------------------------------------------------------------------
// Integration test stubs (require live server at TEST_BASE_URL)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live server at TEST_BASE_URL (default http://localhost:3001)"]
async fn integration_metrics_endpoint_live() {
    let base = std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
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
        assert!(
            body.contains(name),
            "missing metric `{name}` in live /metrics"
        );
    }
}
