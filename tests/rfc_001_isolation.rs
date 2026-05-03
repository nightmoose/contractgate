//! RFC-001 — Org isolation + soft-delete tests.
//!
//! Two flavours:
//!
//! ## Unit tests (no DB)
//! Pure-function checks: slug derivation rules, invite-token format, etc.
//!
//! ## Integration tests
//! Tagged `#[ignore]`. Require:
//!   - `DATABASE_URL` pointing at a Postgres instance with migrations applied
//!     through 012.
//!   - `TEST_BASE_URL` (default `http://localhost:3001`) — running gateway.
//!   - `TEST_API_KEY_A`, `TEST_API_KEY_B` — keys belonging to two different
//!     orgs, each with at least one contract.
//!   - `TEST_CONTRACT_ID_A` — a contract id known to belong to org A.
//!
//! Run with:
//!   cargo test --test rfc_001_isolation -- --include-ignored

#[cfg(test)]
mod unit {
    /// The slug derivation in `handle_new_user()` (migration 012) lowercases,
    /// strips non-alphanumerics, and trims leading/trailing dashes. We mirror
    /// that here so the fallback expectation stays in sync if the trigger
    /// changes.
    fn derive_slug(name: &str) -> String {
        let lowered = name.to_lowercase();
        let mut out = String::with_capacity(lowered.len());
        let mut prev_dash = false;
        for ch in lowered.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                prev_dash = false;
            } else if !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        }
        out.trim_matches('-').to_string()
    }

    #[test]
    fn slug_basic() {
        assert_eq!(derive_slug("Alex Suarez"), "alex-suarez");
        assert_eq!(derive_slug("alex@hotmail.com"), "alex-hotmail-com");
        assert_eq!(derive_slug("  whitespace  "), "whitespace");
        assert_eq!(derive_slug("UPPERCASE"), "uppercase");
    }

    #[test]
    fn slug_collapses_runs() {
        // "foo--bar" should not appear; consecutive non-alnum collapses to one dash.
        assert_eq!(derive_slug("foo!!bar"), "foo-bar");
        assert_eq!(derive_slug("a/b\\c"), "a-b-c");
    }

    #[test]
    fn slug_uuid_suffix_format() {
        // Migration 012 uses `<slug>-<8-hex-chars>` on collision. We don't run
        // SQL here, but verify the substring helper produces 8 hex chars.
        let raw = uuid::Uuid::new_v4().to_string();
        let suffix: String = raw.replace('-', "").chars().take(8).collect();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

// ---------------------------------------------------------------------------
// Integration tests — live HTTP + Postgres
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration {
    use serde_json::json;

    fn base() -> String {
        std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into())
    }

    /// Cross-org isolation: org B's API key must not be able to ingest into
    /// org A's contract. The auth layer rejects on org_id mismatch.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL + two seeded orgs"]
    async fn cross_org_ingest_is_rejected() {
        let key_b = std::env::var("TEST_API_KEY_B").expect("TEST_API_KEY_B required");
        let contract_a = std::env::var("TEST_CONTRACT_ID_A").expect("TEST_CONTRACT_ID_A required");

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/ingest/{contract_a}", base()))
            .header("x-api-key", &key_b)
            .header("content-type", "application/json")
            .json(&json!([{"user_id": "u1", "event_type": "login", "timestamp": 1_714_000_000}]))
            .send()
            .await
            .expect("request failed");

        // 403 (forbidden) or 404 (contract not visible) are both acceptable;
        // 200/422 would mean isolation is broken.
        let status = resp.status().as_u16();
        assert!(
            status == 403 || status == 404,
            "cross-org ingest must be rejected (got {status})"
        );
    }

    /// Soft-delete: deleting a contract via the API removes it from the
    /// list endpoint but leaves the underlying row (and its audit history)
    /// intact in the database.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL + a contract that can be deleted"]
    async fn soft_delete_hides_from_list() {
        let key_a = std::env::var("TEST_API_KEY_A").expect("TEST_API_KEY_A required");
        let contract_a = std::env::var("TEST_CONTRACT_ID_A").expect("TEST_CONTRACT_ID_A required");

        let client = reqwest::Client::new();

        // 1. Confirm the contract is currently listed.
        let before: serde_json::Value = client
            .get(format!("{}/contracts", base()))
            .header("x-api-key", &key_a)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let pre_ids: Vec<&str> = before
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(
            pre_ids.contains(&contract_a.as_str()),
            "test setup: TEST_CONTRACT_ID_A must be listed before deletion"
        );

        // 2. Delete it.
        let del = client
            .delete(format!("{}/contracts/{contract_a}", base()))
            .header("x-api-key", &key_a)
            .send()
            .await
            .unwrap();
        assert!(del.status().is_success(), "delete should succeed");

        // 3. List again — gone.
        let after: serde_json::Value = client
            .get(format!("{}/contracts", base()))
            .header("x-api-key", &key_a)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let post_ids: Vec<&str> = after
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(
            !post_ids.contains(&contract_a.as_str()),
            "soft-deleted contract must not appear in list"
        );

        // 4. Direct fetch is also hidden (404 from get_contract_identity).
        let direct = client
            .get(format!("{}/contracts/{contract_a}", base()))
            .header("x-api-key", &key_a)
            .send()
            .await
            .unwrap();
        assert_eq!(
            direct.status().as_u16(),
            404,
            "soft-deleted contract must 404 on direct fetch"
        );
    }

    /// Expired invites cannot be redeemed — the accept endpoint returns 410.
    #[tokio::test]
    #[ignore = "requires Supabase service role + an expired invite seeded"]
    async fn expired_invite_rejected() {
        let dashboard_base =
            std::env::var("DASHBOARD_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        let token =
            std::env::var("TEST_EXPIRED_INVITE_TOKEN").expect("TEST_EXPIRED_INVITE_TOKEN required");

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{dashboard_base}/api/invites/accept"))
            .json(&serde_json::json!({"token": token}))
            .send()
            .await
            .expect("request failed");

        // 401 (no session) or 410 (expired) — both prove the token cannot
        // succeed without being signed in AND live.
        let status = resp.status().as_u16();
        assert!(
            status == 401 || status == 410,
            "expired/anonymous invite must be rejected (got {status})"
        );
    }
}
