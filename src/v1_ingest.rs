//! RFC-021: `POST /v1/ingest/{contract_id}` — public bulk HTTP ingest endpoint.
//!
//! This is the universal connector surface: anything that can make an HTTP
//! request can validate events against a DataContractGate contract.
//!
//! ## What this module adds vs. `ingest.rs`
//!
//! | Feature                  | `/ingest/:raw_id` | `/v1/ingest/:contract_id` |
//! |--------------------------|:-----------------:|:-------------------------:|
//! | JSON array body          | ✓                 | ✓                         |
//! | Single-object body       | ✓                 | ✓                         |
//! | NDJSON body              | ✗                 | ✓                         |
//! | `?version=` param        | ✗ (@suffix/hdr)   | ✓                         |
//! | 10 MB body limit         | ✗                 | ✓                         |
//! | 1 MB per-event limit     | ✗                 | ✓                         |
//! | `Idempotency-Key` header | ✗                 | ✓                         |
//! | Per-key rate limiting    | ✗                 | ✓                         |
//! | `X-RateLimit-*` headers  | ✗                 | ✓                         |
//! | `quarantine_id` in result| ✗                 | ✓                         |
//! | `index` in result        | ✗                 | ✓                         |
//!
//! ## Execution order
//!
//! 1. Extract `ValidatedKey` — populated by `require_api_key` middleware.
//! 2. Rate-limit check.  Fail fast with 429 if exhausted.
//! 3. Idempotency-Key header: hit → return cached; conflict → 422.
//! 4. Read raw body bytes (10 MB limit already enforced by middleware layer).
//! 5. Parse body: JSON array / single object / NDJSON.
//! 6. Per-event 1 MB size check.  Batch count ≤ 1 000 check.
//! 7. Load contract identity + check key scope.
//! 8. Resolve version (`?version=` or latest stable).
//! 9. Parallel validation (rayon in `spawn_blocking`).
//! 10. RFC-004 PII transforms.
//! 11. Pre-assign quarantine UUIDs for rejected events.
//! 12. Persist audit + quarantine + forward (fire-and-forget).
//! 13. Store idempotency response (unless dry_run).
//! 14. Return response with `X-RateLimit-*` and optionally
//!     `X-Idempotency-Replay: true`.

use axum::{
    body::Bytes,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;
use crate::contract::{ContractIdentity, MultiStableResolution, VersionState};
use crate::error::{AppError, AppResult};
use crate::idempotency;
use crate::ingest::MAX_BATCH_SIZE;
use crate::storage;
use crate::transform::{apply_transforms, TransformedPayload};
use crate::validation::{check_uniqueness_batch, validate, CompiledContract, ValidationResult};
use crate::AppState;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Limits (RFC-021 §Size limits)
// ---------------------------------------------------------------------------

/// Max size of any single event object in the batch.
const MAX_EVENT_BYTES: usize = 1024 * 1024; // 1 MB

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct V1IngestQuery {
    /// Semver pin.  `None` → resolve to latest `stable` (RFC-002).
    pub version: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub atomic: bool,
    /// `quarantine` (default) or `drop`. Header `X-On-Fail` is the same switch.
    #[serde(default)]
    pub on_fail: Option<String>,
}

/// Per-event result returned in the response.
#[derive(Debug, Clone, Serialize)]
pub struct V1IngestEventResult {
    /// Zero-based position of this event in the submitted batch.
    pub index: usize,
    pub passed: bool,
    pub violations: Vec<crate::validation::Violation>,
    pub validation_us: u64,
    pub forwarded: bool,
    /// The contract version that produced the accept/reject decision.
    pub contract_version: String,
    /// UUID of the `quarantine_events` row for a rejected event.
    /// `null` for passing events, and for every event on `dry_run=true` or
    /// an atomic rejection, since neither stores quarantine rows.
    pub quarantine_id: Option<Uuid>,
    /// Post-RFC-004-transform payload that was persisted / forwarded.
    pub transformed_event: Value,
    /// `quarantined` or `dropped` for a failing record. Omitted on a pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<&'static str>,
}

/// Top-level response body.
#[derive(Debug, Serialize)]
pub struct V1IngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub quarantined: usize,
    pub dropped: usize,
    pub on_fail_mode: &'static str,
    pub on_fail_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_trip: Option<crate::fail_policy::AutoTripBody>,
    pub dry_run: bool,
    pub atomic: bool,
    pub resolved_version: String,
    /// The pin exactly as the caller sent it, present only when it resolved
    /// to a differently written stored version (e.g. `@1` → `1.0`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_version: Option<String>,
    /// One of: `"query_param"` | `"path"` | `"default_stable"` | `"pinned_deprecated"`.
    pub version_pin_source: &'static str,
    pub results: Vec<V1IngestEventResult>,
}

// ---------------------------------------------------------------------------
// Body parsing
// ---------------------------------------------------------------------------

/// Parse request body into a `Vec<Value>`.
///
/// Supports:
/// - `application/json`   — array of objects, or a single object
/// - `application/x-ndjson` — one JSON object per line
///
/// Blank lines in NDJSON are silently skipped.
/// A line > `MAX_EVENT_BYTES` → 413.
/// A line that isn't valid JSON → 400 with line index.
fn parse_body(content_type: &str, bytes: &[u8]) -> AppResult<Vec<Value>> {
    check_content_type(content_type)?;
    if content_type.contains("application/x-ndjson") {
        parse_ndjson(bytes)
    } else {
        // Default: JSON (array or single object).
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| AppError::BadRequest(format!("JSON parse error: {e}")))?;
        Ok(match v {
            Value::Array(arr) => arr,
            single => vec![single],
        })
    }
}

/// Reject bodies declared as something other than JSON or NDJSON (e.g.
/// `text/plain`, `application/xml`), the way the legacy `/ingest` route
/// does. A missing Content-Type is read as JSON so a bare `curl -d` works.
fn check_content_type(content_type: &str) -> AppResult<()> {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ok = media.is_empty()
        || media == "application/json"
        || media.ends_with("+json")
        || media == "application/x-ndjson";
    if ok {
        Ok(())
    } else {
        Err(AppError::UnsupportedMediaType(format!(
            "expected Content-Type application/json or application/x-ndjson, got {media}"
        )))
    }
}

fn parse_ndjson(bytes: &[u8]) -> AppResult<Vec<Value>> {
    let mut events = Vec::new();
    for (line_idx, line) in bytes.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() || line == b"\r" {
            continue; // blank / trailing newline
        }
        if line.len() > MAX_EVENT_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "NDJSON line {} exceeds the 1 MB per-event limit ({} bytes)",
                line_idx,
                line.len(),
            )));
        }
        let v: Value = serde_json::from_slice(line).map_err(|e| {
            AppError::BadRequest(format!("NDJSON parse error at line {line_idx}: {e}"))
        })?;
        events.push(v);
    }
    Ok(events)
}

