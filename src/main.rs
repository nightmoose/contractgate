//! ContractGate — Real-time semantic contract enforcement gateway.
//! Patent Pending.
//!
//! Starts an Axum HTTP server with routes for:
//!   - Contract identity CRUD (`/contracts`)
//!   - Contract version CRUD (`/contracts/:id/versions/...`)
//!   - Version state transitions (promote / deprecate)
//!   - Name-change history
//!   - Ingestion API (`POST /ingest/{contract_id}[@version]`)
//!   - Audit log queries
//!   - Health check
//!
//! Version resolution + fallback semantics live in `ingest.rs`; this module
//! is just wiring.

use axum::extract::Request;
use axum::{
    extract::{FromRequest, Path, Query, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

// Pure validation logic lives in the library crate (src/lib.rs) so it can be
// shared with other binaries (e.g. src/bin/demo.rs).  We re-export it here so
// existing submodules that refer to `crate::contract` / `crate::validation`
// continue to resolve unchanged.
pub use contractgate::{contract, transform, validation};

mod api_key_auth;
mod collaboration;
mod conformance;
mod egress;
mod error;
mod fork_filter;
mod idempotency;
mod infer;
mod infer_avro;
mod infer_csv;
mod infer_diff;
mod infer_openapi;
mod infer_proto;
mod ingest;
mod kafka_consumer;
mod kafka_ingress;
mod kinesis_consumer;
mod kinesis_ingress;
pub mod observability;
mod odcs;
mod public_catalog;
mod publication;
mod rate_limit;
mod replay;
mod scorecard;
mod storage;
mod stream_demo;
#[cfg(test)]
mod tests;
mod v1_ingest;

use contract::{
    Contract, ContractIdentity, ContractResponse, ContractSummary, ContractVersion,
    CreateContractRequest, CreateVersionRequest, DeployContractRequest, DeployContractResponse,
    NameHistoryEntry, PatchContractRequest, PatchVersionRequest, VersionResponse, VersionSummary,
};
use error::{AppError, AppResult};
use validation::CompiledContract;

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Shared application state passed to every Axum handler.
///
/// The compiled-contract cache is **keyed by (contract_id, version)** — every
/// live version has at most one compiled form in memory.  The cache is
/// warmed on boot with every stable + deprecated version; drafts load lazily
/// the first time they're pinned.
pub struct AppState {
    pub db: PgPool,
    /// (contract_id, version) → compiled contract.
    contract_cache: RwLock<HashMap<(Uuid, String), Arc<CompiledContract>>>,
    /// Legacy single env-var key (empty = disabled).  Kept for zero-downtime
    /// migration: if set, it still grants access alongside DB-issued keys.
    pub api_key: String,
    /// DB-backed API key cache with 60-second TTL.
    pub key_cache: Arc<api_key_auth::ApiKeyCache>,
    /// Per-API-key token-bucket rate limiter (RFC-021).
    pub rate_limiter: Arc<rate_limit::RateLimitState>,
    /// In-process stream demo state (no Kafka, no DB writes).
    pub stream_demo: std::sync::Arc<stream_demo::StreamDemoState>,
    /// RFC-025: platform-side Kafka consumer pool (one task per enabled contract).
    pub kafka_consumers: kafka_consumer::ConsumerPool,
    /// RFC-026: platform-side Kinesis consumer pool (one task per enabled contract).
    pub kinesis_consumers: kinesis_consumer::ConsumerPool,
}

impl AppState {
    pub fn new(db: PgPool, api_key: String) -> Self {
        AppState {
            db,
            contract_cache: RwLock::new(HashMap::new()),
            api_key,
            key_cache: Arc::new(api_key_auth::ApiKeyCache::default()),
            rate_limiter: Arc::new(rate_limit::RateLimitState::default()),
            stream_demo: std::sync::Arc::new(stream_demo::StreamDemoState::new()),
            kafka_consumers: kafka_consumer::ConsumerPool::new(),
            kinesis_consumers: kinesis_consumer::ConsumerPool::new(),
        }
    }

    // ---- contract_cache lock helpers -------------------------------------
    //
    // The cache is wrapped in `RwLock`, which can be poisoned if a holder
    // panics.  We've never seen this in practice (the hot paths are tiny
    // map ops with no allocation between the lock guard and the return), but
    // calling `.unwrap()` everywhere obscures that invariant.  These helpers
    // centralize the assumption — and the panic message identifies the cache
    // by name if the contract ever does break — so future call sites stay
    // one-liners and the rationale lives in exactly one place.

    fn cache_read(&self) -> RwLockReadGuard<'_, HashMap<(Uuid, String), Arc<CompiledContract>>> {
        self.contract_cache
            .read()
            .expect("contract cache RwLock poisoned (a prior holder panicked)")
    }

    fn cache_write(&self) -> RwLockWriteGuard<'_, HashMap<(Uuid, String), Arc<CompiledContract>>> {
        self.contract_cache
            .write()
            .expect("contract cache RwLock poisoned (a prior holder panicked)")
    }

    /// Load every stable + deprecated version into the cache.  Drafts are
    /// loaded lazily — they're rare, mutable, and not usually pinned.
    ///
    /// Also loads every contract's `pii_salt` in a single round-trip so
    /// each compiled contract is seeded with the correct HMAC key for
    /// `kind: hash` and `format_preserving` transforms (RFC-004).
    pub async fn warm_cache(&self) -> AppResult<()> {
        let versions = storage::load_all_non_draft_versions(&self.db).await?;
        let salts = storage::load_all_pii_salts(&self.db).await?;
        let mut cache = self.cache_write();
        for v in versions {
            // Missing salt would only happen if a contract row vanished
            // between the two queries.  Fall back to an empty salt and
            // log — a follow-up cache miss will re-fetch cleanly.
            let salt = salts.get(&v.contract_id).cloned().unwrap_or_default();
            match compile_version(&v, salt) {
                Ok(compiled) => {
                    cache.insert((v.contract_id, v.version.clone()), Arc::new(compiled));
                }
                Err(e) => {
                    // A single malformed contract must not prevent the
                    // server from booting — log it and move on.
                    tracing::warn!(
                        contract_id = %v.contract_id,
                        version = %v.version,
                        "skipping cache warmup for bad contract: {}",
                        e
                    );
                }
            }
        }
        Ok(())
    }

    /// Look up the compiled contract for (contract_id, version), loading from
    /// DB + compiling on a cache miss.  Returns a clone of the shared `Arc`
    /// so validation can run without holding the lock.
    pub async fn get_compiled(
        &self,
        contract_id: Uuid,
        version: &str,
    ) -> AppResult<Arc<CompiledContract>> {
        // Fast path: read lock
        {
            let cache = self.cache_read();
            if let Some(cc) = cache.get(&(contract_id, version.to_string())) {
                return Ok(Arc::clone(cc));
            }
        }

        // Slow path: fetch + compile + insert.  We load the identity so
        // we can seed the compiled contract with the correct `pii_salt`
        // (RFC-004).  Cost is one extra round-trip on cache miss —
        // acceptable because misses are rare (boot + draft pins).
        let row = storage::get_version(&self.db, contract_id, version).await?;
        let identity = storage::get_contract_identity(&self.db, contract_id).await?;
        let compiled = compile_version(&row, identity.pii_salt).map_err(|e| {
            AppError::InvalidContractYaml(format!(
                "could not compile contract {}@{}: {}",
                contract_id, version, e
            ))
        })?;
        let arc = Arc::new(compiled);

        {
            let mut cache = self.cache_write();
            cache.insert((contract_id, version.to_string()), Arc::clone(&arc));
        }

        Ok(arc)
    }

    /// Drop a single (contract_id, version) entry from the cache.  Call on
    /// draft edits — the YAML has changed, so the compiled form is stale.
    pub fn invalidate_version(&self, contract_id: Uuid, version: &str) {
        let mut cache = self.cache_write();
        cache.remove(&(contract_id, version.to_string()));
    }

    /// Drop every cached version for a contract.  Call on delete.
    pub fn invalidate_contract_all(&self, contract_id: Uuid) {
        let mut cache = self.cache_write();
        cache.retain(|(cid, _), _| *cid != contract_id);
    }
}

