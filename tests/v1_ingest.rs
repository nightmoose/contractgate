//! Tests for the v1 bulk ingest endpoint (RFC-021).
//!
//! ## Unit tests (no DB, no server)
//!
//! These test pure functions exported from `v1_ingest.rs` and
//! `rate_limit.rs`: body parsing, size enforcement, rate-limit token-bucket
//! logic, and the OpenAPI spec shape.
//!
//! ## Integration tests
//!
//! Tagged `#[ignore]` — require a live database (`DATABASE_URL` env var).
//! Run with:
//!   cargo test --test v1_ingest -- --include-ignored
//!
//! ## Load test
//!
//! Separate script in `ops/load/v1_ingest.yaml` (oha/drill); see that file
//! for instructions.  Not encoded as a Rust test to avoid a mandatory
//! binary dependency in CI.

// ---------------------------------------------------------------------------
// Imports — unit tests only need the library types
// ---------------------------------------------------------------------------

#[cfg(test)]
mod unit {
    // Body-parsing helpers are `pub(crate)` so we test via the binary crate's
    // integration test harness.  For now, the pure-function tests live in
    // v1_ingest.rs's own `#[cfg(test)] mod tests` block and are exercised by
    // `cargo test --bin contractgate-server`.  This file adds the higher-level
    // integration and load stubs.

    /// Placeholder confirming this test file compiles and is wired into the
    /// test runner.  Remove once real integration tests are added below.
    #[test]
    fn placeholder_compiles() {}
}

// ---------------------------------------------------------------------------
// Integration tests (require DATABASE_URL + a running Postgres)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration {
    use serde_json::json;

    /// POST → reject → quarantine → replay round trip.
    ///
    /// 1. Submit one event that violates the contract.
    /// 2. Assert the response contains a non-null `quarantine_id`.
    /// 3. Call `POST /contracts/{id}/quarantine/replay` with that ID.
    /// 4. Assert the replay response reflects re-validation.
    #[tokio::test]
    #[ignore = "requires live DATABASE_URL and a seeded contract"]
    async fn post_reject_quarantine_replay_round_trip() {
        let base =
            std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
        let api_key = std::env::var("TEST_API_KEY").expect("TEST_API_KEY required");
        let contract_id = std::env::var("TEST_CONTRACT_ID").expect("TEST_CONTRACT_ID required");

        let client = reqwest::Client::new();

        // Step 1: submit a known-bad event.
        let bad_event = json!([{"user_id": null, "event_type": "purchase", "timestamp": 0}]);
        let resp = client
            .post(format!("{base}/v1/ingest/{contract_id}"))
            .header("x-api-key", &api_key)
            .header("content-type", "application/json")
            .json(&bad_event)
            .send()
            .await
            .expect("request failed");

        assert_eq!(resp.status(), 422);
        let body: serde_json::Value = resp.json().await.unwrap();

        // Step 2: quarantine_id must be present.
        let qid = body["results"][0]["quarantine_id"]
            .as_str()
            .expect("quarantine_id must be a string UUID for rejected events");
        assert!(!qid.is_empty());

        // Step 3: replay.
        let replay_resp = client
            .post(format!("{base}/contracts/{contract_id}/quarantine/replay"))
            .header("x-api-key", &api_key)
            .json(&json!({"quarantine_ids": [qid]}))
            .send()
            .await
            .expect("replay request failed");

        // Step 4: replay should succeed (or return a structured validation result).
        assert!(
            replay_resp.status().is_success() || replay_resp.status() == 422,
            "replay should return 200 or 422, got {}",
            replay_resp.status()
        );
    }

    /// Idempotency end-to-end: two identical requests with the same key.
    #[tokio::test]
    #[ignore = "requires live DATABASE_URL and a seeded contract"]
    async fn idempotency_replay_same_body() {
        let base =
            std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
        let api_key = std::env::var("TEST_API_KEY").expect("TEST_API_KEY required");
        let contract_id = std::env::var("TEST_CONTRACT_ID").expect("TEST_CONTRACT_ID required");

        let client = reqwest::Client::new();
        let idem_key = format!("test-idem-{}", uuid::Uuid::new_v4());
        let payload =
            json!([{"user_id": "u_test", "event_type": "login", "timestamp": 1_714_000_000}]);

        let send = || {
            client
                .post(format!("{base}/v1/ingest/{contract_id}"))
                .header("x-api-key", &api_key)
                .header("content-type", "application/json")
                .header("idempotency-key", &idem_key)
                .json(&payload)
        };

        let r1 = send().send().await.unwrap();
        let r2 = send().send().await.unwrap();

        assert_eq!(
            r1.status(),
            r2.status(),
            "both requests must return same status"
        );
        let replay_header = r2.headers().get("x-idempotency-replay");
        assert_eq!(
            replay_header.and_then(|v| v.to_str().ok()),
            Some("true"),
            "second request must carry X-Idempotency-Replay: true"
        );
    }

    /// Rate-limit burst: fire DEFAULT_BURST + 1 requests, expect 429 on last.
    ///
    /// Default burst = 1 000 (RFC-021).  Hardcoded here because `rate_limit`
    /// lives in the binary crate and is not importable from integration tests.
    #[tokio::test]
    #[ignore = "requires live DATABASE_URL; sends 1001 requests"]
    async fn rate_limit_burst_exceeded() {
        const DEFAULT_BURST: usize = 1_000; // mirrors rate_limit::DEFAULT_RATE_LIMIT_BURST

        let base =
            std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
        let api_key = std::env::var("TEST_API_KEY").expect("TEST_API_KEY required");
        let contract_id = std::env::var("TEST_CONTRACT_ID").expect("TEST_CONTRACT_ID required");

        let client = reqwest::Client::new();
        let payload = json!([{"user_id": "u1", "event_type": "login", "timestamp": 1}]);

        let mut got_429 = false;
        for _ in 0..=(DEFAULT_BURST) {
            let resp = client
                .post(format!("{base}/v1/ingest/{contract_id}"))
                .header("x-api-key", &api_key)
                .header("content-type", "application/json")
                .json(&payload)
                .send()
                .await
                .unwrap();
            if resp.status() == 429 {
                got_429 = true;
                break;
            }
        }
        assert!(
            got_429,
            "should have received 429 after exhausting burst capacity"
        );
    }
}