/// Validate per-event sizes for a JSON-parsed batch.
fn check_event_sizes(events: &[Value]) -> AppResult<()> {
    for (idx, event) in events.iter().enumerate() {
        // Re-serialise to measure.  Cost is minimal relative to validation.
        let size = serde_json::to_vec(event)
            .map(|b| b.len())
            .unwrap_or(MAX_EVENT_BYTES + 1);
        if size > MAX_EVENT_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "Event at index {idx} exceeds the 1 MB per-event limit ({size} bytes)",
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rate-limit response helper
// ---------------------------------------------------------------------------

fn rate_limit_headers(
    outcome: &crate::rate_limit::RateLimitOutcome,
) -> [(&'static str, String); 3] {
    [
        ("X-RateLimit-Limit", outcome.limit.to_string()),
        ("X-RateLimit-Remaining", outcome.remaining.to_string()),
        ("X-RateLimit-Reset", outcome.reset_unix.to_string()),
    ]
}

// ---------------------------------------------------------------------------
// OpenAPI spec handler (GET /openapi.json)
// ---------------------------------------------------------------------------

/// Serve the statically-embedded OpenAPI spec.
///
/// The spec is the canonical source of truth for the v1 ingest surface; it
/// is generated from the type annotations in this module (see `build_openapi`)
/// and embedded at compile time so the `/openapi.json` endpoint adds zero
/// latency at runtime.
pub async fn openapi_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        build_openapi(),
    )
}

/// Build a minimal but accurate OpenAPI 3.1 document for the v1 ingest endpoint.
///
/// Kept hand-rolled for now (utoipa dependency deferred to task #5) so the
/// endpoint can ship with a usable spec immediately.  Task #5 will replace
/// this with utoipa-generated output; the shape is identical so no client
/// changes will be needed.
fn build_openapi() -> String {
    let mut doc = json!({
      "openapi": "3.1.0",
      "info": {
        "title": "DataContractGate Ingest API",
        "version": "1.0.0",
        "description": "POST events to DataContractGate for real-time semantic contract validation."
      },
      "servers": [{ "url": "https://api.datacontractgate.com", "description": "DataContractGate Cloud" }],
      "paths": {
        "/v1/ingest/{contract_id}": {
          "post": {
            "operationId": "v1_ingest",
            "summary": "Validate a batch of events against a contract",
            "parameters": [
              {
                "name": "contract_id",
                "in": "path",
                "required": true,
                "description": "Contract UUID, optionally suffixed with @<version> (same as ?version=).",
                "schema": { "type": "string" }
              },
              {
                "name": "version",
                "in": "query",
                "required": false,
                "description": "Semver pin. Defaults to latest stable.",
                "schema": { "type": "string", "example": "1.2.0" }
              },
              {
                "name": "dry_run",
                "in": "query",
                "required": false,
                "schema": { "type": "boolean", "default": false }
              },
              {
                "name": "atomic",
                "in": "query",
                "required": false,
                "description": "All-or-nothing batch semantics.",
                "schema": { "type": "boolean", "default": false }
              },
              {
                "name": "on_fail",
                "in": "query",
                "required": false,
                "description": "quarantine (default) stores a failing record. drop counts it and stores nothing. Ignored when the contract locks on_fail. Cannot force quarantine while an enforcing safety cap is tripped.",
                "schema": { "type": "string", "enum": ["quarantine", "drop"] }
              },
              {
                "name": "X-On-Fail",
                "in": "header",
                "required": false,
                "description": "Same values as on_fail. The query parameter wins when both are set to the same value; a disagreement is 400.",
                "schema": { "type": "string", "enum": ["quarantine", "drop"] }
              },
              {
                "name": "X-Api-Key",
                "in": "header",
                "required": true,
                "schema": { "type": "string" }
              },
              {
                "name": "Idempotency-Key",
                "in": "header",
                "required": false,
                "description": "Opaque string (max 255 chars). Same key + same body → cached response.",
                "schema": { "type": "string", "maxLength": 255 }
              }
            ],
            "requestBody": {
              "required": true,
              "content": {
                "application/json": {
                  "schema": {
                    "oneOf": [
                      { "type": "array", "items": { "type": "object" } },
                      { "type": "object" }
                    ]
                  }
                },
                "application/x-ndjson": {
                  "schema": { "type": "string", "description": "Newline-delimited JSON objects" }
                }
              }
            },
            "responses": {
              "200": { "description": "All events passed validation" },
              "207": { "description": "Mixed pass/fail batch" },
              "400": { "description": "Malformed body, empty batch, or size limit exceeded" },
              "401": { "description": "Missing or invalid X-Api-Key" },
              "413": { "description": "Body > 10 MB or event > 1 MB" },
              "422": { "description": "All events failed, atomic rejection, idempotency conflict, or deprecated version pin (code contract_version_deprecated)" },
              "429": { "description": "Per-key rate limit exceeded" }
            },
            "security": [{ "ApiKeyAuth": [] }]
          }
        },
        "/egress/{contract_id}": {
          "post": {
            "operationId": "egress",
            "summary": "Validate outbound records (e.g. LLM output) against a contract",
            "description": "Used by the AISchemaGate LLMGate SDK. Returns the post-transform payload to forward downstream; failing records are handled per `disposition`.",
            "parameters": [
              {
                "name": "contract_id",
                "in": "path",
                "required": true,
                "description": "Contract UUID, optionally suffixed with @<version>.",
                "schema": { "type": "string" }
              },
              {
                "name": "disposition",
                "in": "query",
                "required": false,
                "description": "block: drop failing records (default). fail: any failure fails the whole response (422). tag: keep failing records, marked.",
                "schema": { "type": "string", "enum": ["block", "fail", "tag"], "default": "block" }
              },
              {
                "name": "dry_run",
                "in": "query",
                "required": false,
                "schema": { "type": "boolean", "default": false }
              },
              {
                "name": "on_fail",
                "in": "query",
                "required": false,
                "description": "quarantine (default) or drop. AISchemaGate keeps quarantine as the default.",
                "schema": { "type": "string", "enum": ["quarantine", "drop"] }
              },
              {
                "name": "X-On-Fail",
                "in": "header",
                "required": false,
                "schema": { "type": "string", "enum": ["quarantine", "drop"] }
              },
              {
                "name": "X-Contract-Version",
                "in": "header",
                "required": false,
                "description": "Semver pin. Defaults to latest stable.",
                "schema": { "type": "string" }
              },
              {
                "name": "X-Api-Key",
                "in": "header",
                "required": true,
                "schema": { "type": "string" }
              }
            ],
            "requestBody": {
              "required": true,
              "content": {
                "application/json": {
                  "schema": {
                    "oneOf": [
                      { "type": "array", "items": { "type": "object" } },
                      { "type": "object" }
                    ]
                  }
                }
              }
            },
            "responses": {
              "200": { "description": "All records passed" },
              "207": { "description": "Some records failed (block / tag)" },
              "400": { "description": "Malformed body or contract id" },
              "401": { "description": "Missing or invalid X-Api-Key" },
              "413": { "description": "Body > 1 MB" },
              "422": { "description": "Every record failed, or any failed with disposition=fail (full egress response with per-record violations); or a deprecated version pin (code contract_version_deprecated)." },
              "429": { "description": "Per-key rate limit exceeded" }
            },
            "security": [{ "ApiKeyAuth": [] }]
          }
        },
        "/published/{publication_ref}": {
          "get": {
            "operationId": "fetch_published",
            "summary": "Fetch a published contract by ref (no API key)",
            "description": "Public publications need no token. Link/org publications need ?token=; a missing or wrong token returns 403 with a specific message (not the API-key 401).",
            "parameters": [
              { "name": "publication_ref", "in": "path", "required": true, "schema": { "type": "string" } },
              { "name": "token", "in": "query", "required": false, "schema": { "type": "string" } }
            ],
            "responses": {
              "200": { "description": "Publication YAML and metadata" },
              "403": { "description": "Link token missing or invalid" },
              "404": { "description": "Unknown or revoked publication" }
            }
          }
        },
        "/catalog": {
          "get": {
            "operationId": "list_public_catalog",
            "summary": "List public publications (no API key)",
            "parameters": [
              { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "default": 20, "maximum": 100 } }
            ],
            "responses": { "200": { "description": "Newest-first list with publication_ref, contract_name, contract_version, published_by, published_at" } }
          }
        },
        "/contracts/publications": {
          "get": {
            "operationId": "list_my_publications",
            "summary": "List the caller's live publications",
            "responses": { "200": { "description": "Live publications owned by the caller's org" }, "401": { "description": "Missing or invalid X-Api-Key" } },
            "security": [{ "ApiKeyAuth": [] }]
          }
        },
        "/contracts/publications/{publication_ref}": {
          "delete": {
            "operationId": "revoke_publication",
            "summary": "Unpublish (revoke) a publication",
            "parameters": [
              { "name": "publication_ref", "in": "path", "required": true, "schema": { "type": "string" } }
            ],
            "responses": {
              "200": { "description": "Revoked" },
              "401": { "description": "Missing or invalid X-Api-Key" },
              "403": { "description": "Publication belongs to another org" },
              "404": { "description": "Unknown or already revoked" },
              "409": { "description": "Publication has been imported; publish a new version instead" }
            },
            "security": [{ "ApiKeyAuth": [] }]
          }
        },
        "/contracts/import-published": {
          "post": {
            "operationId": "import_published",
            "summary": "Import a published contract into the caller's org",
            "description": "A published stable version is imported as stable (same version number) so it validates immediately. Link/org publications need link_token.",
            "requestBody": {
              "required": true,
              "content": {
                "application/json": {
                  "schema": {
                    "type": "object",
                    "required": ["publication_ref"],
                    "properties": {
                      "publication_ref": { "type": "string" },
                      "link_token": { "type": "string" },
                      "import_mode": { "type": "string", "enum": ["snapshot", "subscribe"], "default": "snapshot" }
                    }
                  }
                }
              }
            },
            "responses": {
              "201": { "description": "Imported; body includes state (stable|draft) and a message" },
              "401": { "description": "Missing or invalid X-Api-Key" },
              "403": { "description": "Link token missing or invalid" },
              "404": { "description": "Unknown or revoked publication" }
            },
            "security": [{ "ApiKeyAuth": [] }]
          }
        },
        "/contracts/{id}/versions/{version}/publish": { "post": { "operationId": "publish_version", "summary": "Publish a contract version (owner org)", "parameters": [ { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }, { "name": "version", "in": "path", "required": true, "schema": { "type": "string" } } ], "requestBody": { "required": false, "content": { "application/json": { "schema": { "type": "object", "properties": { "visibility": { "type": "string", "enum": ["public", "link", "org"], "default": "link" } } } } } }, "responses": { "201": { "description": "Published; body has publication_ref (and link_token for link/org)" }, "401": { "description": "Missing or invalid credentials" }, "404": { "description": "Contract or version not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/me": { "get": { "operationId": "me", "summary": "Caller org id, org name and plan (no emails). Also at /v1/me.", "responses": { "200": { "description": "org_id, org_name, plan, plan_display, paid_features, auth (api_key|session), api_key_id, monthly_event_limit, rate_limit_rps, rate_limit_burst. paid_features is true for a paid plan and for every plan when CONTRACTGATE_SELF_HOSTED is set. self_hosted is true only when that variable is set. plan stays the stored org plan. A self-hosted org reports a null monthly_event_limit and the paid rate limit unless the key overrides it." }, "401": { "description": "Missing or invalid credentials" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/v1/me": { "get": { "operationId": "v1_me", "summary": "Alias of /me", "responses": { "200": { "description": "Same as /me" }, "401": { "description": "Missing or invalid credentials" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/collaborators": { "get": { "operationId": "list_collaborators", "summary": "List collaborators (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "responses": { "200": { "description": "Collaborator grants" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "grant_collaborator", "summary": "Grant a role to another org (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["org_id", "role"], "properties": { "org_id": { "type": "string", "format": "uuid" }, "role": { "type": "string", "enum": ["viewer", "editor", "reviewer"] } } } } } }, "responses": { "201": { "description": "Grant created or updated" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/collaborators/{org_id}": { "patch": { "operationId": "patch_collaborator", "summary": "Change a role (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "org_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["role"], "properties": { "role": { "type": "string", "enum": ["viewer", "editor", "reviewer"] } } } } } }, "responses": { "200": { "description": "Updated grant" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "delete": { "operationId": "revoke_collaborator", "summary": "Revoke a grant (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "org_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "204": { "description": "Revoked" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments": { "get": { "operationId": "list_comments", "summary": "List comments (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "responses": { "200": { "description": "Comments, oldest first" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "add_comment", "summary": "Add a comment (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["author", "body"], "properties": { "field": { "type": ["string", "null"] }, "author": { "type": "string" }, "body": { "type": "string" } } } } } }, "responses": { "201": { "description": "Created" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments/{id}": { "delete": { "operationId": "delete_comment", "summary": "Delete a comment (author org or owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "204": { "description": "Deleted" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments/{id}/resolve": { "post": { "operationId": "resolve_comment", "summary": "Resolve a comment (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "200": { "description": "Resolved" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals": { "get": { "operationId": "list_proposals", "summary": "List proposals (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "responses": { "200": { "description": "Newest first" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "create_proposal", "summary": "Open a proposal (editor+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["proposed_yaml"], "properties": { "proposed_yaml": { "type": "string" } } } } } }, "responses": { "201": { "description": "Opened" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals/{id}": { "delete": { "operationId": "delete_proposal", "summary": "Delete a proposal (proposing org while open, or owner)", "parameters": [ { "name": "name", "in": "path", "required": true, "description": "Contract name or contract UUID", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } } ], "responses": { "204": { "description": "Deleted" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Not the proposing org or owner" }, "404": { "description": "Contract or proposal not found" }, "409": { "description": "Proposing org, but the proposal is no longer open" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals/{id}/decide": { "post": { "operationId": "decide_proposal", "summary": "Approve or reject (reviewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["decision"], "properties": { "decision": { "type": "string", "enum": ["approved", "rejected"] } } } } } }, "responses": { "200": { "description": "Decided" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals/{id}/apply": { "post": { "operationId": "apply_proposal", "summary": "Mark an approved proposal applied (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name, or contract UUID to address a shared contract by id", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "200": { "description": "Applied" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } }
      },
      "components": {
        "securitySchemes": {
          "ApiKeyAuth": {
            "type": "apiKey",
            "in": "header",
            "name": "X-Api-Key"
          }
        }
      }
    });
    if let Some(paths) = doc["paths"].as_object_mut() {
        extra_openapi_paths(paths);
    }
    crate::openapi_schemas::expand_empty_schemas(&mut doc);
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
}

/// Paths added after the main `json!` block, which sits at the macro's
/// recursion limit. One small `json!` per path keeps each expansion shallow.
fn extra_openapi_paths(paths: &mut serde_json::Map<String, Value>) {
    paths.insert("/contracts/{id}/versions/latest-stable".into(), json!({"get": {"operationId": "get_latest_stable_version", "summary": "Latest stable version (viewer+)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Version"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or no stable version"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}".into(), json!({"get": {"operationId": "get_version", "summary": "Get a stored version (viewer+)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Version, including yaml_content (its version: and name: match the row and contract)"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or version not found"}}, "security": [{"ApiKeyAuth": []}]}, "patch": {"operationId": "patch_version", "summary": "Edit a draft's YAML (editor+)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["yaml_content"], "properties": {"yaml_content": {"type": "string"}}}}}}, "responses": {"200": {"description": "Updated draft"}, "400": {"description": "Invalid YAML"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller lacks the required role"}, "404": {"description": "Contract or version not found"}, "409": {"description": "Version is not a draft"}}, "security": [{"ApiKeyAuth": []}]}, "delete": {"operationId": "delete_version", "summary": "Delete a draft (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"204": {"description": "Deleted"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller lacks the required role"}, "404": {"description": "Contract or version not found"}, "409": {"description": "Only draft versions may be deleted"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}/promote".into(), json!({"post": {"operationId": "promote_version", "summary": "Promote a draft to stable (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Promoted"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller lacks the required role"}, "404": {"description": "Contract or version not found"}, "409": {"description": "Illegal state change, or import still requires review"}, "504": {"description": "Timed out during pool_acquire, lock_wait, or db_statement. Nothing was applied. The error log names the route and the stage."}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}/deprecate".into(), json!({"post": {"operationId": "deprecate_version", "summary": "Deprecate a stable version (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Deprecated"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller lacks the required role"}, "404": {"description": "Contract or version not found"}, "409": {"description": "Illegal state change"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}/approve-import".into(), json!({"post": {"operationId": "approve_import", "summary": "Clear requires_review on a stripped ODCS import", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Review flag cleared"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or version not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}/odcs-conformance".into(), json!({"get": {"operationId": "odcs_conformance", "summary": "ODCS conformance report for a version", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Conformance report"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or version not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions/{version}/export".into(), json!({"get": {"operationId": "export_odcs", "summary": "Export a version as ODCS v3.1.0 YAML", "description": "Top-level `version` is the contract version row. The document validates against the official ODCS v3.1.0 JSON Schema. `x-contractgate-*` extensions are entries in the root `customProperties[]`; `x-contractgate-version` is the extension format version (currently \"1.0\"), not the contract version.", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "ODCS YAML", "content": {"application/yaml": {"schema": {"type": "string"}}}}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or version not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/export".into(), json!({"post": {"operationId": "export_fork_data", "summary": "Export a forked public-catalog contract's data", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "format", "in": "query", "required": false, "schema": {"type": "string", "enum": ["csv", "parquet"], "default": "csv"}}], "responses": {"200": {"description": "Exported data (CSV)"}, "400": {"description": "Unsupported format (parquet is not implemented yet)"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/import".into(), json!({"post": {"operationId": "import_odcs", "summary": "Import an ODCS v3.1 document as a draft version", "description": "With target_contract_id (or its alias contract_id) the document is imported as a new draft version on that contract, which must be owned by the caller. Without a target, a new contract is created, unless the caller already owns a live contract with that name (409; pass target_contract_id).", "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["odcs_yaml"], "properties": {"odcs_yaml": {"type": "string"}, "name_override": {"type": ["string", "null"]}, "target_contract_id": {"type": ["string", "null"], "format": "uuid"}, "contract_id": {"type": ["string", "null"], "format": "uuid", "description": "Alias of target_contract_id"}}}}}}, "responses": {"201": {"description": "Draft version created"}, "400": {"description": "Invalid ODCS document, or target_contract_id and contract_id disagree"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Collaborator grant on the target (owner required)"}, "404": {"description": "Target contract not found or not owned by the caller's org"}, "409": {"description": "Version (or an equivalent, e.g. 1.0 vs 1.0.0) already exists on the target, or a live contract with this name exists and no target was given"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kafka-ingress".into(), json!({"get": {"operationId": "get_kafka_ingress", "summary": "Kafka ingress config (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Config (no secret)"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kafka-ingress/enable".into(), json!({"post": {"operationId": "enable_kafka_ingress", "summary": "Enable Kafka ingress (owner, Enterprise; available on request)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Already enabled"}, "201": {"description": "Enabled; secret shown once"}, "400": {"description": "Kafka ingress unavailable on this deployment (KAFKA_ENABLED=false)"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}, "403": {"description": "Collaborator grant (owner required), or org below Enterprise: \"Kafka/Kinesis streaming ingress requires the Enterprise plan (available on request)\""}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kafka-ingress/disable".into(), json!({"delete": {"operationId": "disable_kafka_ingress", "summary": "Disable Kafka ingress (owner, any plan)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"204": {"description": "Disabled; credentials revoked"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kinesis-ingress".into(), json!({"get": {"operationId": "get_kinesis_ingress", "summary": "Kinesis ingress config (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Config (no secret)"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}, "501": {"description": "kinesis-ingress feature not compiled in"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kinesis-ingress/enable".into(), json!({"post": {"operationId": "enable_kinesis_ingress", "summary": "Enable Kinesis ingress (owner, Enterprise; available on request)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Already enabled"}, "201": {"description": "Enabled; secret shown once"}, "400": {"description": "Self-hosted gateway without AWS credentials: Kinesis ingress is currently unavailable on this deployment. Hosted gateways are unchanged."}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}, "403": {"description": "Collaborator grant (owner required), or org below Enterprise: \"Kafka/Kinesis streaming ingress requires the Enterprise plan (available on request)\""}, "501": {"description": "kinesis-ingress feature not compiled in"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kinesis-ingress/disable".into(), json!({"delete": {"operationId": "disable_kinesis_ingress", "summary": "Disable Kinesis ingress (owner, any plan)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"204": {"description": "Disabled"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}, "501": {"description": "kinesis-ingress feature not compiled in"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/kinesis-ingress/rotate-credentials".into(), json!({"post": {"operationId": "rotate_kinesis_credentials", "summary": "Rotate Kinesis IAM credentials (owner, Enterprise)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "New credentials; secret shown once"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found or not owned by the caller's org"}, "403": {"description": "Collaborator grant (owner required), or org below Enterprise: \"Kafka/Kinesis streaming ingress requires the Enterprise plan (available on request)\""}, "501": {"description": "kinesis-ingress feature not compiled in"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/deploy".into(), json!({"post": {"operationId": "deploy_contract", "summary": "Create or update a contract from YAML and promote it to stable", "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "yaml_content"], "properties": {"name": {"type": "string"}, "yaml_content": {"type": "string", "description": "Contract YAML; the version is read from it"}, "source": {"type": ["string", "null"]}, "deployed_by": {"type": ["string", "null"]}}}}}}, "responses": {"201": {"description": "Deployed: contract_id, version_id, name, version"}, "400": {"description": "Invalid YAML or request"}, "401": {"description": "Missing or invalid credentials"}, "409": {"description": "Version conflict"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/ingest/{contract_id}".into(), json!({"post": {"operationId": "ingest", "summary": "Validate one event or an array of events (legacy ingest; pin with {contract_id}@{version} or X-Contract-Version)", "parameters": [{"name": "contract_id", "in": "path", "required": true, "description": "Contract UUID, optionally suffixed @version. Pins match exactly first, then by normalised numeric version (1.0.0 matches a stored 1.0).", "schema": {"type": "string"}}, {"name": "X-Contract-Version", "in": "header", "required": false, "schema": {"type": "string"}}, {"name": "dry_run", "in": "query", "required": false, "schema": {"type": "boolean", "default": false}}, {"name": "atomic", "in": "query", "required": false, "schema": {"type": "boolean", "default": false}}, {"name": "on_fail", "in": "query", "required": false, "schema": {"type": "string", "enum": ["quarantine", "drop"]}}, {"name": "X-On-Fail", "in": "header", "required": false, "schema": {"type": "string", "enum": ["quarantine", "drop"]}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"oneOf": [{"type": "object"}, {"type": "array", "items": {"type": "object"}}]}}}}, "responses": {"200": {"description": "Per-event results"}, "400": {"description": "Invalid contract id or body"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or pinned version not found"}, "409": {"description": "No stable version, or ambiguous version pin"}, "422": {"description": "Pinned version is deprecated (code contract_version_deprecated)"}, "429": {"description": "Plan limit exceeded"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/versions".into(), json!({"get": {"operationId": "list_versions", "summary": "List versions (viewer+)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Version summaries"}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found"}}, "security": [{"ApiKeyAuth": []}]}, "post": {"operationId": "create_version", "summary": "Create a draft version", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["version", "yaml_content"], "properties": {"version": {"type": "string", "description": "e.g. 1.1.0; unique per contract"}, "yaml_content": {"type": "string"}}}}}}, "responses": {"201": {"description": "Draft created"}, "400": {"description": "Invalid YAML"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller lacks the required role"}, "404": {"description": "Contract not found"}, "409": {"description": "Version already exists"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts".into(), json!({"get": {"operationId": "list_contracts", "summary": "List contracts for the caller's org", "description": "store_event_payloads is the effective value ingest uses: false unless the plan is paid (growth, aisg_pro, enterprise) AND the org master switch is on AND the per-contract override is on. free, aisg_free, and unknown plans report false. A paid plan with the org switch off (the default) also reports false. The columns are not rewritten. Shared by DataContractGate and AISchemaGate.", "responses": {"200": {"description": "Contract summaries. store_event_payloads is boolean and effective for the plan.", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "name": {"type": "string"}, "store_event_payloads": {"type": "boolean", "description": "True only when ingest will store the body: paid plan, org switch on, per-contract override on."}}}}}}}, "401": {"description": "Missing or invalid credentials"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/quarantine/replay".into(), json!({"post": {"operationId": "replay_quarantine", "summary": "Replay quarantined events across the caller's contracts", "description": "Same engine as POST /contracts/{id}/quarantine/replay. A row whose body was not stored is redacted: true (per-contract route: outcome redacted), with passed false and violations empty. Not a validation failure.", "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["event_ids"], "properties": {"event_ids": {"type": "array", "items": {"type": "string", "format": "uuid"}}, "version": {"type": "string"}, "contract_id": {"type": "string", "format": "uuid"}}}}}}, "responses": {"200": {"description": "One outcome per event_id, in input order.", "content": {"application/json": {"schema": {"type": "object", "properties": {"replayed": {"type": "integer"}, "outcomes": {"type": "array", "items": {"type": "object", "properties": {"event_id": {"type": "string", "format": "uuid"}, "version": {"type": "string"}, "passed": {"type": "boolean"}, "violations": {"type": "array"}, "replayed_at": {"type": "string", "format": "date-time"}, "redacted": {"type": "boolean", "description": "True when the source body was not stored. Same classification as per-contract outcome=redacted."}}}}}}}}}, "400": {"description": "Empty event_ids, over the 1000 cap, or contract_id assertion failed"}, "401": {"description": "Missing or invalid credentials"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/quarantine/replay".into(), json!({"post": {"operationId": "replay_contract_quarantine", "summary": "Replay quarantined events for one contract", "description": "A row whose body was not stored returns outcome redacted (and increments the redacted count). The org-wide POST /quarantine/replay reports that same row with redacted true.", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["ids"], "properties": {"ids": {"type": "array", "items": {"type": "string", "format": "uuid"}}, "target_version": {"type": "string"}}}}}}, "responses": {"200": {"description": "Per-id results. outcome=redacted when the body was not stored."}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract or target version not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    let fail_schema = json!({"type": "object", "properties": {"on_fail": {"type": "string", "enum": ["quarantine", "drop"]}, "on_fail_locked": {"type": "boolean"}, "quarantine_guard": {"type": "object"}, "from_override": {"type": "boolean"}, "account_guard": {"type": ["object", "null"]}, "auto_trip": {"type": ["object", "null"]}}});
    paths.insert("/contracts/{id}/fail-policy".into(), json!({"get": {"operationId": "get_fail_policy", "summary": "On-fail setting and safety-cap state", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Effective on-fail setting", "content": {"application/json": {"schema": fail_schema}}}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found"}}, "security": [{"ApiKeyAuth": []}]}, "patch": {"operationId": "set_fail_policy", "summary": "Save the live on-fail override (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["on_fail"], "properties": {"on_fail": {"type": "string", "enum": ["quarantine", "drop"]}, "on_fail_locked": {"type": "boolean"}, "quarantine_guard": {"type": "object"}}}}}}, "responses": {"200": {"description": "Updated setting", "content": {"application/json": {"schema": {"type": "object", "properties": {"on_fail": {"type": "string"}}}}}}, "400": {"description": "Invalid threshold"}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller is not the contract owner"}, "404": {"description": "Contract not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/drop-counts".into(), json!({"get": {"operationId": "get_drop_counts", "summary": "Dropped-record counts by reason", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}, {"name": "hours", "in": "query", "required": false, "schema": {"type": "integer", "default": 24}}], "responses": {"200": {"description": "Reason totals for the window", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"reason_key": {"type": "string"}, "source": {"type": "string"}, "dropped": {"type": "integer"}}}}}}}, "401": {"description": "Missing or invalid credentials"}, "404": {"description": "Contract not found"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/contracts/{id}/quarantine-guard/reset".into(), json!({"post": {"operationId": "reset_contract_quarantine_guard", "summary": "Clear an enforcing contract safety-cap trip (owner)", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}], "responses": {"200": {"description": "Reset result", "content": {"application/json": {"schema": {"type": "object", "properties": {"reset": {"type": "boolean"}, "scope": {"type": "string"}}}}}}, "401": {"description": "Missing or invalid credentials"}, "403": {"description": "Caller is not the contract owner"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/settings/quarantine-guard".into(), json!({"get": {"operationId": "get_account_quarantine_guard", "summary": "Account safety cap", "responses": {"200": {"description": "Account cap, or null when unset", "content": {"application/json": {"schema": {"type": "object", "properties": {"quarantine_guard": {"type": ["object", "null"]}}}}}}, "400": {"description": "No org on the caller"}, "401": {"description": "Missing or invalid credentials"}}, "security": [{"ApiKeyAuth": []}]}, "patch": {"operationId": "set_account_quarantine_guard", "summary": "Save or clear the account safety cap", "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "properties": {"quarantine_guard": {"type": ["object", "null"]}}}}}}, "responses": {"200": {"description": "Saved cap", "content": {"application/json": {"schema": {"type": "object", "properties": {"quarantine_guard": {"type": ["object", "null"]}}}}}}, "400": {"description": "Invalid threshold or no org"}, "401": {"description": "Missing or invalid credentials"}}, "security": [{"ApiKeyAuth": []}]}}));
    paths.insert("/settings/quarantine-guard/reset".into(), json!({"post": {"operationId": "reset_account_quarantine_guard", "summary": "Clear an enforcing account safety-cap trip", "responses": {"200": {"description": "Reset result", "content": {"application/json": {"schema": {"type": "object", "properties": {"reset": {"type": "boolean"}, "scope": {"type": "string"}}}}}}, "400": {"description": "No org on the caller"}, "401": {"description": "Missing or invalid credentials"}}, "security": [{"ApiKeyAuth": []}]}}));
    more_openapi_paths(paths);
}

/// Add or merge one HTTP operation. Merging keeps a method already documented
/// on the same path (for example `POST /contracts` next to `GET /contracts`).
fn merge_openapi_op(
    paths: &mut serde_json::Map<String, Value>,
    path: &str,
    method: &str,
    op: Value,
) {
    let item = paths.entry(path.to_string()).or_insert_with(|| json!({}));
    if let Some(obj) = item.as_object_mut() {
        obj.insert(method.to_string(), op);
    }
}

/// Routes registered in `build_router` that were not in the original `json!`
/// document. Schemas match the handler request and response types.
fn more_openapi_paths(paths: &mut serde_json::Map<String, Value>) {
    let upgrade = crate::plan::payload_storage_upgrade_message();
    let paid = crate::plan::paid_plans_phrase();
    let auth = json!([{"ApiKeyAuth": []}]);
    let unauthorized = json!({"description": "Missing or invalid credentials"});

    merge_openapi_op(
        paths,
        "/health",
        "get",
        json!({
            "operationId": "health",
            "summary": "Liveness probe (no database)",
            "responses": {"200": {"description": "Process is up", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}, "version": {"type": "string"}, "build": {"type": ["string", "null"]}, "service": {"type": "string"}}}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/ready",
        "get",
        json!({
            "operationId": "ready",
            "summary": "Readiness probe (SELECT 1, 2s timeout)",
            "responses": {
                "200": {"description": "Pool is healthy", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}, "db": {"type": "string"}, "version": {"type": "string"}, "pool": {"type": "object", "properties": {"size": {"type": "integer"}, "idle": {"type": "integer"}}}}}}}},
                "503": {"description": "Pool exhausted or database unreachable", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}, "db": {"type": "string"}}}}}}
            }
        }),
    );
    merge_openapi_op(
        paths,
        "/openapi.json",
        "get",
        json!({
            "operationId": "openapi",
            "summary": "This OpenAPI document",
            "responses": {"200": {"description": "OpenAPI 3.1 JSON", "content": {"application/json": {"schema": {"type": "object"}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/metrics",
        "get",
        json!({
            "operationId": "metrics",
            "summary": "Prometheus metrics",
            "description": "401 unless METRICS_AUTH_TOKEN matches Authorization: Bearer, or METRICS_PUBLIC=1.",
            "parameters": [{"name": "Authorization", "in": "header", "required": false, "schema": {"type": "string"}}],
            "responses": {
                "200": {"description": "Prometheus text exposition", "content": {"text/plain": {"schema": {"type": "string"}}}},
                "401": {"description": "Metrics are not public and the bearer token is missing or wrong"}
            }
        }),
    );
    merge_openapi_op(
        paths,
        "/demo/start",
        "post",
        json!({
            "operationId": "demo_start",
            "summary": "Start the local stream demo",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["scenario", "fail_ratio", "duration_secs"], "properties": {"scenario": {"type": "string"}, "fail_ratio": {"type": "number"}, "target_rate": {"type": ["integer", "null"]}, "duration_secs": {"type": "integer"}}}}}},
            "responses": {
                "200": {"description": "Started", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}, "scenario": {"type": "string"}}}}}},
                "400": {"description": "Unknown scenario"}
            }
        }),
    );
    merge_openapi_op(
        paths,
        "/demo/stop",
        "post",
        json!({
            "operationId": "demo_stop",
            "summary": "Stop the local stream demo",
            "responses": {"200": {"description": "Stopped", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/demo/stream",
        "get",
        json!({
            "operationId": "demo_stream",
            "summary": "Server-sent stats snapshots for the stream demo",
            "responses": {"200": {"description": "text/event-stream of stats snapshots", "content": {"text/event-stream": {"schema": {"type": "string"}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/demo/events",
        "get",
        json!({
            "operationId": "demo_events",
            "summary": "Server-sent demo events",
            "responses": {"200": {"description": "text/event-stream of demo events", "content": {"text/event-stream": {"schema": {"type": "string"}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/demo/contract",
        "get",
        json!({
            "operationId": "demo_contract",
            "summary": "YAML for a demo scenario",
            "parameters": [{"name": "scenario", "in": "query", "required": false, "schema": {"type": "string", "enum": ["simple", "nested"], "default": "simple"}}],
            "responses": {
                "200": {"description": "Contract YAML", "content": {"text/plain": {"schema": {"type": "string"}}}},
                "404": {"description": "Unknown scenario"}
            }
        }),
    );
    merge_openapi_op(
        paths,
        "/public-contracts",
        "get",
        json!({
            "operationId": "list_public_contracts",
            "summary": "List curated public-catalog contracts (no API key)",
            "responses": {"200": {"description": "Catalog summaries", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "name": {"type": "string"}, "description": {"type": ["string", "null"]}, "source_format": {"type": "string"}, "version": {"type": "string"}, "created_at": {"type": "string", "format": "date-time"}, "updated_at": {"type": "string", "format": "date-time"}}}}}}}}
        }),
    );
    merge_openapi_op(
        paths,
        "/public-contracts/{id}",
        "get",
        json!({
            "operationId": "get_public_contract",
            "summary": "One public-catalog contract, including YAML (no API key)",
            "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
            "responses": {
                "200": {"description": "Catalog entry", "content": {"application/json": {"schema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "name": {"type": "string"}, "description": {"type": ["string", "null"]}, "source_url": {"type": "string"}, "source_format": {"type": "string"}, "contract_yaml": {"type": "string"}, "version": {"type": "string"}, "created_at": {"type": "string", "format": "date-time"}, "updated_at": {"type": "string", "format": "date-time"}}}}}},
                "404": {"description": "Unknown id"}
            }
        }),
    );

    let store_desc = format!(
        "True only when ingest will store the body: paid plan ({paid}), org master switch on, and per-contract override on. free, aisg_free, and unknown plans are false. A paid plan with the org switch off is false. Columns are not rewritten."
    );
    let store_flag = json!({
        "type": "boolean",
        "description": store_desc
    });
    let contract_schema = json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "format": "uuid"},
            "name": {"type": "string"},
            "description": {"type": ["string", "null"]},
            "multi_stable_resolution": {"type": "string", "enum": ["strict", "fallback"]},
            "created_at": {"type": "string", "format": "date-time"},
            "updated_at": {"type": "string", "format": "date-time"},
            "version_count": {"type": "integer"},
            "latest_stable_version": {"type": ["string", "null"]},
            "store_event_payloads": store_flag
        }
    });
    merge_openapi_op(
        paths,
        "/contracts",
        "post",
        json!({
            "operationId": "create_contract",
            "summary": "Create a contract and its initial v1.0.0 draft",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "yaml_content"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "yaml_content": {"type": "string"}, "multi_stable_resolution": {"type": "string", "enum": ["strict", "fallback"]}}}}}},
            "responses": {
                "201": {"description": "Created. store_event_payloads is the effective flag.", "content": {"application/json": {"schema": contract_schema.clone()}}},
                "400": {"description": "Invalid YAML or missing org context in dev mode"},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    let id_param = json!({"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}});
    let get_desc = format!(
        "store_event_payloads is the effective value, the same decision as GET /contracts: paid plan ({paid}) AND org master switch AND per-contract override. Shared by DataContractGate and AISchemaGate."
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}",
        "get",
        json!({
            "operationId": "get_contract",
            "summary": "Get one contract (viewer+)",
            "description": get_desc,
            "parameters": [id_param.clone()],
            "responses": {
                "200": {"description": "Contract. store_event_payloads is boolean and effective for the plan.", "content": {"application/json": {"schema": contract_schema.clone()}}},
                "401": unauthorized.clone(),
                "403": {"description": "Caller holds a grant below viewer"},
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}",
        "patch",
        json!({
            "operationId": "patch_contract",
            "summary": "Update contract identity metadata (owner)",
            "parameters": [id_param.clone()],
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "multi_stable_resolution": {"type": "string", "enum": ["strict", "fallback"]}}}}}},
            "responses": {
                "200": {"description": "Updated contract. store_event_payloads is the effective flag.", "content": {"application/json": {"schema": contract_schema}}},
                "401": unauthorized.clone(),
                "403": {"description": "Caller is not the owner"},
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}",
        "delete",
        json!({
            "operationId": "delete_contract",
            "summary": "Soft-delete a contract (owner)",
            "parameters": [id_param.clone()],
            "responses": {"204": {"description": "Deleted"}, "401": unauthorized.clone(), "403": {"description": "Caller is not the owner"}, "404": {"description": "Contract not found"}},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/name-history",
        "get",
        json!({
            "operationId": "list_name_history",
            "summary": "Rename history for a contract",
            "parameters": [id_param.clone()],
            "responses": {
                "200": {"description": "History, newest changes included", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "contract_id": {"type": "string", "format": "uuid"}, "old_name": {"type": "string"}, "new_name": {"type": "string"}, "changed_at": {"type": "string", "format": "date-time"}}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/diff",
        "post",
        json!({
            "operationId": "diff_contracts",
            "summary": "Diff two contract YAML documents",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["contract_yaml_a", "contract_yaml_b"], "properties": {"contract_yaml_a": {"type": "string"}, "contract_yaml_b": {"type": "string"}}}}}},
            "responses": {
                "200": {"description": "Summary and structured changes", "content": {"application/json": {"schema": {"type": "object", "properties": {"summary": {"type": "string"}, "changes": {"type": "array", "items": {"type": "object", "properties": {"kind": {"type": "string"}, "field": {"type": "string"}, "detail": {"type": "string"}}}}}}}}},
                "400": {"description": "Invalid YAML"},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/scaffold",
        "post",
        json!({
            "operationId": "scaffold_contract",
            "summary": "Scaffold a draft contract from samples or a schema file",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "content": {"type": ["string", "null"]}, "samples": {"type": "array", "items": {"type": "object"}}, "format": {"type": "string", "enum": ["json", "ndjson", "avro_schema", "proto"]}, "fast": {"type": "boolean"}, "pii_threshold": {"type": "number"}, "max_records": {"type": "integer"}}}}}},
            "responses": {
                "200": {"description": "Draft YAML plus PII candidates", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "sample_count": {"type": "integer"}, "pii_candidate_count": {"type": "integer"}, "pii_candidates": {"type": "array", "items": {"type": "object", "properties": {"field_name": {"type": "string"}, "confidence": {"type": "number"}, "reason": {"type": "string"}, "suggested_transform": {"type": "string"}}}}, "format": {"type": "string"}, "sr_unavailable": {"type": "boolean"}}}}}},
                "400": {"description": "Unusable input"},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    more_openapi_paths_tail(paths, &upgrade, &auth, &unauthorized, &id_param);
}

fn more_openapi_paths_tail(
    paths: &mut serde_json::Map<String, Value>,
    upgrade: &str,
    auth: &Value,
    unauthorized: &Value,
    id_param: &Value,
) {
    merge_openapi_op(
        paths,
        "/ingest/{contract_id}/stats",
        "get",
        json!({
            "operationId": "ingest_stats",
            "summary": "Ingestion stats for one contract",
            "parameters": [{"name": "contract_id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
            "responses": {
                "200": {"description": "Totals and latency percentiles", "content": {"application/json": {"schema": {"type": "object", "properties": {"total_events": {"type": "integer"}, "passed_events": {"type": "integer"}, "failed_events": {"type": "integer"}, "pass_rate": {"type": "number"}, "avg_validation_us": {"type": "number"}, "p50_validation_us": {"type": "integer"}, "p95_validation_us": {"type": "integer"}, "p99_validation_us": {"type": "integer"}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    let source_param =
        json!({"name": "source", "in": "path", "required": true, "schema": {"type": "string"}});
    let drift_item = json!({"type": "object", "properties": {"source": {"type": "string"}, "contract_name": {"type": "string"}, "field": {"type": "string"}, "current_rate": {"type": "number"}, "baseline_rate": {"type": "number"}, "delta_pct": {"type": "number"}, "label": {"type": "string"}}});
    merge_openapi_op(
        paths,
        "/scorecard/{source}",
        "get",
        json!({
            "operationId": "get_scorecard",
            "summary": "Provider scorecard for one source",
            "parameters": [source_param.clone()],
            "responses": {
                "200": {"description": "Summary, field health, and drift", "content": {"application/json": {"schema": {"type": "object", "properties": {"source": {"type": "string"}, "summary": {"type": "array", "items": {"type": "object", "properties": {"source": {"type": "string"}, "contract_name": {"type": "string"}, "total_events": {"type": "integer"}, "passed": {"type": "integer"}, "quarantined": {"type": "integer"}, "quarantine_pct": {"type": ["number", "null"]}}}}, "top_violations": {"type": "array", "items": {"type": "object"}}, "field_health": {"type": "array", "items": {"type": "object", "properties": {"source": {"type": "string"}, "contract_name": {"type": "string"}, "field": {"type": "string"}, "code": {"type": "string"}, "violations": {"type": "integer"}}}}, "drift_signals": {"type": "array", "items": drift_item}}}}}},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/scorecard/{source}/drift",
        "get",
        json!({
            "operationId": "get_scorecard_drift",
            "summary": "Active drift signals for one source",
            "parameters": [source_param.clone()],
            "responses": {"200": {"description": "Drift signals", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"source": {"type": "string"}, "contract_name": {"type": "string"}, "field": {"type": "string"}, "current_rate": {"type": "number"}, "baseline_rate": {"type": "number"}, "delta_pct": {"type": "number"}, "label": {"type": "string"}}}}}}}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/scorecard/{source}/export",
        "get",
        json!({
            "operationId": "export_scorecard",
            "summary": "Download a scorecard as CSV",
            "parameters": [source_param, {"name": "format", "in": "query", "required": false, "schema": {"type": "string", "enum": ["csv"], "default": "csv"}}],
            "responses": {"200": {"description": "CSV", "content": {"text/csv": {"schema": {"type": "string"}}}}, "400": {"description": "Only format=csv is supported"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/quarantine/{quar_id}/replay-history",
        "get",
        json!({
            "operationId": "quarantine_replay_history",
            "summary": "Replay history for one quarantine row",
            "parameters": [id_param.clone(), {"name": "quar_id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
            "responses": {
                "200": {"description": "Chain of source, failed_replay, and passed_replay entries (serde tag kind)", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "required": ["kind"], "properties": {"kind": {"type": "string", "enum": ["source", "failed_replay", "passed_replay"]}, "id": {"type": "string", "format": "uuid"}, "contract_version": {"type": "string"}, "status": {"type": "string"}, "violation_count": {"type": "integer"}, "replayed_at": {"type": ["string", "null"], "format": "date-time"}, "replayed_into_audit_id": {"type": ["string", "null"], "format": "uuid"}, "created_at": {"type": "string", "format": "date-time"}}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    let report_schema = json!({
        "type": "object",
        "properties": {
            "contract_id": {"type": "string", "format": "uuid"},
            "contract_name": {"type": "string"},
            "window": {"type": "object", "properties": {"from": {"type": ["string", "null"]}, "to": {"type": ["string", "null"]}}},
            "generated_at": {"type": "string", "format": "date-time"},
            "totals": {"type": "object", "properties": {"total": {"type": "integer"}, "passed": {"type": "integer"}, "quarantined": {"type": "integer"}, "pass_rate": {"type": "number"}}},
            "by_version": {"type": "array", "items": {"type": "object"}},
            "top_violations": {"type": "array", "items": {"type": "object", "properties": {"field": {"type": ["string", "null"]}, "kind": {"type": ["string", "null"]}, "count": {"type": "integer"}}}}
        }
    });
    merge_openapi_op(
        paths,
        "/contracts/{id}/report",
        "get",
        json!({
            "operationId": "contract_report",
            "summary": "Pilot report for one contract (JSON or CSV)",
            "parameters": [
                id_param.clone(),
                {"name": "from", "in": "query", "required": false, "schema": {"type": "string", "format": "date-time"}},
                {"name": "to", "in": "query", "required": false, "schema": {"type": "string", "format": "date-time"}},
                {"name": "format", "in": "query", "required": false, "schema": {"type": "string", "enum": ["json", "csv"], "default": "json"}}
            ],
            "responses": {
                "200": {"description": "JSON report, or CSV when format=csv", "content": {"application/json": {"schema": report_schema}}},
                "400": {"description": "Unsupported format"},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/usage",
        "get",
        json!({
            "operationId": "get_usage",
            "summary": "Current-month event usage against the plan limit",
            "responses": {
                "200": {"description": "Usage. limit, remaining, and pct are null when unlimited.", "content": {"application/json": {"schema": {"type": "object", "properties": {"plan": {"type": "string"}, "period_start": {"type": "string", "format": "date-time"}, "used": {"type": "integer"}, "limit": {"type": ["integer", "null"]}, "remaining": {"type": ["integer", "null"]}, "pct": {"type": ["number", "null"]}, "unlimited": {"type": "boolean"}}}}}},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    let quarantined_event = json!({"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "contract_id": {"type": "string", "format": "uuid"}, "contract_version": {"type": ["string", "null"]}, "raw_event": {}, "payload_redacted": {"type": "boolean"}, "violation_details": {}, "violation_count": {"type": "integer"}, "source_ip": {"type": ["string", "null"]}, "quarantined_at": {"type": "string", "format": "date-time"}, "replay_count": {"type": "integer"}, "last_replayed_at": {"type": ["string", "null"], "format": "date-time"}, "last_replay_passed": {"type": ["boolean", "null"]}}});
    merge_openapi_op(
        paths,
        "/quarantine",
        "get",
        json!({
            "operationId": "list_quarantine",
            "summary": "List quarantined events for the caller's org",
            "parameters": [
                {"name": "contract_id", "in": "query", "required": false, "schema": {"type": "string", "format": "uuid"}},
                {"name": "limit", "in": "query", "required": false, "schema": {"type": "integer", "default": 100, "maximum": 500}},
                {"name": "offset", "in": "query", "required": false, "schema": {"type": "integer", "default": 0}}
            ],
            "responses": {"200": {"description": "Quarantine rows. payload_redacted true means raw_event is null.", "content": {"application/json": {"schema": {"type": "array", "items": quarantined_event}}}}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    let replay_outcome = json!({"type": "object", "properties": {"event_id": {"type": "string", "format": "uuid"}, "version": {"type": "string"}, "passed": {"type": "boolean"}, "violations": {"type": "array"}, "replayed_at": {"type": "string", "format": "date-time"}, "redacted": {"type": "boolean"}}});
    merge_openapi_op(
        paths,
        "/quarantine/replay-history",
        "get",
        json!({
            "operationId": "quarantine_replay_history_all",
            "summary": "Replay attempts for one event across the caller's contracts",
            "parameters": [
                {"name": "event_id", "in": "query", "required": true, "schema": {"type": "string", "format": "uuid"}},
                {"name": "limit", "in": "query", "required": false, "schema": {"type": "integer", "default": 100, "maximum": 500}}
            ],
            "responses": {"200": {"description": "Attempts, oldest source row excluded", "content": {"application/json": {"schema": {"type": "array", "items": replay_outcome}}}}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    let payload_status = json!({"type": "object", "required": ["enabled", "plan", "eligible"], "properties": {"enabled": {"type": "boolean", "description": "Org master switch"}, "plan": {"type": "string"}, "eligible": {"type": "boolean", "description": "True when the plan is paid, so enabling the switch is allowed"}}});
    let enabled_body = json!({"type": "object", "required": ["enabled"], "properties": {"enabled": {"type": "boolean"}}});
    merge_openapi_op(
        paths,
        "/settings/payload-storage",
        "get",
        json!({
            "operationId": "get_payload_storage",
            "summary": "Org event-payload storage switch and plan eligibility",
            "responses": {"200": {"description": "enabled is the org master switch. eligible is true for a paid plan.", "content": {"application/json": {"schema": payload_status.clone()}}}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/settings/payload-storage",
        "put",
        json!({
            "operationId": "set_payload_storage",
            "summary": "Set the org event-payload storage switch",
            "description": "Enabling requires a paid plan. Disabling purges stored bodies for the org, including forwarded_events payloads (JSON null). Rows are kept.",
            "requestBody": {"required": true, "content": {"application/json": {"schema": enabled_body.clone()}}},
            "responses": {
                "200": {"description": "Updated status", "content": {"application/json": {"schema": payload_status}}},
                "400": {"description": upgrade},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/payload-storage",
        "patch",
        json!({
            "operationId": "set_contract_payload_storage",
            "summary": "Set the per-contract payload-storage override",
            "description": "Write-forward only: does not purge history. Effective when the org master switch is on.",
            "parameters": [id_param.clone()],
            "requestBody": {"required": true, "content": {"application/json": {"schema": enabled_body}}},
            "responses": {
                "200": {"description": "The override that was written", "content": {"application/json": {"schema": {"type": "object", "required": ["enabled"], "properties": {"enabled": {"type": "boolean"}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found or not owned by the caller's org"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/purge-bodies",
        "post",
        json!({
            "operationId": "purge_contract_bodies",
            "summary": "Redact stored bodies for one contract",
            "description": "Keeps audit, quarantine, and forwarded_events rows. Nulls stored bodies; forwarded_events.payload becomes JSON null. Never deletes a row.",
            "parameters": [id_param.clone()],
            "responses": {
                "200": {"description": "Rows whose bodies were redacted", "content": {"application/json": {"schema": {"type": "object", "required": ["quarantine_bodies_redacted", "audit_bodies_redacted", "forwarded_bodies_redacted"], "properties": {"quarantine_bodies_redacted": {"type": "integer"}, "audit_bodies_redacted": {"type": "integer"}, "forwarded_bodies_redacted": {"type": "integer", "description": "forwarded_events rows whose payload was set to JSON null"}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/import-status",
        "get",
        json!({
            "operationId": "import_status",
            "summary": "Whether a newer published version is available for an import",
            "parameters": [id_param.clone()],
            "responses": {
                "200": {"description": "Import provenance", "content": {"application/json": {"schema": {"type": "object", "properties": {"import_mode": {"type": ["string", "null"]}, "publication_ref": {"type": ["string", "null"]}, "source_revoked": {"type": "boolean"}, "update_available": {"type": "boolean"}, "latest_published_version": {"type": ["string", "null"]}, "imported_version": {"type": ["string", "null"]}}}}}},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/{id}/fork",
        "post",
        json!({
            "operationId": "fork_public_contract",
            "summary": "Fork a public-catalog contract into the caller's org",
            "description": "id is a public_contracts id, not a tenant contract id.",
            "parameters": [id_param.clone()],
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "fork_filter": {"type": ["object", "null"], "properties": {"fields": {"type": ["array", "null"], "items": {"type": "string"}}, "predicates": {"type": "array", "items": {"type": "object", "properties": {"field": {"type": "string"}, "op": {"type": "string", "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "in"]}, "value": {}}}}}}}}}}},
            "responses": {
                "201": {"description": "Forked contract", "content": {"application/json": {"schema": {"type": "object", "properties": {"contract_id": {"type": "string", "format": "uuid"}, "name": {"type": "string"}, "parent_public_contract_id": {"type": "string", "format": "uuid"}, "fork_filter": {"type": ["object", "null"]}, "created_at": {"type": "string", "format": "date-time"}}}}}},
                "400": {"description": "Invalid body"},
                "401": unauthorized.clone(),
                "404": {"description": "Public contract not found"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/audit",
        "get",
        json!({
            "operationId": "list_audit",
            "summary": "Recent audit log entries for the caller's org",
            "parameters": [
                {"name": "contract_id", "in": "query", "required": false, "schema": {"type": "string", "format": "uuid"}},
                {"name": "limit", "in": "query", "required": false, "schema": {"type": "integer", "default": 50, "maximum": 500}},
                {"name": "offset", "in": "query", "required": false, "schema": {"type": "integer", "default": 0}}
            ],
            "responses": {
                "200": {"description": "Audit rows", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "contract_id": {"type": "string", "format": "uuid"}, "contract_version": {"type": ["string", "null"]}, "passed": {"type": "boolean"}, "violation_count": {"type": "integer"}, "violation_details": {}, "raw_event": {}, "validation_us": {"type": "integer"}, "source_ip": {"type": ["string", "null"]}, "created_at": {"type": "string", "format": "date-time"}}}}}}},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/stats",
        "get",
        json!({
            "operationId": "get_stats",
            "summary": "Org-wide ingestion stats",
            "responses": {
                "200": {"description": "Totals and latency percentiles", "content": {"application/json": {"schema": {"type": "object", "properties": {"total_events": {"type": "integer"}, "passed_events": {"type": "integer"}, "failed_events": {"type": "integer"}, "pass_rate": {"type": "number"}, "avg_validation_us": {"type": "number"}, "p50_validation_us": {"type": "integer"}, "p95_validation_us": {"type": "integer"}, "p99_validation_us": {"type": "integer"}}}}}},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/playground/validate",
        "post",
        json!({
            "operationId": "playground_validate",
            "summary": "Validate one event against unsaved YAML",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["yaml_content", "event"], "properties": {"yaml_content": {"type": "string"}, "event": {"type": "object"}}}}}},
            "responses": {
                "200": {"description": "Validation result plus the post-transform event", "content": {"application/json": {"schema": {"type": "object", "properties": {"passed": {"type": "boolean"}, "violations": {"type": "array", "items": {"type": "object", "properties": {"field": {"type": "string"}, "message": {"type": "string"}, "kind": {"type": "string"}}}}, "validation_us": {"type": "integer"}, "transformed_event": {}}}}}},
                "400": {"description": "Invalid contract YAML"},
                "401": unauthorized.clone()
            },
            "security": auth.clone()
        }),
    );
    let infer_ok = json!({"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "sample_count": {"type": "integer"}}});
    merge_openapi_op(
        paths,
        "/contracts/infer",
        "post",
        json!({
            "operationId": "infer_json",
            "summary": "Infer a draft contract from JSON samples",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "samples"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "samples": {"type": "array", "items": {"type": "object"}}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": infer_ok}}}, "400": {"description": "No samples, or a sample is not an object"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/infer/avro",
        "post",
        json!({
            "operationId": "infer_avro",
            "summary": "Infer a draft contract from an Avro schema or samples",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "schema": {"type": ["string", "null"]}, "samples": {"type": "array", "items": {"type": "object"}}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "mode": {"type": "string", "enum": ["schema", "samples"]}}}}}}, "400": {"description": "Invalid Avro schema or samples"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/infer/proto",
        "post",
        json!({
            "operationId": "infer_proto",
            "summary": "Infer a draft contract from a proto3 file",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "proto_source"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "proto_source": {"type": "string"}, "message": {"type": ["string", "null"]}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "message_used": {"type": "string"}}}}}}, "400": {"description": "Empty or unparseable proto"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/infer/openapi",
        "post",
        json!({
            "operationId": "infer_openapi_document",
            "summary": "Infer a draft contract from an OpenAPI or AsyncAPI document",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "openapi_source"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "openapi_source": {"type": "string"}, "schema_name": {"type": ["string", "null"]}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "schema_used": {"type": "string"}}}}}}, "400": {"description": "Empty or unparseable document"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/infer/csv",
        "post",
        json!({
            "operationId": "infer_csv",
            "summary": "Infer a draft contract from CSV text",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "csv_content": {"type": ["string", "null"]}, "base64": {"type": ["string", "null"]}, "delimiter": {"type": ["string", "null"]}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "sample_count": {"type": "integer"}}}}}}, "400": {"description": "Empty CSV, too large, or neither csv_content nor base64"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/contracts/infer/url",
        "post",
        json!({
            "operationId": "infer_url",
            "summary": "Infer a draft contract by fetching a URL",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["name", "url"], "properties": {"name": {"type": "string"}, "description": {"type": ["string", "null"]}, "url": {"type": "string"}, "headers": {"type": "object", "additionalProperties": {"type": "string"}}}}}}},
            "responses": {"200": {"description": "Draft YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"yaml_content": {"type": "string"}, "field_count": {"type": "integer"}, "sample_count": {"type": "integer"}, "detected_format": {"type": "string", "enum": ["json", "csv"]}}}}}}, "400": {"description": "Invalid URL, blocked host, or unusable body"}, "401": unauthorized.clone()},
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/v1/contracts/{contract_id}/version",
        "get",
        json!({
            "operationId": "probe_contract_version",
            "summary": "Latest stable version and YAML hash (SMT reload probe)",
            "parameters": [{"name": "contract_id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
            "responses": {
                "200": {"description": "version plus sha256 hex of the YAML", "content": {"application/json": {"schema": {"type": "object", "properties": {"version": {"type": "string"}, "hash": {"type": "string"}}}}}},
                "401": unauthorized.clone(),
                "404": {"description": "Contract not found"},
                "409": {"description": "No stable version"}
            },
            "security": auth.clone()
        }),
    );
    merge_openapi_op(
        paths,
        "/v1/contracts/{id}/passed",
        "get",
        crate::passed_events::openapi_operation(),
    );
    merge_openapi_op(
        paths,
        "/v1/integrations/fivetran/connect",
        "post",
        crate::fivetran::openapi_operation(),
    );
    crate::openapi_schemas::apply(paths);
}

// ---------------------------------------------------------------------------
// Parallel validation helper (shared with ingest.rs pattern)
// ---------------------------------------------------------------------------

async fn parallel_validate(
    compiled: Arc<CompiledContract>,
    events: &[Value],
) -> AppResult<Vec<ValidationResult>> {
    let events = events.to_vec();
    tokio::task::spawn_blocking(move || events.par_iter().map(|e| validate(&compiled, e)).collect())
        .await
        .map_err(|e| AppError::Internal(format!("Validation task join error: {e}")))
}

// ---------------------------------------------------------------------------
// Version resolution (RFC-002 + RFC-021 `?version=` param)
// ---------------------------------------------------------------------------

async fn resolve_version(
    state: &AppState,
    contract_id: Uuid,
    pin: Option<(String, &'static str)>,
) -> AppResult<(String, &'static str)> {
    if let Some(p) = pin {
        return Ok(p);
    }
    let latest = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok((latest.version, "default_stable"))
}

// ---------------------------------------------------------------------------
// Main handler
// ---------------------------------------------------------------------------

pub async fn v1_ingest_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id_str): Path<String>,
    Query(query): Query<V1IngestQuery>,
    headers: HeaderMap,
    key_ext: Option<Extension<ValidatedKey>>,
    rl_ext: Option<Extension<crate::rate_limit::RateLimitOutcome>>,
    body: Bytes,
) -> AppResult<Response> {
    // --- 1. Parse + validate contract_id -----------------------------------
    // D1: accept `{id}@{version}` like /ingest and /egress.
    let (contract_id, path_version) = crate::version_pin::parse_pinned_id(&contract_id_str)?;

    // --- 2. Rate limit -----------------------------------------------------
    let (rl_rps, rl_burst, key_id, org_id) = if let Some(Extension(ref k)) = key_ext {
        (
            k.rate_limit_rps,
            k.rate_limit_burst,
            Some(k.api_key_id),
            Some(k.org_id),
        )
    } else {
        (None, None, None, None)
    };

    // `require_api_key` already spent this request's token and left the
    // outcome in the extensions; reuse it. Checking again would charge two
    // tokens per request (E2E retest 2026-10-13: Remaining stepped by 2 and
    // Free keys got ~5 req/s instead of 10). Only the dev no-auth path has
    // no outcome; it uses a nil-UUID bucket.
    let rl_outcome = match rl_ext {
        Some(Extension(outcome)) => outcome,
        None => {
            let bucket_id = key_id.unwrap_or(Uuid::nil());
            state.rate_limiter.check(bucket_id, rl_rps, rl_burst)
        }
    };
    let rl_headers = rate_limit_headers(&rl_outcome);

    if !rl_outcome.allowed {
        let body = json!({
            "error": "rate_limit_exceeded",
            "detail": format!(
                "Rate limit of {} req/sec (bursts up to {}) exceeded. Retry after {}ms.",
                rl_outcome.rate, rl_outcome.limit, rl_outcome.retry_after_ms
            ),
            "retry_after_ms": rl_outcome.retry_after_ms,
        });
        // RFC-103: include Retry-After (seconds, RFC 6585) alongside the
        // IETF-draft X-RateLimit-* headers so clients can back off.
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            rl_outcome.headers(),
            Json(body),
        )
            .into_response());
    }

    // --- 3. Idempotency-Key lookup -----------------------------------------
    let idem_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Validate key length before any DB touch.
    if let Some(ref k) = idem_key {
        if k.len() > 255 {
            return Err(AppError::BadRequest(
                "Idempotency-Key must be ≤ 255 characters".into(),
            ));
        }
    }

    let body_hash = idempotency::body_hash(&body);

    if let Some(ref key) = idem_key {
        match idempotency::lookup(&state.db, key, contract_id, &body_hash).await {
            Ok(idempotency::IdempotencyLookup::Hit {
                status_code,
                response,
            }) => {
                // Replay: return cached response.
                let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
                return Ok((
                    status,
                    [
                        ("X-Idempotency-Replay", "true".to_string()),
                        ("X-RateLimit-Limit", rl_headers[0].1.clone()),
                        ("X-RateLimit-Remaining", rl_headers[1].1.clone()),
                        ("X-RateLimit-Reset", rl_headers[2].1.clone()),
                    ],
                    Json(response),
                )
                    .into_response());
            }
            Ok(idempotency::IdempotencyLookup::Conflict) => {
                let body = json!({
                    "error": "idempotency_conflict",
                    "detail": "A different request body was already submitted with this Idempotency-Key."
                });
                return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
            }
            Ok(idempotency::IdempotencyLookup::Miss) => {
                // Fall through to normal processing.
            }
            Err(e) => {
                // Log and fall through — idempotency is best-effort; don't
                // block the request on a DB error.
                tracing::warn!("idempotency lookup failed: {e}");
            }
        }
    }

    // --- 4. Parse body (body-size limit already enforced by middleware) ----
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let events = parse_body(content_type, &body)?;

    // --- 5. Per-event size check (JSON path only — NDJSON checked inline) --
    if !content_type.contains("application/x-ndjson") {
        check_event_sizes(&events)?;
    }

    // --- 6. Batch count check ---------------------------------------------
    if events.is_empty() {
        return Err(AppError::BadRequest("Empty event batch".into()));
    }
    if events.len() > MAX_BATCH_SIZE {
        return Err(AppError::BadRequest(format!(
            "Batch too large: {} events submitted, maximum is {}",
            events.len(),
            MAX_BATCH_SIZE,
        )));
    }

    // --- 6b. RFC-083 Phase 2: plan monthly event cap ----------------------
    // dry_run is not billable — over-cap orgs may still validate.
    if !query.dry_run {
        crate::metering::enforce_plan_limit(&state.db, org_id, state.self_hosted).await?;
    }

    // --- 7. Load contract identity + scope check --------------------------
    // RFC-074: org-scoped identity load (404 if the caller's org doesn't own
    // it) closes the cross-tenant write hole; RFC-065 per-key allow-list still
    // applies on top.
    let identity: ContractIdentity =
        storage::get_contract_identity(&state.db, contract_id, org_id).await?;

    // API key contract-scope enforcement (RFC-065 shared helper).
    if let Some(Extension(ref k)) = key_ext {
        if !k.permits_contract(contract_id) {
            return Err(AppError::Unauthorized);
        }
    }

    // --- 8. Resolve version -----------------------------------------------
    let (resolved_version, pin_source) =
        resolve_version(&state, contract_id, pick_pin(query.version, path_version)).await?;

    let version_row =
        storage::get_version_for_pin(&state.db, contract_id, &resolved_version, org_id).await?;
    // A normalised pin (`1.0.0` for a stored `1.0`) continues as the stored string;
    // the caller's spelling is echoed back as `requested_version`.
    let requested_version =
        (resolved_version != version_row.version).then(|| resolved_version.clone());
    let resolved_version = version_row.version.clone();

    tracing::debug!(
        contract_id = %contract_id,
        version = %resolved_version,
        pin_source = pin_source,
        state = version_row.state.as_str(),
        batch_size = events.len(),
        "v1 ingest request routed"
    );

    // Source IP for audit rows.
    let source_ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    // RFC-086: gate event-body storage for this org/contract (same rule as the
    // v0 ingest path — a paid, opted-in org stores; everyone else is redacted).
    let store_payloads =
        crate::ingest::resolve_store_payloads(&state, org_id, identity.store_event_payloads).await;

    // Deprecated-pin: quarantine wholesale (reuse existing logic shape).
    if version_row.state == VersionState::Deprecated {
        return deprecated_quarantine(
            &state,
            contract_id,
            org_id,
            &resolved_version,
            events,
            source_ip,
            query.dry_run,
            store_payloads,
            &headers,
            &version_row.yaml_content,
            identity.fail_policy.as_ref(),
            query.on_fail.as_deref(),
        )
        .await;
    }

    // --- 9. Compile + validate --------------------------------------------
    let compiled = state.get_compiled(contract_id, &resolved_version).await?;
    let mut compiled_by_version: HashMap<String, Arc<CompiledContract>> = HashMap::new();
    compiled_by_version.insert(resolved_version.clone(), Arc::clone(&compiled));

    // RFC-016: time the top-level validation call.
    let _validation_start = Instant::now();
    let mut validation_results = parallel_validate(Arc::clone(&compiled), &events).await?;
    {
        let elapsed = _validation_start.elapsed().as_secs_f64();
        let outcome = if validation_results.iter().all(|r| r.passed) {
            "passed"
        } else {
            "failed"
        };
        metrics::histogram!(
            "contractgate_validation_duration_seconds",
            "contract_id" => contract_id.to_string(),
            "outcome" => outcome,
        )
        .record(elapsed);
    }

    // Fallback multi-stable resolution (RFC-002, same logic as ingest.rs).
    let mut per_event_versions = vec![resolved_version.clone(); events.len()];

    let fallback_eligible = identity.multi_stable_resolution == MultiStableResolution::Fallback
        && pin_source == "default_stable"
        && validation_results.iter().any(|r| !r.passed);

    if fallback_eligible {
        let stables = storage::list_stable_versions(&state.db, contract_id).await?;
        let other_stables: Vec<_> = stables
            .into_iter()
            .filter(|v| v.version != resolved_version)
            .collect();

        if !other_stables.is_empty() {
            let mut compiled_fallbacks: Vec<(String, Arc<CompiledContract>)> =
                Vec::with_capacity(other_stables.len());
            for v in &other_stables {
                let cc = state.get_compiled(contract_id, &v.version).await?;
                compiled_by_version.insert(v.version.clone(), Arc::clone(&cc));
                compiled_fallbacks.push((v.version.clone(), cc));
            }
            for (idx, result) in validation_results.iter_mut().enumerate() {
                if result.passed {
                    continue;
                }
                let event = &events[idx];
                let candidate_results: Vec<(String, ValidationResult)> = compiled_fallbacks
                    .par_iter()
                    .map(|(ver, cc)| (ver.clone(), validate(cc, event)))
                    .collect();
                if let Some((winning_version, winning_vr)) =
                    candidate_results.into_iter().find(|(_, vr)| vr.passed)
                {
                    *result = winning_vr;
                    per_event_versions[idx] = winning_version;
                }
            }
        }
    }

    // Uniqueness check (RFC quality).
    let uniqueness_violations = check_uniqueness_batch(&compiled.contract.quality, &events);
    for (idx, violation) in uniqueness_violations {
        if let Some(vr) = validation_results.get_mut(idx) {
            vr.violations.push(violation);
            vr.passed = false;
        }
    }

    // --- 10. RFC-004 transforms -------------------------------------------
    let transformed_payloads: Vec<TransformedPayload> = events
        .iter()
        .enumerate()
        .map(|(idx, event)| {
            let version = &per_event_versions[idx];
            let cc = compiled_by_version
                .get(version)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&compiled));
            apply_transforms(&cc, event.clone())
        })
        .collect();

    // --- Roll-up counts ---------------------------------------------------
    let total = events.len();
    let passed_count = validation_results.iter().filter(|r| r.passed).count();
    let failed_count = total - passed_count;
    let decision = crate::fail_policy::resolve_request(
        &state,
        org_id,
        contract_id,
        &compiled.contract,
        &version_row.yaml_content,
        identity.fail_policy.as_ref(),
        query.on_fail.as_deref(),
        &headers,
        query.dry_run,
        failed_count,
    )
    .await?;
    let failed_indices: Vec<usize> = validation_results
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.passed)
        .map(|(i, _)| i)
        .collect();

    // --- Atomic short-circuit ---------------------------------------------
    let atomic_rejected = query.atomic && failed_count > 0;

    // --- 11. Pre-assign quarantine UUIDs ----------------------------------
    // Only when a quarantine row will actually be written: dry runs and
    // atomic rejections store none, and an id that points at nothing can't
    // be fetched or replayed (E2E retest 2026-10-13).
    let mut quarantine_ids: Vec<Option<Uuid>> = vec![None; total];
    if !query.dry_run
        && !atomic_rejected
        && decision.mode == crate::contract::OnFailMode::Quarantine
    {
        for &idx in &failed_indices {
            quarantine_ids[idx] = Some(Uuid::new_v4());
        }
    }

    // --- 12. Persist (fire-and-forget) ------------------------------------
    if !query.dry_run && !atomic_rejected {
        let audit_rows: Vec<storage::AuditEntryInsert> = validation_results
            .iter()
            .enumerate()
            .filter(|(_, vr)| crate::drop_mode::writes_audit(vr.passed, decision.mode, false))
            .map(|(idx, vr)| storage::AuditEntryInsert {
                contract_id,
                org_id,
                contract_version: per_event_versions[idx].clone(),
                passed: vr.passed,
                violation_count: vr.violations.len() as i32,
                violation_details: serde_json::to_value(&vr.violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                raw_event: transformed_payloads[idx].clone(),
                validation_us: vr.validation_us as i64,
                source_ip: source_ip.clone(),
                source: "http".to_string(),
                pre_assigned_id: None,
                replay_of_quarantine_id: None,
                direction: "ingress".to_string(),
            })
            .collect();

        let quarantine_rows: Vec<storage::QuarantineEventInsert> = failed_indices
            .iter()
            .filter(|_| decision.mode == crate::contract::OnFailMode::Quarantine)
            .map(|&idx| storage::QuarantineEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: transformed_payloads[idx].clone(),
                violation_count: validation_results[idx].violations.len() as i32,
                violation_details: serde_json::to_value(&validation_results[idx].violations)
                    .unwrap_or_else(|_| Value::Array(vec![])),
                validation_us: validation_results[idx].validation_us as i64,
                source_ip: source_ip.clone(),
                replay_of_quarantine_id: None,
                pre_assigned_id: quarantine_ids[idx], // pre-assigned so we can return it
                direction: "ingress".to_string(),
            })
            .collect();

        let forward_rows: Vec<storage::ForwardEventInsert> = validation_results
            .iter()
            .enumerate()
            .filter(|(_, vr)| vr.passed)
            .map(|(idx, _)| storage::ForwardEventInsert {
                contract_id,
                contract_version: per_event_versions[idx].clone(),
                payload: transformed_payloads[idx].clone(),
            })
            .collect();

        // RFC-016: emit violation + quarantine counters at the audit write path.
        // `serde_json::to_string` gives the snake_case label (rename_all = "snake_case").
        {
            let cid = contract_id.to_string();
            for vr in &validation_results {
                for violation in &vr.violations {
                    let kind = serde_json::to_string(&violation.kind)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!(
                        "contractgate_violations_total",
                        "contract_id" => cid.clone(),
                        "kind" => kind,
                    )
                    .increment(1);
                }
            }
            if !quarantine_rows.is_empty() {
                metrics::counter!(
                    "contractgate_quarantined_total",
                    "contract_id" => cid,
                )
                .increment(quarantine_rows.len() as u64);
            }
        }

        if !audit_rows.is_empty() {
            let n = audit_rows.len();
            let pool = state.db.clone();
            let meter_org = org_id;
            tokio::spawn(async move {
                if let Err(e) =
                    storage::log_audit_entries_batch(&pool, &audit_rows, store_payloads).await
                {
                    tracing::warn!("v1 ingest: audit write failed: {e:?}");
                    return;
                }
                crate::metering::record_batch_usage_blocking(&pool, meter_org, n).await;
            });
        }
        if !quarantine_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    storage::quarantine_events_batch(&pool, &quarantine_rows, store_payloads).await
                {
                    tracing::warn!("v1 ingest: quarantine write failed: {e:?}");
                }
            });
        }
        if !forward_rows.is_empty() {
            let pool = state.db.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    storage::forward_events_batch(&pool, &forward_rows, store_payloads).await
                {
                    tracing::warn!("v1 ingest: forward write failed: {e:?}");
                }
            });
        }
    } else if crate::drop_mode::writes_atomic_summary(decision.mode, query.dry_run, failed_count) {
        // Atomic summary audit row.
        let details: Vec<Value> = failed_indices
            .iter()
            .map(|&idx| {
                json!({
                    "index": idx,
                    "violations": validation_results[idx].violations,
                })
            })
            .collect();
        let summary = TransformedPayload::from_stored(json!({
            "batch_rejected": true,
            "atomic": true,
            "batch_size": total,
            "failing_count": failed_indices.len(),
        }));
        let pool = state.db.clone();
        let version = resolved_version.clone();
        let violation_count = failed_indices.len() as i32;
        let details_value = Value::Array(details);
        tokio::spawn(async move {
            if let Err(e) = storage::log_audit_entry(
                &pool,
                contract_id,
                org_id,
                &version,
                false,
                violation_count,
                details_value,
                summary,
                0,
                source_ip.as_deref(),
                "http",
                "ingress",
            )
            .await
            {
                tracing::warn!("v1 ingest: atomic audit write failed: {e:?}");
            }
        });
    }

    // --- Build per-event result list --------------------------------------
    let per_event_results: Vec<V1IngestEventResult> = validation_results
        .iter()
        .enumerate()
        .map(|(idx, vr)| V1IngestEventResult {
            index: idx,
            passed: vr.passed,
            violations: vr.violations.clone(),
            validation_us: vr.validation_us,
            forwarded: vr.passed && !query.dry_run && !atomic_rejected,
            contract_version: per_event_versions[idx].clone(),
            quarantine_id: quarantine_ids[idx],
            transformed_event: transformed_payloads[idx].as_value().clone(),
            disposition: crate::drop_mode::per_record_disposition(vr.passed, decision.mode),
        })
        .collect();

    // --- 13. Compose response + idempotency store -------------------------
    let http_status = if atomic_rejected {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if failed_count == 0 {
        StatusCode::OK
    } else if passed_count == 0 {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::MULTI_STATUS
    };

    if decision.dropped > 0 {
        let mut by_version: HashMap<String, Vec<crate::fail_policy::FailureFacts>> = HashMap::new();
        for (idx, vr) in validation_results.iter().enumerate() {
            if !vr.passed {
                by_version
                    .entry(per_event_versions[idx].clone())
                    .or_default()
                    .push(crate::fail_policy::failure_facts(&vr.violations));
            }
        }
        for (version, failures) in by_version {
            if let Err(err) = crate::fail_policy::record_drops(
                &state,
                org_id,
                contract_id,
                &version,
                "ingress",
                decision.source.as_str(),
                decision.sample_drops,
                &failures,
            )
            .await
            {
                tracing::warn!("v1 drop ledger write failed: {err:?}");
            }
        }
    }
    if crate::drop_mode::records_quarantine_volume(decision.quarantined, atomic_rejected) {
        if let Err(err) = crate::fail_policy::record_quarantined(
            &state,
            org_id,
            contract_id,
            decision.quarantined,
        )
        .await
        {
            tracing::warn!("v1 quarantine volume write failed: {err:?}");
        }
    }

    let response_body = V1IngestResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        quarantined: decision.quarantined,
        dropped: decision.dropped,
        on_fail_mode: decision.mode.as_str(),
        on_fail_source: decision.source.as_str(),
        auto_trip: crate::fail_policy::auto_trip_body(decision.auto_trip.as_ref()),
        dry_run: query.dry_run,
        atomic: query.atomic,
        resolved_version: resolved_version.clone(),
        requested_version: requested_version.clone(),
        version_pin_source: pin_source,
        results: per_event_results,
    };
    let response_value = serde_json::to_value(&response_body)
        .unwrap_or_else(|_| json!({"error": "serialization_error"}));

    // Store idempotency record (unless dry_run, confirmed 2026-05-01).
    if let Some(ref key) = idem_key {
        if !query.dry_run {
            let db = state.db.clone();
            let key_owned = key.clone();
            let hash_owned = body_hash.clone();
            let status_u16 = http_status.as_u16();
            let resp_clone = response_value.clone();
            tokio::spawn(async move {
                if let Err(e) = idempotency::store(
                    &db,
                    &key_owned,
                    contract_id,
                    &hash_owned,
                    status_u16,
                    &resp_clone,
                )
                .await
                {
                    tracing::warn!("idempotency store failed: {e}");
                }
            });
        }
    }

    Ok((
        http_status,
        [
            ("X-RateLimit-Limit", rl_headers[0].1.clone()),
            ("X-RateLimit-Remaining", rl_headers[1].1.clone()),
            ("X-RateLimit-Reset", rl_headers[2].1.clone()),
        ],
        Json(response_value),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Deprecated-version wholesale quarantine
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn deprecated_quarantine(
    state: &AppState,
    contract_id: Uuid,
    org_id: Option<Uuid>,
    pinned_version: &str,
    events: Vec<Value>,
    source_ip: Option<String>,
    dry_run: bool,
    store_payloads: bool,
    headers: &HeaderMap,
    yaml: &str,
    fail_policy: Option<&Value>,
    query_on_fail: Option<&str>,
) -> AppResult<Response> {
    let total = events.len();
    let compiled = state.get_compiled(contract_id, pinned_version).await?;
    let decision = crate::fail_policy::resolve_request(
        state,
        org_id,
        contract_id,
        &compiled.contract,
        yaml,
        fail_policy,
        query_on_fail,
        headers,
        dry_run,
        total,
    )
    .await?;

    let transformed: Vec<TransformedPayload> = events
        .iter()
        .map(|e| apply_transforms(&compiled, e.clone()))
        .collect();

    let per_event: Vec<V1IngestEventResult> = (0..total)
        .map(|idx| V1IngestEventResult {
            index: idx,
            passed: false,
            violations: vec![crate::version_pin::deprecated_violation(
                pinned_version,
                None,
            )],
            validation_us: 0,
            forwarded: false,
            contract_version: pinned_version.to_string(),
            quarantine_id: None,
            transformed_event: transformed[idx].as_value().clone(),
            disposition: crate::drop_mode::per_record_disposition(false, decision.mode),
        })
        .collect();

    if !dry_run && decision.mode == crate::contract::OnFailMode::Quarantine {
        let synthetic = json!({
            "kind": "deprecated_contract_version",
            "pinned_version": pinned_version,
        });
        let qrows: Vec<storage::QuarantineEventInsert> = transformed
            .iter()
            .map(|tp| storage::QuarantineEventInsert {
                contract_id,
                contract_version: pinned_version.to_string(),
                payload: tp.clone(),
                violation_count: 1,
                violation_details: Value::Array(vec![synthetic.clone()]),
                validation_us: 0,
                source_ip: source_ip.clone(),
                replay_of_quarantine_id: None,
                pre_assigned_id: None,
                direction: "ingress".to_string(),
            })
            .collect();
        let pool = state.db.clone();
        tokio::spawn(async move {
            if let Err(e) = storage::quarantine_events_batch(&pool, &qrows, store_payloads).await {
                tracing::warn!("v1 ingest: deprecated quarantine write failed: {e:?}");
            }
        });
    }

    if decision.dropped > 0 {
        let failure = crate::fail_policy::FailureFacts {
            violations: vec![("deprecated_contract_version".into(), String::new())],
        };
        let failures = vec![failure; total];
        if let Err(err) = crate::fail_policy::record_drops(
            state,
            org_id,
            contract_id,
            pinned_version,
            "ingress",
            decision.source.as_str(),
            decision.sample_drops,
            &failures,
        )
        .await
        {
            tracing::warn!("v1 deprecated-pin drop ledger write failed: {err:?}");
        }
    }

    let body = json!({
        "error": deprecated_message(pinned_version, contract_id, dry_run),
        "code": "contract_version_deprecated",
        "deprecated_version": pinned_version,
        "total": total,
        "passed": 0,
        "failed": total,
        "quarantined": decision.quarantined,
        "dropped": decision.dropped,
        "on_fail_mode": decision.mode.as_str(),
        "on_fail_source": decision.source.as_str(),
        "auto_trip": crate::fail_policy::auto_trip_body(decision.auto_trip.as_ref()),
        "dry_run": dry_run,
        "version_pin_source": "pinned_deprecated",
        "results": per_event,
    });

    Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

/// `?version=` wins over an `@version` path suffix (it is the documented
/// /v1 form); both name the same thing.
/// The pin and where it came from: `?version=` (`"query_param"`) wins over
/// the `@version` path suffix (`"path"`).
fn pick_pin(query: Option<String>, path: Option<String>) -> Option<(String, &'static str)> {
    if query.is_some() && path.is_some() && query != path {
        tracing::warn!("both ?version= and @version given on /v1/ingest; ?version wins");
    }
    query
        .map(|v| (v, "query_param"))
        .or(path.map(|v| (v, "path")))
}

/// R-X2b: explain a deprecated pin instead of a bare 422 with empty
/// per-event violations.
fn deprecated_message(pinned_version: &str, contract_id: Uuid, dry_run: bool) -> String {
    let fate = if dry_run {
        "dry run: nothing was quarantined"
    } else {
        "every event was quarantined"
    };
    crate::version_pin::deprecated_message(pinned_version, contract_id, fate)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deprecated_message_explains_and_says_what_to_do() {
        let id = Uuid::nil();
        let m = deprecated_message("1.0", id, false);
        assert!(m.contains("1.0") && m.contains("deprecated") && m.contains("quarantined"));
        assert!(m.contains("?version="));
        assert_eq!(
            pick_pin(None, Some("1.0".into())),
            Some(("1.0".into(), "path"))
        );
        assert_eq!(
            pick_pin(Some("2.0".into()), Some("1.0".into())),
            Some(("2.0".into(), "query_param"))
        );
        assert_eq!(
            pick_pin(Some("2.0".into()), None),
            Some(("2.0".into(), "query_param"))
        );
        assert_eq!(pick_pin(None, None), None);
        assert!(deprecated_message("1.0", id, true).contains("nothing was quarantined"));
    }

    #[test]
    fn content_type_json_ndjson_or_missing_accepted_others_415() {
        let body = br#"{"a":1}"#;
        for ct in [
            "",
            "application/json",
            "application/json; charset=utf-8",
            "application/vnd.api+json",
        ] {
            assert!(parse_body(ct, body).is_ok(), "{ct:?} should be accepted");
        }
        assert!(parse_body("application/x-ndjson", b"{\"a\":1}\n").is_ok());
        for ct in [
            "text/plain",
            "application/xml",
            "text/xml",
            "application/x-www-form-urlencoded",
        ] {
            assert!(
                matches!(parse_body(ct, body), Err(AppError::UnsupportedMediaType(_))),
                "{ct:?} should be 415"
            );
        }
    }

    #[test]
    fn json_array_parsed() {
        let body = br#"[{"a":1},{"b":2}]"#;
        let events = parse_body("application/json", body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn json_single_object_parsed_as_batch_of_one() {
        let body = br#"{"a":1}"#;
        let events = parse_body("application/json", body).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn ndjson_parsed() {
        let body = b"{\"a\":1}\n{\"b\":2}\n";
        let events = parse_body("application/x-ndjson", body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ndjson_trailing_newline_ignored() {
        let body = b"{\"a\":1}\n{\"b\":2}\n\n";
        let events = parse_ndjson(body).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ndjson_bad_line_returns_error() {
        let body = b"{\"a\":1}\nnot json\n";
        let err = parse_ndjson(body).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("line 1"),
            "expected line index in error, got: {msg}"
        );
    }

    #[test]
    fn event_over_1mb_rejected() {
        // Build a JSON object > 1 MB.
        let big = format!(r#"{{"data":"{}"}}"#, "x".repeat(MAX_EVENT_BYTES + 1));
        let events = vec![serde_json::from_str(&big).unwrap()];
        let err = check_event_sizes(&events).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("1 MB"),
            "expected size limit message, got: {msg}"
        );
    }

    #[test]
    fn rate_limit_headers_format() {
        use crate::rate_limit::{RateLimitOutcome, DEFAULT_RATE_LIMIT_RPS};
        let outcome = RateLimitOutcome {
            allowed: true,
            rate: DEFAULT_RATE_LIMIT_RPS,
            limit: DEFAULT_RATE_LIMIT_RPS,
            remaining: 999,
            reset_unix: 1_714_000_000,
            retry_after_ms: 0,
        };
        let hdrs = rate_limit_headers(&outcome);
        assert_eq!(hdrs[0].0, "X-RateLimit-Limit");
        assert_eq!(hdrs[1].0, "X-RateLimit-Remaining");
        assert_eq!(hdrs[2].0, "X-RateLimit-Reset");
    }

    #[test]
    fn openapi_passed_events_schema() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let op = &v["paths"]["/v1/contracts/{id}/passed"]["get"];
        let schema = &op["responses"]["200"]["content"]["application/json"]["schema"];
        for field in ["events", "next_cursor", "has_more"] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == field),
                "missing {field}"
            );
        }
        let item = &schema["properties"]["events"]["items"];
        assert_eq!(
            item["properties"]["payload"]["type"],
            json!(["object", "null"])
        );
        let forbidden = op["responses"]["403"]["description"].as_str().unwrap();
        assert!(forbidden.contains(&crate::plan::passed_events_upgrade_message()));
        for plan in crate::plan::PAID_PLANS {
            assert!(forbidden.contains(plan), "{forbidden}");
        }
        let bad_request = op["responses"]["400"]["description"].as_str().unwrap();
        assert!(bad_request.contains("re-sync"));
        assert!(bad_request.contains("invalid cursor"));
        assert!(bad_request.contains("limit must be an integer 1-1000"));
        let next_cursor = &schema["properties"]["next_cursor"];
        assert_eq!(next_cursor["type"], json!(["string", "null"]));
        let cursor_doc = next_cursor["description"].as_str().unwrap();
        assert!(
            cursor_doc.contains("even if has_more is false"),
            "{cursor_doc}"
        );
        assert!(cursor_doc.contains("request cursor"), "{cursor_doc}");
        assert!(op["responses"]["410"]["description"]
            .as_str()
            .unwrap()
            .contains("re-sync"));
    }

    #[test]
    fn openapi_spec_is_valid_json() {
        let spec = build_openapi();
        let parsed: Result<Value, _> = serde_json::from_str(&spec);
        assert!(parsed.is_ok(), "openapi spec must be valid JSON");
        let v = parsed.unwrap();
        assert_eq!(v["openapi"], "3.1.0");
    }

    // contractgate.io is an unrelated company's product; clients and codegen
    // read this URL, so it must be ours.
    #[test]
    fn openapi_server_is_contractgate_cloud() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        assert_eq!(v["servers"][0]["url"], "https://api.datacontractgate.com");
    }

    // AISchemaGate's SDK calls /egress; the spec must list it next to ingest.
    #[test]
    fn openapi_lists_ingest_and_egress() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        assert!(v["paths"]["/v1/ingest/{contract_id}"]["post"].is_object());
        assert!(v["paths"]["/egress/{contract_id}"]["post"].is_object());
    }

    #[test]
    fn openapi_lists_fivetran_connect_with_schemas() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let op = &v["paths"]["/v1/integrations/fivetran/connect"]["post"];
        let request = &op["requestBody"]["content"]["application/json"]["schema"];
        let ok = &op["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(request["type"], "object");
        assert_eq!(ok["type"], "object");
        assert!(request["properties"].as_object().unwrap().len() >= 4);
        assert!(ok["properties"].as_object().unwrap().len() >= 3);
        let forbidden = op["responses"]["403"]["description"].as_str().unwrap();
        assert!(forbidden.contains(&crate::plan::fivetran_connect_upgrade_message()));
    }

    #[test]
    fn openapi_lists_publication_endpoints() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        for path in [
            "/published/{publication_ref}",
            "/catalog",
            "/contracts/publications",
            "/contracts/publications/{publication_ref}",
            "/contracts/import-published",
            "/contracts/{id}/versions/{version}/publish",
            "/contracts/{name}/collaborators",
            "/contracts/{name}/collaborators/{org_id}",
            "/contracts/{name}/comments",
            "/contracts/{name}/comments/{id}",
            "/contracts/{name}/proposals",
            "/contracts/{name}/proposals/{id}",
            "/me",
            "/v1/me",
            "/contracts/{id}/kafka-ingress",
            "/contracts/{id}/kafka-ingress/enable",
            "/contracts/{id}/kafka-ingress/disable",
            "/contracts/{id}/kinesis-ingress",
            "/contracts/{id}/kinesis-ingress/enable",
            "/contracts/{id}/kinesis-ingress/disable",
            "/contracts/{id}/kinesis-ingress/rotate-credentials",
            "/contracts/deploy",
            "/ingest/{contract_id}",
            "/contracts/{id}/versions",
            "/contracts/import",
            "/contracts/{id}/versions/latest-stable",
            "/contracts/{id}/versions/{version}",
            "/contracts/{id}/versions/{version}/promote",
            "/contracts/{id}/versions/{version}/deprecate",
            "/contracts/{id}/versions/{version}/approve-import",
            "/contracts/{id}/versions/{version}/odcs-conformance",
            "/contracts/{id}/versions/{version}/export",
            "/contracts/{id}/export",
        ] {
            assert!(v["paths"][path].is_object(), "openapi missing path {path}");
        }
    }

    /// Org-wide replay gained `redacted`; contract list documents the effective
    /// `store_event_payloads` (false when the plan cannot store).
    #[test]
    fn openapi_documents_redacted_replay_and_effective_payload_flag() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let replay = &v["paths"]["/quarantine/replay"]["post"];
        let desc = replay["description"].as_str().unwrap_or("");
        assert!(desc.contains("redacted"), "{desc}");
        let schema =
            replay["responses"]["200"]["content"]["application/json"]["schema"].to_string();
        assert!(schema.contains("redacted"), "{schema}");
        let list = v["paths"]["/contracts"]["get"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(list.contains("store_event_payloads"), "{list}");
        assert!(list.contains("free"), "{list}");
        assert!(
            v["paths"]["/contracts/{id}/quarantine/replay"]["post"].is_object(),
            "per-contract replay must stay in the spec"
        );
        let timed_out = v["paths"]["/contracts/{id}/versions/{version}/promote"]["post"]
            ["responses"]["504"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(timed_out.contains("pool_acquire"), "{timed_out}");
        assert!(timed_out.contains("lock_wait"), "{timed_out}");
    }

    /// Every `.route` in `build_router` is in the spec, with the same methods,
    /// and the spec has no extra path. Path-parameter names are ignored
    /// (`{raw_id}` and `{contract_id}` are the same route).
    #[test]
    fn openapi_matches_router() {
        let router = router_operations(include_str!("main.rs"));
        let spec = spec_operations(&serde_json::from_str(&build_openapi()).unwrap());
        let mut missing: Vec<String> = Vec::new();
        for (path, methods) in &router {
            match spec.get(path) {
                None => missing.push(format!("{path} {methods:?}")),
                Some(spec_methods) => {
                    for method in methods {
                        if !spec_methods.contains(method) {
                            missing.push(format!("{method} {path}"));
                        }
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "router routes missing from openapi:\n{}",
            missing.join("\n")
        );
        let mut extra: Vec<String> = Vec::new();
        for (path, methods) in &spec {
            match router.get(path) {
                None => extra.push(path.clone()),
                Some(router_methods) => {
                    for method in methods {
                        if !router_methods.contains(method) {
                            extra.push(format!("{method} {path}"));
                        }
                    }
                }
            }
        }
        assert!(
            extra.is_empty(),
            "openapi documents routes the router does not register:\n{}",
            extra.join("\n")
        );
    }

    #[test]
    fn openapi_payload_storage_audit_and_contract_get_schemas() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let settings = &v["paths"]["/settings/payload-storage"];
        assert!(
            settings["get"]["responses"]["200"]["content"]["application/json"]["schema"]
                ["properties"]["eligible"]
                .is_object()
        );
        let put = &settings["put"];
        assert_eq!(
            put["requestBody"]["content"]["application/json"]["schema"]["properties"]["enabled"]
                ["type"],
            "boolean"
        );
        assert_eq!(
            put["responses"]["400"]["description"].as_str().unwrap(),
            crate::plan::payload_storage_upgrade_message()
        );
        let patch = &v["paths"]["/contracts/{id}/payload-storage"]["patch"];
        assert_eq!(patch["parameters"][0]["name"], "id");
        assert_eq!(
            patch["requestBody"]["content"]["application/json"]["schema"]["properties"]["enabled"]
                ["type"],
            "boolean"
        );
        let purge = &v["paths"]["/contracts/{id}/purge-bodies"]["post"];
        let counts = &purge["responses"]["200"]["content"]["application/json"]["schema"];
        for field in [
            "quarantine_bodies_redacted",
            "audit_bodies_redacted",
            "forwarded_bodies_redacted",
        ] {
            assert_eq!(counts["properties"][field]["type"], "integer", "{field}");
            assert!(
                counts["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == field),
                "missing {field}"
            );
        }
        let audit = &v["paths"]["/audit"]["get"];
        let names: Vec<&str> = audit["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"contract_id"));
        assert!(names.contains(&"limit"));
        assert!(names.contains(&"offset"));
        assert_eq!(
            audit["responses"]["200"]["content"]["application/json"]["schema"]["items"]
                ["properties"]["passed"]["type"],
            "boolean"
        );
        let get = &v["paths"]["/contracts/{id}"]["get"];
        assert_eq!(
            get["responses"]["200"]["content"]["application/json"]["schema"]["properties"]
                ["store_event_payloads"]["type"],
            "boolean"
        );
        for plan in crate::plan::PAID_PLANS {
            let desc = get["description"].as_str().unwrap_or("");
            assert!(desc.contains(plan), "{desc} missing {plan}");
        }
    }

    /// QA gaps: replay 200/400, contract-list fields, `/me`, egress 207.
    #[test]
    fn openapi_replay_me_egress_and_contract_list_schemas() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let cap = "replay request exceeds cap of 1000 ids (got N)";
        let per = &v["paths"]["/contracts/{id}/quarantine/replay"]["post"];
        let per_schema = &per["responses"]["200"]["content"]["application/json"]["schema"];
        for field in [
            "total",
            "replayed",
            "still_quarantined",
            "already_replayed",
            "not_found",
            "wrong_contract",
            "purged",
            "redacted",
            "target_version",
            "target_version_source",
            "target_is_draft",
            "results",
        ] {
            assert!(
                per_schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == field),
                "per-contract replay missing required {field}"
            );
        }
        assert_eq!(per_schema["properties"]["redacted"]["type"], "integer");
        let outcome = &per_schema["properties"]["results"]["items"]["properties"]["outcome"];
        assert!(outcome["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "redacted"));
        assert!(per["responses"]["400"]["description"]
            .as_str()
            .unwrap()
            .contains(cap));
        let org = &v["paths"]["/quarantine/replay"]["post"];
        assert!(
            org["responses"]["200"]["content"]["application/json"]["schema"]["properties"]
                ["outcomes"]
                .is_object()
        );
        assert!(org["responses"]["400"]["description"]
            .as_str()
            .unwrap()
            .contains(cap));
        assert!(
            org["responses"]["400"]["content"]["application/json"]["schema"]["properties"]["error"]
                .is_object()
        );

        let item = &v["paths"]["/contracts"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["items"];
        assert_eq!(
            item["properties"]["multi_stable_resolution"]["type"],
            "string"
        );
        assert!(item["properties"]["multi_stable_resolution"]["type"]
            .as_array()
            .is_none());
        assert_eq!(
            item["properties"]["multi_stable_resolution"]["enum"],
            json!(["strict", "fallback"])
        );
        assert_eq!(
            item["properties"]["latest_stable_version"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(item["properties"]["version_count"]["type"], "integer");
        for field in [
            "multi_stable_resolution",
            "latest_stable_version",
            "version_count",
        ] {
            assert!(
                item["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == field),
                "contract list missing required {field}"
            );
        }

        for path in ["/me", "/v1/me"] {
            let me = &v["paths"][path]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"];
            for field in [
                "org_id",
                "org_name",
                "plan",
                "plan_display",
                "paid_features",
                "self_hosted",
                "auth",
                "api_key_id",
                "monthly_event_limit",
                "rate_limit_rps",
                "rate_limit_burst",
            ] {
                assert!(
                    me["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == field),
                    "{path} missing {field}"
                );
            }
            assert_eq!(
                me["properties"]["org_name"]["type"],
                json!(["string", "null"])
            );
            assert_eq!(me["properties"]["plan"]["type"], "string");
            assert_eq!(me["properties"]["paid_features"]["type"], "boolean");
            assert_eq!(me["properties"]["self_hosted"]["type"], "boolean");
            assert_eq!(
                me["properties"]["api_key_id"]["type"],
                json!(["string", "null"])
            );
            assert_eq!(
                me["properties"]["monthly_event_limit"]["type"],
                json!(["integer", "null"])
            );
            assert_eq!(
                me["properties"]["auth"]["enum"],
                json!(["api_key", "session"])
            );
        }

        let egress = &v["paths"]["/egress/{contract_id}"]["post"]["responses"];
        let eg207 = &egress["207"]["content"]["application/json"]["schema"];
        let eg200 = &egress["200"]["content"]["application/json"]["schema"];
        assert_eq!(eg200, eg207);
        for field in [
            "total",
            "passed",
            "failed",
            "dry_run",
            "disposition",
            "egress_leakage_mode",
            "resolved_version",
            "payload",
            "outcomes",
        ] {
            assert!(
                eg207["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == field),
                "egress 207 missing {field}"
            );
        }
        assert_eq!(
            eg207["properties"]["outcomes"]["items"]["properties"]["action"]["enum"],
            json!(["included", "blocked", "rejected", "tagged"])
        );
    }

    /// Every documented 2xx body has a schema. 204 has no body.
    #[test]
    fn openapi_success_responses_have_nonempty_schemas() {
        let v: Value = serde_json::from_str(&build_openapi()).unwrap();
        let mut empty = Vec::new();
        for (path, item) in v["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("path item") {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let Some(responses) = op.get("responses").and_then(Value::as_object) else {
                    continue;
                };
                for (status, resp) in responses {
                    let Ok(code) = status.parse::<u16>() else {
                        continue;
                    };
                    if !(200..300).contains(&code) || code == 204 {
                        continue;
                    }
                    if !response_schema_nonempty(resp) {
                        empty.push(format!("{method} {path} {status}"));
                    }
                }
            }
        }
        assert!(
            empty.is_empty(),
            "2xx responses with an empty schema:\n{}",
            empty.join("\n")
        );
    }

    fn response_schema_nonempty(resp: &Value) -> bool {
        let Some(content) = resp.get("content").and_then(Value::as_object) else {
            return false;
        };
        !content.is_empty()
            && content
                .values()
                .all(|media| schema_has_shape(media.get("schema").unwrap_or(&Value::Null)))
    }

    fn schema_has_shape(schema: &Value) -> bool {
        let Some(obj) = schema.as_object() else {
            return false;
        };
        [
            "type",
            "properties",
            "oneOf",
            "anyOf",
            "allOf",
            "$ref",
            "enum",
            "const",
            "items",
        ]
        .iter()
        .any(|key| obj.contains_key(*key))
    }

    fn router_operations(
        src: &str,
    ) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
        use std::collections::{BTreeMap, BTreeSet};
        let start = src.find("fn build_router").expect("build_router");
        let src = &src[start..];
        let body_start = src.find('{').expect("build_router body");
        let mut depth = 0usize;
        let mut end = body_start;
        for (i, b) in src.as_bytes().iter().enumerate().skip(body_start) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &src[body_start..=end];
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut i = 0usize;
        while let Some(rel) = body[i..].find(".route(") {
            let args_start = i + rel + ".route(".len();
            let mut paren = 1i32;
            let mut j = args_start;
            let bytes = body.as_bytes();
            while j < bytes.len() && paren > 0 {
                match bytes[j] {
                    b'(' => paren += 1,
                    b')' => paren -= 1,
                    _ => {}
                }
                if paren > 0 {
                    j += 1;
                }
            }
            let args = &body[args_start..j];
            let path = first_string(args).unwrap_or_else(|| panic!("route without path: {args}"));
            let methods = http_methods(args);
            assert!(!methods.is_empty(), "no methods on {path}");
            let norm = normalize_openapi_path(&path);
            let prev = out.insert(norm.clone(), methods.clone());
            assert!(prev.is_none(), "two router paths normalize to {norm}");
            i = j + 1;
        }
        assert!(out.len() > 40, "parser missed routes: {}", out.len());
        out
    }

    fn spec_operations(
        v: &Value,
    ) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
        use std::collections::{BTreeMap, BTreeSet};
        const METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];
        let mut out = BTreeMap::new();
        for (path, item) in v["paths"].as_object().expect("paths") {
            let methods = item
                .as_object()
                .expect("path item")
                .keys()
                .filter(|k| METHODS.contains(&k.as_str()))
                .cloned()
                .collect::<BTreeSet<_>>();
            out.insert(normalize_openapi_path(path), methods);
        }
        out
    }

    fn normalize_openapi_path(path: &str) -> String {
        let mut out = String::new();
        let mut chars = path.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                out.push_str("{}");
                for c2 in chars.by_ref() {
                    if c2 == '}' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn first_string(args: &str) -> Option<String> {
        let bytes = args.as_bytes();
        let start = bytes.iter().position(|b| *b == b'"')? + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'"' {
            end += 1;
        }
        Some(args[start..end].to_string())
    }

    fn http_methods(args: &str) -> std::collections::BTreeSet<String> {
        use std::collections::BTreeSet;
        const METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];
        let bytes = args.as_bytes();
        let mut found = BTreeSet::new();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                i = i.saturating_add(1);
                continue;
            }
            for method in METHODS {
                let mb = method.as_bytes();
                if !bytes[i..].starts_with(mb) {
                    continue;
                }
                let before_ok = i == 0 || {
                    let p = bytes[i - 1];
                    !p.is_ascii_alphanumeric() && p != b'_'
                };
                let mut j = i + mb.len();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if before_ok && j < bytes.len() && bytes[j] == b'(' {
                    found.insert((*method).to_string());
                }
            }
            i += 1;
        }
        found
    }
}