/// Parse + compile a `ContractVersion` row into a `CompiledContract`,
/// seeded with the contract's per-contract `pii_salt` for RFC-004
/// transforms (`kind: hash`, `mask:format_preserving`).
fn compile_version(v: &ContractVersion, salt: Vec<u8>) -> Result<CompiledContract, String> {
    let parsed: Contract = serde_yaml::from_str(&v.yaml_content).map_err(|e| e.to_string())?;
    CompiledContract::compile_with_salt(parsed, salt).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Response shaping helpers
// ---------------------------------------------------------------------------

/// Merge an identity row with aggregated version info into a response.
async fn identity_to_response(db: &PgPool, id: ContractIdentity) -> AppResult<ContractResponse> {
    let summaries = storage::list_versions(db, id.id).await?;
    let version_count = summaries.len() as i64;
    let latest_stable_version = storage::get_latest_stable_version(db, id.id)
        .await?
        .map(|v| v.version);
    Ok(ContractResponse {
        id: id.id,
        name: id.name,
        description: id.description,
        multi_stable_resolution: id.multi_stable_resolution,
        created_at: id.created_at,
        updated_at: id.updated_at,
        version_count,
        latest_stable_version,
    })
}

// ---------------------------------------------------------------------------
// Helper: extract org_id from request extensions.
//
// Priority:
//   1. DB-backed API key (ValidatedKey in extensions) — most secure; org_id
//      is authoritative from the database row.
//   2. `x-org-id` request header — trusted fallback when using the legacy
//      env-var key or in dev mode (no key).  The dashboard passes this header
//      so the user's personal org is used even before they've created a
//      DB-backed key.  Not trusted when a DB-backed key is already present.
// ---------------------------------------------------------------------------

fn org_id_from_req(req: &axum::extract::Request) -> Option<uuid::Uuid> {
    // 1. DB-backed key wins unconditionally.
    if let Some(k) = req.extensions().get::<api_key_auth::ValidatedKey>() {
        return Some(k.org_id);
    }
    // 2. Fallback: client-supplied header (legacy/dev mode only).
    req.headers()
        .get("x-org-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
}

// ---------------------------------------------------------------------------
// Contract identity handlers
// ---------------------------------------------------------------------------

async fn create_contract_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ContractResponse>)> {
    let org_id = org_id_from_req(&req);
    // Extract JSON body after reading extensions
    let Json(body_req): Json<CreateContractRequest> =
        axum::Json::from_request(req, &state)
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let (identity, _first_version) = storage::create_contract(
        &state.db,
        &body_req.name,
        body_req.description.as_deref(),
        &body_req.yaml_content,
        body_req.multi_stable_resolution.unwrap_or_default(),
        org_id,
    )
    .await?;

    let resp = identity_to_response(&state.db, identity).await?;
    Ok((StatusCode::CREATED, Json(resp)))
}

async fn get_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ContractResponse>> {
    let identity = storage::get_contract_identity(&state.db, id).await?;
    Ok(Json(identity_to_response(&state.db, identity).await?))
}

async fn list_contracts_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<Json<Vec<ContractSummary>>> {
    let org_id = org_id_from_req(&req);
    let contracts = storage::list_contracts(&state.db, org_id).await?;
    Ok(Json(contracts))
}

async fn patch_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchContractRequest>,
) -> AppResult<Json<ContractResponse>> {
    let identity = storage::patch_contract_identity(
        &state.db,
        id,
        req.name.as_deref(),
        req.description.as_deref(),
        req.multi_stable_resolution,
    )
    .await?;
    // Identity-only patch doesn't touch yaml; no cache eviction needed.
    Ok(Json(identity_to_response(&state.db, identity).await?))
}

