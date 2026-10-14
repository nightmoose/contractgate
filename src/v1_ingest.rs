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
}

/// Top-level response body.
#[derive(Debug, Serialize)]
pub struct V1IngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub atomic: bool,
    pub resolved_version: String,
    /// One of: `"query_param"` | `"default_stable"` | `"pinned_deprecated"`.
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
    serde_json::to_string_pretty(&json!({
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
                "schema": { "type": "string", "format": "uuid" }
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
              "422": { "description": "All events failed, atomic rejection, idempotency conflict, or deprecated version pin" },
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
              "422": { "description": "Every record failed, or any failed with disposition=fail. The body is still a full egress response with per-record violations." },
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
        "/me": { "get": { "operationId": "me", "summary": "Caller org id, org name and plan (no emails). Also at /v1/me.", "responses": { "200": { "description": "org_id, org_name, plan, plan_display, auth (api_key|session), api_key_id, monthly_event_limit, rate_limit_rps, rate_limit_burst" }, "401": { "description": "Missing or invalid credentials" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/v1/me": { "get": { "operationId": "v1_me", "summary": "Alias of /me", "responses": { "200": { "description": "Same as /me" }, "401": { "description": "Missing or invalid credentials" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/collaborators": { "get": { "operationId": "list_collaborators", "summary": "List collaborators (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "responses": { "200": { "description": "Collaborator grants" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "grant_collaborator", "summary": "Grant a role to another org (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["org_id", "role"], "properties": { "org_id": { "type": "string", "format": "uuid" }, "role": { "type": "string", "enum": ["viewer", "editor", "reviewer"] } } } } } }, "responses": { "201": { "description": "Grant created or updated" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/collaborators/{org_id}": { "patch": { "operationId": "patch_collaborator", "summary": "Change a role (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "org_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["role"], "properties": { "role": { "type": "string", "enum": ["viewer", "editor", "reviewer"] } } } } } }, "responses": { "200": { "description": "Updated grant" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "delete": { "operationId": "revoke_collaborator", "summary": "Revoke a grant (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "org_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "204": { "description": "Revoked" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments": { "get": { "operationId": "list_comments", "summary": "List comments (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "responses": { "200": { "description": "Comments, oldest first" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "add_comment", "summary": "Add a comment (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["author", "body"], "properties": { "field": { "type": ["string", "null"] }, "author": { "type": "string" }, "body": { "type": "string" } } } } } }, "responses": { "201": { "description": "Created" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments/{id}": { "delete": { "operationId": "delete_comment", "summary": "Delete a comment (author org or owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "204": { "description": "Deleted" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/comments/{id}/resolve": { "post": { "operationId": "resolve_comment", "summary": "Resolve a comment (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "200": { "description": "Resolved" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals": { "get": { "operationId": "list_proposals", "summary": "List proposals (viewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "responses": { "200": { "description": "Newest first" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] }, "post": { "operationId": "create_proposal", "summary": "Open a proposal (editor+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["proposed_yaml"], "properties": { "proposed_yaml": { "type": "string" } } } } } }, "responses": { "201": { "description": "Opened" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals/{id}/decide": { "post": { "operationId": "decide_proposal", "summary": "Approve or reject (reviewer+)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["decision"], "properties": { "decision": { "type": "string", "enum": ["approved", "rejected"] } } } } } }, "responses": { "200": { "description": "Decided" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } },
        "/contracts/{name}/proposals/{id}/apply": { "post": { "operationId": "apply_proposal", "summary": "Mark an approved proposal applied (owner)", "parameters": [{ "name": "name", "in": "path", "required": true, "description": "Contract name", "schema": { "type": "string" } }, { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }], "responses": { "200": { "description": "Applied" }, "401": { "description": "Missing or invalid credentials" }, "403": { "description": "Caller lacks the required role; the message names it, e.g. requires owner role on contract 'x'" }, "404": { "description": "Contract not found" } }, "security": [{ "ApiKeyAuth": [] }] } }
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
    }))
    .unwrap_or_else(|_| "{}".to_string())
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
    version_param: Option<String>,
) -> AppResult<(String, &'static str)> {
    if let Some(v) = version_param {
        return Ok((v, "query_param"));
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
    let contract_id = Uuid::parse_str(&contract_id_str)
        .map_err(|_| AppError::BadRequest(format!("invalid contract_id: {contract_id_str}")))?;

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
        crate::metering::enforce_plan_limit(&state.db, org_id).await?;
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
        resolve_version(&state, contract_id, query.version).await?;

    let version_row =
        storage::get_version(&state.db, contract_id, &resolved_version, org_id).await?;

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
    if !query.dry_run && !atomic_rejected {
        for &idx in &failed_indices {
            quarantine_ids[idx] = Some(Uuid::new_v4());
        }
    }

    // --- 12. Persist (fire-and-forget) ------------------------------------
    if !query.dry_run && !atomic_rejected {
        let audit_rows: Vec<storage::AuditEntryInsert> = validation_results
            .iter()
            .enumerate()
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
                if let Err(e) = storage::forward_events_batch(&pool, &forward_rows).await {
                    tracing::warn!("v1 ingest: forward write failed: {e:?}");
                }
            });
        }
    } else if !query.dry_run && atomic_rejected {
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

    let response_body = V1IngestResponse {
        total,
        passed: passed_count,
        failed: failed_count,
        dry_run: query.dry_run,
        atomic: query.atomic,
        resolved_version: resolved_version.clone(),
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
    _org_id: Option<Uuid>,
    pinned_version: &str,
    events: Vec<Value>,
    source_ip: Option<String>,
    dry_run: bool,
    store_payloads: bool,
) -> AppResult<Response> {
    let total = events.len();
    let compiled = state.get_compiled(contract_id, pinned_version).await?;

    let transformed: Vec<TransformedPayload> = events
        .iter()
        .map(|e| apply_transforms(&compiled, e.clone()))
        .collect();

    let per_event: Vec<V1IngestEventResult> = (0..total)
        .map(|idx| V1IngestEventResult {
            index: idx,
            passed: false,
            violations: vec![],
            validation_us: 0,
            forwarded: false,
            contract_version: pinned_version.to_string(),
            quarantine_id: None,
            transformed_event: transformed[idx].as_value().clone(),
        })
        .collect();

    if !dry_run {
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

    let body = json!({
        "error": format!(
            "Version {} on contract {} is deprecated; batch quarantined.",
            pinned_version, contract_id,
        ),
        "total": total,
        "passed": 0,
        "failed": total,
        "dry_run": dry_run,
        "version_pin_source": "pinned_deprecated",
        "results": per_event,
    });

    Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
            "/me",
            "/v1/me",
        ] {
            assert!(v["paths"][path].is_object(), "openapi missing path {path}");
        }
    }
}
