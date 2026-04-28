//! Thin HTTP client for the demo seeder.
//!
//! Wraps `reqwest::blocking::Client` with the two gateway operations the
//! seeder needs:
//!   1. `ensure_contract_published` — idempotently publish + promote a
//!      starter contract so the seeder can POST events to it.
//!   2. `post_event` — POST a single JSON event to `/ingest/:contract_id`
//!      and return the gateway's `passed` verdict.

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Gateway response shapes (only the fields we care about)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ContractSummary {
    pub id: Uuid,
    pub name: String,
    pub latest_stable_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchIngestResponse {
    #[allow(dead_code)]
    pub passed: usize,
    pub failed: usize,
}

// ---------------------------------------------------------------------------
// GatewayClient
// ---------------------------------------------------------------------------

pub struct GatewayClient {
    pub base_url: String,
    pub api_key: Option<String>,
    client: Client,
}

impl GatewayClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");
        GatewayClient {
            base_url,
            api_key,
            client,
        }
    }

    fn get(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        let req = self.client.get(format!("{}{}", self.base_url, path));
        self.with_auth(req)
    }

    fn post(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        let req = self.client.post(format!("{}{}", self.base_url, path));
        self.with_auth(req)
    }

    fn with_auth(
        &self,
        req: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match &self.api_key {
            Some(k) if !k.is_empty() => req.header("x-api-key", k),
            _ => req,
        }
    }

    // -------------------------------------------------------------------------
    // Contract publish (idempotent)
    // -------------------------------------------------------------------------

    /// Ensure a starter contract is published and has a stable version.
    ///
    /// Steps:
    ///   1. List contracts — if the name already has a stable version, return id.
    ///   2. If the name exists but has no stable, promote v1.0.0.
    ///   3. If the name is absent, POST /contracts then promote v1.0.0.
    pub fn ensure_contract_published(&self, name: &str, yaml_content: &str) -> Result<Uuid> {
        // 1. Check if already published.
        let resp = self.get("/contracts").send().context("GET /contracts")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("GET /contracts returned {status}: {body}");
        }

        // The list endpoint may return either `{"contracts": [...]}` or a bare
        // array depending on gateway version.  Try both shapes.
        let body: Value = resp.json().context("parse GET /contracts response")?;
        let items: Vec<ContractSummary> =
            if let Some(arr) = body.get("contracts").and_then(|v| v.as_array()) {
                serde_json::from_value(Value::Array(arr.clone()))
                    .context("deserialize contracts list")?
            } else if body.is_array() {
                serde_json::from_value(body).context("deserialize contracts list (bare array)")?
            } else {
                vec![]
            };

        if let Some(existing) = items.iter().find(|c| c.name == name) {
            if existing.latest_stable_version.is_some() {
                tracing::info!(name = name, id = %existing.id, "contract already published and stable");
                return Ok(existing.id);
            }
            // Exists but has no stable — promote the initial draft.
            tracing::info!(name = name, id = %existing.id, "contract exists; promoting v1.0.0");
            self.promote_version(existing.id, "1.0.0")?;
            return Ok(existing.id);
        }

        // 2. Create the contract (creates identity + v1.0.0 draft atomically).
        tracing::info!(name = name, "contract not found; creating");
        let create_body = json!({
            "name": name,
            "yaml_content": yaml_content,
        });
        let resp = self
            .post("/contracts")
            .json(&create_body)
            .send()
            .context("POST /contracts")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("POST /contracts returned {status}: {body}");
        }

        let created: Value = resp.json().context("parse POST /contracts response")?;
        let id: Uuid = serde_json::from_value(created.get("id").cloned().unwrap_or(Value::Null))
            .context("missing 'id' in POST /contracts response")?;

        // 3. Promote the initial v1.0.0 draft to stable.
        self.promote_version(id, "1.0.0")?;
        tracing::info!(name = name, %id, "contract created and promoted");
        Ok(id)
    }

    fn promote_version(&self, contract_id: Uuid, version: &str) -> Result<()> {
        let path = format!("/contracts/{contract_id}/versions/{version}/promote");
        let resp = self.post(&path).send().context("promote version")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("POST {path} returned {status}: {body}");
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Event ingest
    // -------------------------------------------------------------------------

    /// POST a single event to `/ingest/:contract_id`.
    ///
    /// Returns `(passed, round_trip_ms)`.
    pub fn post_event(&self, contract_id: Uuid, event: &Value) -> Result<(bool, u64)> {
        let start = std::time::Instant::now();
        let path = format!("/ingest/{contract_id}");
        let resp = self
            .post(&path)
            .json(event)
            .send()
            .with_context(|| format!("POST {path}"))?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let status = resp.status();

        if status.is_server_error() {
            let body = resp.text().unwrap_or_default();
            bail!("POST {path} server error {status}: {body}");
        }

        // Parse passed/failed counts from the response body.
        // A 200 means all passed; 207 means mixed; 422 means all failed.
        let passed = if status.as_u16() == 200 {
            true
        } else if status.as_u16() == 422 {
            false
        } else {
            // 207 Multi-Status: check the body
            let body: BatchIngestResponse = resp.json().context("parse ingest response")?;
            body.failed == 0
        };

        Ok((passed, elapsed_ms))
    }
}