async fn delete_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    storage::delete_contract(&state.db, id).await?;
    state.invalidate_contract_all(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Version handlers
// ---------------------------------------------------------------------------

async fn create_version_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<CreateVersionRequest>,
) -> AppResult<(StatusCode, Json<VersionResponse>)> {
    let v =
        storage::create_version(&state.db, contract_id, &req.version, &req.yaml_content).await?;
    // Drafts are cached lazily on first pin — no eager insert.
    Ok((StatusCode::CREATED, Json(VersionResponse::from(&v))))
}

async fn list_versions_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<Vec<VersionSummary>>> {
    let versions = storage::list_versions(&state.db, contract_id).await?;
    Ok(Json(versions))
}

async fn get_version_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    let v = storage::get_version(&state.db, contract_id, &version).await?;
    Ok(Json(VersionResponse::from(&v)))
}

async fn get_latest_stable_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<VersionResponse>> {
    // Ensure the contract exists (clean 404).
    let _ = storage::get_contract_identity(&state.db, contract_id).await?;
    let v = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok(Json(VersionResponse::from(&v)))
}

async fn patch_version_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
    Json(req): Json<PatchVersionRequest>,
) -> AppResult<Json<VersionResponse>> {
    let v =
        storage::patch_version_yaml(&state.db, contract_id, &version, &req.yaml_content).await?;
    // Evict: a draft edit changes its compiled form.
    state.invalidate_version(contract_id, &version);
    Ok(Json(VersionResponse::from(&v)))
}

async fn promote_version_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    let v = storage::promote_version(&state.db, contract_id, &version).await?;
    // Version is now stable and frozen.  Pre-warm the cache so the first
    // ingest request doesn't take the slow path.
    let _ = state.get_compiled(contract_id, &version).await;
    Ok(Json(VersionResponse::from(&v)))
}

async fn deprecate_version_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    let v = storage::deprecate_version(&state.db, contract_id, &version).await?;
    // Cached compiled form is still correct (YAML didn't change); the
    // deprecated-pin short-circuit in ingest.rs looks at the DB row's state,
    // not the cache.  No invalidation needed.
    Ok(Json(VersionResponse::from(&v)))
}

async fn delete_version_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<StatusCode> {
    storage::delete_version(&state.db, contract_id, &version).await?;
    state.invalidate_version(contract_id, &version);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_name_history_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<Vec<NameHistoryEntry>>> {
    // 404 if the contract is gone.
    let _ = storage::get_contract_identity(&state.db, contract_id).await?;
    let rows = storage::list_name_history(&state.db, contract_id).await?;
    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// Deploy handler (RFC-028)
// ---------------------------------------------------------------------------

/// `POST /contracts/deploy` — atomically deploy a contract version as stable.
///
/// Admin-only: requires a service-role or admin API key (validated by the
/// standard auth middleware; no additional role check is needed here because
/// only service-role keys bypass org-scoped RLS).
///
/// Steps (delegated to `storage::deploy_contract_version`):
///   1. Find-or-create the contract identity by name.
///   2. Reject if pending quarantine events exist for this contract.
///   3. Insert the version as `stable` with parsed_json/source/deployed_by/deployed_at.
///   4. Deprecate all previously-stable versions.
async fn deploy_contract_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<DeployContractResponse>)> {
    let org_id = org_id_from_req(&req);
    let Json(body): Json<DeployContractRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let (version, deprecated_count) = storage::deploy_contract_version(
        &state.db,
        &body.name,
        &body.yaml_content,
        body.source.as_deref(),
        body.deployed_by.as_deref(),
        org_id,
    )
    .await?;

    // Warm the cache for the new stable version so the first ingest is fast.
    let _ = state
        .get_compiled(version.contract_id, &version.version)
        .await;

    let deployed_at = version.promoted_at.unwrap_or_else(chrono::Utc::now);
    let resp = DeployContractResponse {
        contract_id: version.contract_id,
        version_id: version.id,
        name: body.name,
        version: version.version,
        source: body.source,
        deployed_by: body.deployed_by,
        deployed_at,
        deprecated_count,
    };

    Ok((StatusCode::CREATED, Json(resp)))
}

// ---------------------------------------------------------------------------
// ODCS import / export / approve-import handlers
// ---------------------------------------------------------------------------

/// `POST /contracts/import` — accept an ODCS v3.1.0 document, create a new
/// contract identity + draft version.
///
/// Mode A (lossless): document carries `x-contractgate-*` extensions.
/// Mode B (stripped): best-effort reconstruction; `requires_review = true`.
///
/// Flow:
///   1. Parse ODCS → `ImportResult { version, yaml_content, import_source }`
///   2. Create identity + a throwaway v1.0.0 native draft via `create_contract`
///      (the only transactional identity-create helper we have).
///   3. Delete the throwaway draft.
///   4. Create the real versioned draft with correct import provenance.
async fn import_odcs_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<VersionResponse>)> {
    let org_id = org_id_from_req(&req);
    let Json(body): Json<odcs::ImportOdcsRequest> = axum::Json::from_request(req, &state)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let result = odcs::import_odcs(&body.odcs_yaml).map_err(AppError::BadRequest)?;

    // Name override from request body wins over the ODCS-parsed name.
    let name_in_yaml: Contract = serde_yaml::from_str(&result.yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let name = body
        .name_override
        .as_deref()
        .unwrap_or(&name_in_yaml.name)
        .to_string();

    // Step 2: create identity + throwaway v1.0.0 draft.
    let (identity, _) = storage::create_contract(
        &state.db,
        &name,
        name_in_yaml.description.as_deref(),
        &result.yaml_content,
        Default::default(),
        org_id,
    )
    .await?;

    // Step 3: remove the throwaway native draft.
    storage::delete_version(&state.db, identity.id, "1.0.0").await?;

    // Step 4: create the real versioned draft with correct import provenance.
    let cv = storage::create_version_from_import(
        &state.db,
        identity.id,
        &result.version,
        &result.yaml_content,
        result.import_source,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(VersionResponse::from(&cv))))
}

/// `GET /contracts/:id/versions/:version/export` — return the contract version
/// serialized as ODCS v3.1.0 YAML.
async fn export_odcs_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<axum::response::Response> {
    let identity = storage::get_contract_identity(&state.db, contract_id).await?;
    let cv = storage::get_version(&state.db, contract_id, &version).await?;
    let contract: Contract = serde_yaml::from_str(&cv.yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
        identity: &identity,
        version: &cv,
        contract: &contract,
    })
    .map_err(AppError::Internal)?;

    Ok((
        StatusCode::OK,
        [("content-type", "application/yaml")],
        odcs_yaml,
    )
        .into_response())
}

/// `POST /contracts/:id/versions/:version/approve-import` — clear the
/// `requires_review` flag set on a stripped ODCS import (D-002).  Only legal
/// on draft versions.
async fn approve_import_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    let cv = storage::clear_requires_review(&state.db, contract_id, &version).await?;
    Ok(Json(VersionResponse::from(&cv)))
}

// ---------------------------------------------------------------------------
// Conformance report handler
// ---------------------------------------------------------------------------

/// `GET /contracts/:id/versions/:version/odcs-conformance`
///
/// Returns a `ConformanceReport` with four ODCS v3.1.0 dimension scores.
async fn odcs_conformance_handler(
    State(state): State<Arc<AppState>>,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<conformance::ConformanceReport>> {
    let identity = storage::get_contract_identity(&state.db, contract_id).await?;
    let cv = storage::get_version(&state.db, contract_id, &version).await?;
    let contract: Contract = serde_yaml::from_str(&cv.yaml_content)
        .map_err(|e| AppError::BadRequest(format!("stored yaml_content is invalid: {e}")))?;
    let report = conformance::compute_conformance(&identity, &cv, &contract);
    Ok(Json(report))
}

// ---------------------------------------------------------------------------
// Audit log handler
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct AuditQuery {
    contract_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

async fn audit_log_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
    req: axum::extract::Request,
) -> AppResult<Json<Vec<storage::AuditEntry>>> {
    let org_id = org_id_from_req(&req);
    let entries =
        storage::recent_audit_entries(&state.db, org_id, q.contract_id, q.limit.min(500), q.offset)
            .await?;
    Ok(Json(entries))
}

async fn global_stats_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<Json<storage::IngestionStats>> {
    let org_id = org_id_from_req(&req);
    let stats = storage::ingestion_stats(&state.db, org_id, None).await?;
    Ok(Json(stats))
}

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "contractgate"
    }))
}

// ---------------------------------------------------------------------------
// Playground — validate a YAML + event without persisting anything
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct PlaygroundRequest {
    yaml_content: String,
    event: serde_json::Value,
}

/// Playground response — the unsaved-contract counterpart to
/// `BatchIngestResponse`.  Carries the validation verdict plus the
/// post-transform payload the backend *would* persist, so the dashboard
/// can render the "what we stored" diff (RFC-004) without requiring the
/// user to save the contract first.
///
/// `transformed_event` is produced by [`transform::apply_transforms`] on
/// the compiled contract regardless of whether validation passed — if the
/// contract declares no transforms it is byte-for-byte identical to the
/// request body.  The salt used here is empty (the Playground has no row
/// in `contracts`), so hash + format-preserving outputs are *illustrative*
/// only — they will not match what ingest produces under the real
/// per-contract salt.
#[derive(serde::Serialize)]
struct PlaygroundResponse {
    #[serde(flatten)]
    validation: validation::ValidationResult,
    /// Echo of the post-transform payload.  `null` is possible only if
    /// the request body was literal JSON `null`; every other shape round-
    /// trips through `apply_transforms`.
    transformed_event: serde_json::Value,
}

async fn playground_handler(
    Json(req): Json<PlaygroundRequest>,
) -> AppResult<Json<PlaygroundResponse>> {
    let contract: contract::Contract = serde_yaml::from_str(&req.yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let compiled = CompiledContract::compile(contract)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let validation = validation::validate(&compiled, &req.event);
    // RFC-004: always run transforms so the dashboard can surface the
    // diff — even failing events are informative ("look what we were
    // about to write to quarantine").
    let transformed_event = transform::apply_transforms(&compiled, req.event.clone()).into_inner();

    Ok(Json(PlaygroundResponse {
        validation,
        transformed_event,
    }))
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn require_api_key(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<axum::response::Response, error::AppError> {
    let provided = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    // Dev mode: no auth configured at all — pass through.
    if state.api_key.is_empty() {
        return Ok(next.run(request).await);
    }

    // Legacy env-var key: still accepted for zero-downtime migration.
    // Remove this branch once all connectors are issuing DB-backed keys.
    if !provided.is_empty() && provided == state.api_key {
        return Ok(next.run(request).await);
    }

    // DB-backed key: validate via cache (60-second TTL).
    if !provided.is_empty() {
        match state.key_cache.validate(&provided, &state.db).await {
            Ok(validated) => {
                // Inject the validated key into request extensions so
                // downstream handlers can scope queries to the correct org.
                request.extensions_mut().insert(validated);
                return Ok(next.run(request).await);
            }
            Err(()) => {
                // Evict so the next retry re-checks the DB immediately.
                state.key_cache.evict(&provided);
            }
        }
    }

    tracing::warn!("Rejected request: missing or invalid x-api-key");
    Err(error::AppError::Unauthorized)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Public routes — no auth required
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/openapi.json", get(v1_ingest::openapi_handler))
        .route("/playground/validate", post(playground_handler))
        // Prometheus metrics scrape endpoint (RFC-016).
        // Open by default; Bearer-auth gated when METRICS_AUTH_TOKEN is set.
        // Mounted as public so Prometheus can scrape without an x-api-key.
        .route("/metrics", get(observability::metrics_handler))
        // Stream demo — public so the browser's EventSource can connect
        // without auth headers (no sensitive data, local demo only).
        .route("/demo/start", post(stream_demo::start_handler))
        .route("/demo/stop", post(stream_demo::stop_handler))
        .route("/demo/stream", get(stream_demo::stream_handler))
        .route("/demo/events", get(stream_demo::events_handler))
        .route("/demo/contract", get(stream_demo::contract_handler))
        // RFC-032: public-fetch for published contracts (visibility checked inside handler)
        .route(
            "/published/{publication_ref}",
            get(publication::fetch_published_handler),
        )
        // Curated open-data public contracts (no auth — readable by anyone)
        .route(
            "/public-contracts",
            get(public_catalog::list_public_contracts_handler),
        )
        .route(
            "/public-contracts/{id}",
            get(public_catalog::get_public_contract_handler),
        )
        // User-published contracts catalog (no auth — lists public visibility publications)
        .route("/catalog", get(publication::catalog_handler));

    // Protected routes — require x-api-key header
    let protected = Router::new()
        // Deploy — RFC-028: atomically push a version straight to stable
        .route("/contracts/deploy", post(deploy_contract_handler))
        // ODCS import / export / approve-import
        .route("/contracts/import", post(import_odcs_handler))
        .route(
            "/contracts/{id}/versions/{version}/export",
            get(export_odcs_handler),
        )
        .route(
            "/contracts/{id}/versions/{version}/approve-import",
            post(approve_import_handler),
        )
        .route(
            "/contracts/{id}/versions/{version}/odcs-conformance",
            get(odcs_conformance_handler),
        )
        // Contract inference — JSON samples
        .route("/contracts/infer", post(infer::infer_handler))
        // Contract inference — format-specific routes (RFC-006, RFC-035)
        .route(
            "/contracts/infer/avro",
            post(infer_avro::infer_avro_handler),
        )
        .route(
            "/contracts/infer/proto",
            post(infer_proto::infer_proto_handler),
        )
        .route(
            "/contracts/infer/openapi",
            post(infer_openapi::infer_openapi_handler),
        )
        // CSV inference (RFC-035)
        .route("/contracts/infer/csv", post(infer_csv::infer_csv_handler))
        // Evolution diff summarizer (RFC-006)
        .route("/contracts/diff", post(infer_diff::diff_handler))
        // Contract identity CRUD
        .route(
            "/contracts",
            get(list_contracts_handler).post(create_contract_handler),
        )
        .route(
            "/contracts/{id}",
            get(get_contract_handler)
                .patch(patch_contract_handler)
                .delete(delete_contract_handler),
        )
        .route(
            "/contracts/{id}/name-history",
            get(list_name_history_handler),
        )
        // Versions
        .route(
            "/contracts/{id}/versions",
            get(list_versions_handler).post(create_version_handler),
        )
        .route(
            "/contracts/{id}/versions/latest-stable",
            get(get_latest_stable_handler),
        )
        .route(
            "/contracts/{id}/versions/{version}",
            get(get_version_handler)
                .patch(patch_version_handler)
                .delete(delete_version_handler),
        )
        .route(
            "/contracts/{id}/versions/{version}/promote",
            post(promote_version_handler),
        )
        .route(
            "/contracts/{id}/versions/{version}/deprecate",
            post(deprecate_version_handler),
        )
        // Ingestion — the path is String so we can accept `@version` suffix.
        .route("/ingest/{raw_id}", post(ingest::ingest_handler))
        .route(
            "/ingest/{contract_id}/stats",
            get(ingest::ingest_stats_handler),
        )
        // Egress validation (RFC-029) — same @version suffix convention.
        .route("/egress/{raw_id}", post(egress::egress_handler))
        // Provider scorecard (RFC-031) — keyed by provider source name.
        .route("/scorecard/{source}", get(scorecard::scorecard_handler))
        .route("/scorecard/{source}/drift", get(scorecard::drift_handler))
        .route("/scorecard/{source}/export", get(scorecard::export_handler))
        // Replay Quarantine (RFC-003)
        .route(
            "/contracts/{id}/quarantine/replay",
            post(replay::replay_handler),
        )
        .route(
            "/contracts/{id}/quarantine/{quar_id}/replay-history",
            get(replay::replay_history_handler),
        )
        // Kafka Ingress (RFC-025)
        .route(
            "/contracts/{id}/kafka-ingress",
            get(kafka_ingress::get_kafka_ingress_handler),
        )
        .route(
            "/contracts/{id}/kafka-ingress/enable",
            post(kafka_ingress::enable_kafka_ingress_handler),
        )
        .route(
            "/contracts/{id}/kafka-ingress/disable",
            axum::routing::delete(kafka_ingress::disable_kafka_ingress_handler),
        )
        // Kinesis Ingress (RFC-026)
        .route(
            "/contracts/{id}/kinesis-ingress",
            get(kinesis_ingress::get_kinesis_ingress_handler),
        )
        .route(
            "/contracts/{id}/kinesis-ingress/enable",
            post(kinesis_ingress::enable_kinesis_ingress_handler),
        )
        .route(
            "/contracts/{id}/kinesis-ingress/disable",
            axum::routing::delete(kinesis_ingress::disable_kinesis_ingress_handler),
        )
        .route(
            "/contracts/{id}/kinesis-ingress/rotate-credentials",
            post(kinesis_ingress::rotate_kinesis_credentials_handler),
        )
        // RFC-032: Contract Sharing & Publication
        .route(
            "/contracts/{id}/versions/{version}/publish",
            post(publication::publish_handler),
        )
        .route(
            "/contracts/publications/{publication_ref}",
            axum::routing::delete(publication::revoke_handler),
        )
        .route(
            "/contracts/import-published",
            post(publication::import_published_handler),
        )
        .route(
            "/contracts/{id}/import-status",
            get(publication::import_status_handler),
        )
        // RFC-034: Public Catalog — fork + export (auth required)
        .route(
            "/contracts/{id}/fork",
            post(public_catalog::fork_public_contract_handler),
        )
        .route(
            "/contracts/{id}/export",
            post(public_catalog::export_fork_handler),
        )
        // RFC-033: Provider-Consumer Collaboration
        // Collaborator grants — owner-only writes, viewer+ reads.
        .route(
            "/contracts/{name}/collaborators",
            get(collaboration::list_collaborators_handler)
                .post(collaboration::grant_collaborator_handler),
        )
        .route(
            "/contracts/{name}/collaborators/{org_id}",
            axum::routing::patch(collaboration::patch_collaborator_handler)
                .delete(collaboration::revoke_collaborator_handler),
        )
        // Comments — any collaborator/owner can read and write.
        .route(
            "/contracts/{name}/comments",
            get(collaboration::list_comments_handler).post(collaboration::add_comment_handler),
        )
        .route(
            "/contracts/{name}/comments/{id}/resolve",
            post(collaboration::resolve_comment_handler),
        )
        // Change proposals — editor+ creates, reviewer+ decides, owner applies.
        .route(
            "/contracts/{name}/proposals",
            get(collaboration::list_proposals_handler).post(collaboration::create_proposal_handler),
        )
        .route(
            "/contracts/{name}/proposals/{id}/decide",
            post(collaboration::decide_proposal_handler),
        )
        .route(
            "/contracts/{name}/proposals/{id}/apply",
            post(collaboration::apply_proposal_handler),
        )
        // Audit + stats
        .route("/audit", get(audit_log_handler))
        .route("/stats", get(global_stats_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    // v1 ingest: own sub-router so the 10 MB RequestBodyLimitLayer is scoped
    // to this route only and does not affect other routes.  Auth middleware
    // must be applied here too (not just on `protected`) because Axum merges
    // routers at the path level, not at the layer level.
    let v1 = Router::new()
        .route(
            "/v1/ingest/{contract_id}",
            post(v1_ingest::v1_ingest_handler),
        )
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            10 * 1024 * 1024, // 10 MB — RFC-021
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(v1)
        .layer(cors)
        .layer(middleware::from_fn(observability::track_requests))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(30)))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // ── CLI subcommand: scorecard-rollup ──────────────────────────────────────
    // Run the daily baseline rollup job and exit.
    // Usage: cargo run -- scorecard-rollup
    if std::env::args().any(|a| a == "scorecard-rollup") {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "contractgate=info".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&database_url).await?;
        scorecard::run_baseline_rollup(&pool).await?;
        tracing::info!("scorecard-rollup complete");
        return Ok(());
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "contractgate=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Install the Prometheus recorder before any metrics macros fire.
    // Must happen before the first request and before warm_cache (which
    // could theoretically emit metrics in future).
    observability::install_recorder();
    tracing::info!("Prometheus metrics recorder installed");

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = PgPool::connect(&database_url).await?;
    tracing::info!("Connected to database");

    let api_key = std::env::var("API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        tracing::warn!("API_KEY is not set — running without authentication (dev mode only)");
    } else {
        tracing::info!("API key authentication enabled");
    }

    let state = Arc::new(AppState::new(pool, api_key));

    // Warm the compiled-contract cache with every stable + deprecated
    // version.  Failure here is logged but does not block boot.
    match state.warm_cache().await {
        Ok(()) => tracing::info!("contract cache warmed"),
        Err(e) => tracing::warn!("failed to warm contract cache: {:?}", e),
    }

    // RFC-025: restore Kafka consumers for all currently-enabled contracts.
    // Runs in the background; boot is not blocked if Confluent is unavailable.
    state.kafka_consumers.restore_all(Arc::clone(&state)).await;
    tracing::info!("kafka consumer pool restored");
    state
        .kinesis_consumers
        .restore_all(Arc::clone(&state))
        .await;
    tracing::info!("kinesis consumer pool restored");

    // Spawn background gauge-refresh tasks (RFC-016 §Decisions Q5).
    // Must be spawned after the pool is created and the recorder is installed.
    observability::spawn_gauge_tasks(state.db.clone());
    tracing::info!("metrics gauge-refresh tasks spawned");

    let app = build_router(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("ContractGate listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
