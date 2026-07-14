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

use arc_swap::ArcSwap;
use axum::extract::Request;
use axum::{
    extract::{FromRequest, Path, Query, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use jsonwebtoken::jwk::JwkSet;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock, RwLockReadGuard, RwLockWriteGuard,
    },
};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
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
mod infer_url;
mod ingest;
mod jwt_auth;
mod kafka_consumer;
mod kafka_ingress;
mod kinesis_consumer;
mod kinesis_ingress;
pub mod observability;
mod odcs;
mod public_catalog;
mod publication;
#[cfg(test)]
mod rag_contract_tests;
mod quarantine;
mod rate_limit;
mod replay;
mod scaffold_handler;
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
use sha2::{Digest, Sha256};
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
    /// RFC-066: explicit local-dev escape hatch. `true` ⇒ the authenticated
    /// surface accepts unauthenticated requests (and trusts `x-org-id`) for
    /// local/compose/demo use only. Driven by `CONTRACTGATE_DEV_NO_AUTH=1`;
    /// defaults to `false` so production can never silently run open. There is
    /// no longer any env-var master key — auth is JWT or a DB-backed key.
    pub dev_no_auth: bool,
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
    // RFC-052: live-swappable JWKS — replaced by the background refresh task
    // every ~10 min and on unknown-kid hits.  Inner Option is None when JWT
    // auth is disabled (no Supabase URL configured or initial fetch failed).
    pub supabase_jwks: Arc<ArcSwap<Option<JwkSet>>>,
    /// JWKS URL for the background refresh task and on-demand kid-refresh.
    /// None = no Supabase URL derived at startup.
    pub supabase_jwks_url: Option<String>,
    /// Unix-epoch-seconds of the last out-of-band kid-refresh attempt.
    /// Used to debounce refresh-on-unknown-kid to at most once per 60 s.
    pub jwks_last_kid_refresh: Arc<AtomicU64>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        dev_no_auth: bool,
        supabase_jwks: Option<JwkSet>,
        supabase_jwks_url: Option<String>,
    ) -> Self {
        AppState {
            db,
            contract_cache: RwLock::new(HashMap::new()),
            dev_no_auth,
            key_cache: Arc::new(api_key_auth::ApiKeyCache::default()),
            rate_limiter: Arc::new(rate_limit::RateLimitState::default()),
            stream_demo: std::sync::Arc::new(stream_demo::StreamDemoState::new()),
            kafka_consumers: kafka_consumer::ConsumerPool::new(),
            kinesis_consumers: kinesis_consumer::ConsumerPool::new(),
            supabase_jwks: Arc::new(ArcSwap::from_pointee(supabase_jwks)),
            supabase_jwks_url,
            jwks_last_kid_refresh: Arc::new(AtomicU64::new(0)),
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
        // RFC-054: recover from poison instead of re-panicking.  A poisoned
        // RwLock means a prior holder panicked; the inner value is intact.
        self.contract_cache.read().unwrap_or_else(|e| {
            tracing::error!("contract cache RwLock was poisoned — recovering inner value");
            e.into_inner()
        })
    }

    fn cache_write(&self) -> RwLockWriteGuard<'_, HashMap<(Uuid, String), Arc<CompiledContract>>> {
        self.contract_cache.write().unwrap_or_else(|e| {
            tracing::error!("contract cache RwLock was poisoned — recovering inner value");
            e.into_inner()
        })
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
        // Cache compilation — contract already auth-verified by handler; pass None.
        let row = storage::get_version(&self.db, contract_id, version, None).await?;
        let identity = storage::get_contract_identity(&self.db, contract_id, None).await?;
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

    /// Returns true when auth is required (the normal case). Used by org-scoped
    /// handlers to decide whether a missing `org_id` is dev-mode unscoped
    /// (false) or an unauthenticated request to reject with 401 (true).
    ///
    /// RFC-066: auth is always required unless the explicit `dev_no_auth`
    /// escape hatch is set, so this is simply `!dev_no_auth`. (Previously this
    /// also keyed off the env-var master key, which has been removed.)
    pub fn auth_configured(&self) -> bool {
        !self.dev_no_auth
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
    // identity was already org-verified before this helper is called — pass None.
    let summaries = storage::list_versions(db, id.id, None).await?;
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
// RFC-048: only `ValidatedKey` (DB-backed key or verified JWT) is trusted.
// The `x-org-id` header fallback has been removed — it was spoofable by any
// caller holding the legacy env-var key and allowed full tenant impersonation.
//
// Returns `None` only in dev mode (`CONTRACTGATE_DEV_NO_AUTH`), where no
// ValidatedKey is injected.  In production every authenticated request carries
// a ValidatedKey, so callers still check `state.auth_configured()` and return
// 401 if this is None (defense in depth).
// ---------------------------------------------------------------------------

fn org_id_from_req(req: &axum::extract::Request) -> Option<uuid::Uuid> {
    req.extensions()
        .get::<api_key_auth::ValidatedKey>()
        .map(|k| k.org_id)
}

// ---------------------------------------------------------------------------
// OrgId extractor — pulls org_id from request extensions for handlers that
// also need to extract Path<T> or Json<T> as separate parameters.
//
// Implements `FromRequestParts` so it can coexist with any other extractor.
// Always succeeds (never 400/401 on its own); the 401 check is in handlers.
// ---------------------------------------------------------------------------

pub(crate) struct OrgId(pub(crate) Option<Uuid>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for OrgId {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let org_id = parts
            .extensions
            .get::<api_key_auth::ValidatedKey>()
            .map(|k| k.org_id);
        Ok(OrgId(org_id))
    }
}

// ---------------------------------------------------------------------------
// Contract identity handlers
// ---------------------------------------------------------------------------

async fn create_contract_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ContractResponse>)> {
    let org_id = org_id_from_req(&req);

    // RFC-048 removed the x-org-id header fallback from org_id_from_req to
    // prevent tenant impersonation in production.  However, contracts.org_id
    // is NOT NULL, so an INSERT with org_id = None crashes with a DB error in
    // dev / compose mode where no ValidatedKey is injected.
    //
    // Fix: when auth is not configured (compose smoke, local dev) trust the
    // x-org-id header as a convenience so the smoke tests and demo seeder can
    // operate without a full auth setup.  This branch is gated on
    // !auth_configured() — it never executes in production, so RFC-048's
    // spoofability protection is fully preserved for all real deployments.
    let org_id = if org_id.is_none() && !state.auth_configured() {
        req.headers()
            .get("x-org-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
    } else {
        org_id
    };

    // Extract JSON body after reading extensions (req move must come after
    // the header read above, which is a borrow — order is safe).
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
    OrgId(org_id): OrgId,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ContractResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let identity = storage::get_contract_identity(&state.db, id, org_id).await?;
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
    OrgId(org_id): OrgId,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchContractRequest>,
) -> AppResult<Json<ContractResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let identity = storage::patch_contract_identity(
        &state.db,
        id,
        req.name.as_deref(),
        req.description.as_deref(),
        req.multi_stable_resolution,
        org_id,
    )
    .await?;
    // Identity-only patch doesn't touch yaml; no cache eviction needed.
    Ok(Json(identity_to_response(&state.db, identity).await?))
}

async fn delete_contract_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    storage::delete_contract(&state.db, id, org_id).await?;
    state.invalidate_contract_all(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Version handlers
// ---------------------------------------------------------------------------

async fn create_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<CreateVersionRequest>,
) -> AppResult<(StatusCode, Json<VersionResponse>)> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let v = storage::create_version(
        &state.db,
        contract_id,
        &req.version,
        &req.yaml_content,
        org_id,
    )
    .await?;
    // Drafts are cached lazily on first pin — no eager insert.
    Ok((StatusCode::CREATED, Json(VersionResponse::from(&v))))
}

async fn list_versions_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<Vec<VersionSummary>>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let versions = storage::list_versions(&state.db, contract_id, org_id).await?;
    Ok(Json(versions))
}

async fn get_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let v = storage::get_version(&state.db, contract_id, &version, org_id).await?;
    Ok(Json(VersionResponse::from(&v)))
}

async fn get_latest_stable_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    // Org-scoped existence check (RFC-047); stable lookup unscoped — same contract_id.
    let _ = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let v = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;
    Ok(Json(VersionResponse::from(&v)))
}

async fn patch_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
    Json(req): Json<PatchVersionRequest>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let v =
        storage::patch_version_yaml(&state.db, contract_id, &version, &req.yaml_content, org_id)
            .await?;
    // Evict: a draft edit changes its compiled form.
    state.invalidate_version(contract_id, &version);
    Ok(Json(VersionResponse::from(&v)))
}

async fn promote_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let v = storage::promote_version(&state.db, contract_id, &version, org_id).await?;
    // Version is now stable and frozen.  Pre-warm the cache so the first
    // ingest request doesn't take the slow path.
    let _ = state.get_compiled(contract_id, &version).await;
    Ok(Json(VersionResponse::from(&v)))
}

async fn deprecate_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let v = storage::deprecate_version(&state.db, contract_id, &version, org_id).await?;
    // Cached compiled form is still correct (YAML didn't change); the
    // deprecated-pin short-circuit in ingest.rs looks at the DB row's state,
    // not the cache.  No invalidation needed.
    Ok(Json(VersionResponse::from(&v)))
}

async fn delete_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<StatusCode> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    storage::delete_version(&state.db, contract_id, &version, org_id).await?;
    state.invalidate_version(contract_id, &version);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_name_history_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<Vec<NameHistoryEntry>>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    // list_name_history verifies ownership internally (RFC-047).
    let rows = storage::list_name_history(&state.db, contract_id, org_id).await?;
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
    // Contract was just created in this request so org is already verified.
    storage::delete_version(&state.db, identity.id, "1.0.0", org_id).await?;

    // Step 4: create the real versioned draft with correct import provenance.
    let cv = storage::create_version_from_import(
        &state.db,
        identity.id,
        &result.version,
        &result.yaml_content,
        result.import_source,
        org_id,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(VersionResponse::from(&cv))))
}

/// `GET /contracts/:id/versions/:version/export` — return the contract version
/// serialized as ODCS v3.1.0 YAML.
async fn export_odcs_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<axum::response::Response> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let identity = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    // Version scoped through the same contract — identity already org-verified.
    let cv = storage::get_version(&state.db, contract_id, &version, org_id).await?;
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
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<VersionResponse>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let cv = storage::clear_requires_review(&state.db, contract_id, &version, org_id).await?;
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
    OrgId(org_id): OrgId,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<conformance::ConformanceReport>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(error::AppError::Unauthorized);
    }
    let identity = storage::get_contract_identity(&state.db, contract_id, org_id).await?;
    let cv = storage::get_version(&state.db, contract_id, &version, org_id).await?;
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
// Health / readiness probes
// ---------------------------------------------------------------------------

/// `GET /health` — liveness probe.
///
/// Cheap, no DB.  Returns 200 as long as the process is up.  Used by Fly /
/// orchestrators to decide whether to restart the pod.  Must NOT depend on
/// the database — a DB outage should not cause the orchestrator to kill an
/// otherwise-live process.
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "contractgate"
    }))
}

/// `GET /ready` — readiness probe (RFC-053).
///
/// Runs `SELECT 1` against the DB pool with a 2-second timeout.
/// - `200 {"status":"ready","db":"ok"}` — pool is healthy.
/// - `503 {"status":"degraded","db":"error"}` — pool exhausted or Supabase down.
///
/// Platform health checks (Fly, docker-compose) should point at `/ready`;
/// `/health` is the liveness probe and intentionally never touches the DB.
async fn ready_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = &state.db;
    let probe = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(db),
    )
    .await;

    let size = db.size();
    let idle = db.num_idle();

    match probe {
        Ok(Ok(_)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ready",
                "db": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "pool": { "size": size, "idle": idle },
            })),
        )
            .into_response(),
        Ok(Err(e)) => {
            tracing::warn!("readiness probe DB error: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "status": "degraded", "db": "error" })),
            )
                .into_response()
        }
        Err(_elapsed) => {
            tracing::warn!("readiness probe timed out after 2s");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "status": "degraded", "db": "error" })),
            )
                .into_response()
        }
    }
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
// RFC-052: out-of-band JWKS kid-refresh helper
// ---------------------------------------------------------------------------

/// Attempt an out-of-band JWKS refresh when a token's kid is not in the
/// cached set.  Debounced to at most one network fetch per 60 seconds to
/// avoid hammering the Supabase JWKS endpoint under a sustained rotation.
///
/// Returns `true` if a refresh was attempted AND succeeded (the store has
/// been updated).  Returns `false` on debounce hit or fetch failure
/// (previous keys are preserved either way).
async fn maybe_refresh_jwks_on_unknown_kid(state: &AppState) -> bool {
    let Some(url) = &state.supabase_jwks_url else {
        return false;
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let last = state.jwks_last_kid_refresh.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 60 {
        tracing::debug!(
            secs_since_last = now.saturating_sub(last),
            "JWKS kid-refresh debounced"
        );
        return false;
    }

    // CAS to claim the refresh slot — prevents concurrent duplicate fetches.
    if state
        .jwks_last_kid_refresh
        .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return false; // another task won the race
    }

    tracing::info!("Triggering out-of-band JWKS refresh (unknown kid)");
    match jwt_auth::fetch_jwks(url).await {
        Ok(new_jwks) => {
            tracing::info!(keys = new_jwks.keys.len(), "JWKS refreshed on unknown kid");
            state.supabase_jwks.store(Arc::new(Some(new_jwks)));
            true
        }
        Err(e) => {
            tracing::warn!("JWKS kid-refresh fetch failed: {e}");
            // Leave jwks_last_kid_refresh at `now` (already written by the
            // CAS above).  Failures are debounced to at most once per 60 s —
            // same as successes — so a down JWKS endpoint cannot cause a
            // thundering herd of concurrent re-fetches.
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn require_api_key(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<axum::response::Response, error::AppError> {
    // RFC-039: Bearer JWT wins — used by the dashboard after Supabase sign-in.
    // Hard-reject on a bad token so clients see the real error (expired, invalid
    // signature) rather than silently falling through to a 401 about x-api-key.
    if let Some(bearer) = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
    {
        // RFC-052: load the live JWKS snapshot (lock-free ArcSwap read).
        let jwks_guard = state.supabase_jwks.load();
        if let Some(jwks) = jwks_guard.as_ref() {
            // First verification attempt with the current key set.
            let result = jwt_auth::verify_supabase_jwt(&bearer, jwks, &state.db).await;

            // On NoMatchingKey, attempt a debounced out-of-band refresh and
            // retry once.  This makes key rotation near-instant instead of
            // waiting up to 10 min for the next periodic tick.
            let result = match result {
                Err(jwt_auth::JwtAuthError::NoMatchingKey) => {
                    if maybe_refresh_jwks_on_unknown_kid(&state).await {
                        let new_guard = state.supabase_jwks.load();
                        if let Some(new_jwks) = new_guard.as_ref() {
                            jwt_auth::verify_supabase_jwt(&bearer, new_jwks, &state.db).await
                        } else {
                            Err(jwt_auth::JwtAuthError::NoMatchingKey)
                        }
                    } else {
                        Err(jwt_auth::JwtAuthError::NoMatchingKey)
                    }
                }
                other => other,
            };

            match result {
                Ok(validated) => {
                    // RFC-043 fix-1: key by user_id, not api_key_id (nil UUID).
                    // api_key_id = Uuid::nil() is the "JWT session" sentinel used
                    // in audit logs — it must not double as a rate-limit key or
                    // every dashboard user shares one global bucket.
                    let outcome = state.rate_limiter.check(
                        validated.user_id, // real Supabase user UUID
                        Some(500),
                        Some(2_000),
                    );
                    if !outcome.allowed {
                        return Err(error::AppError::RateLimitExceeded);
                    }
                    request.extensions_mut().insert(validated);
                    return Ok(next.run(request).await);
                }
                Err(e) => {
                    tracing::warn!("JWT verification failed: {e}");
                    return Err(error::AppError::Unauthorized);
                }
            }
        } else {
            tracing::warn!("Bearer token received but JWKS not loaded — rejecting");
            return Err(error::AppError::Unauthorized);
        }
    }

    let provided = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    // RFC-066: explicit local-dev escape hatch — pass through unauthenticated.
    // Never set in production (defaults off).
    if state.dev_no_auth {
        return Ok(next.run(request).await);
    }

    // DB-backed key: validate via cache (60-second TTL). This is now the only
    // `x-api-key` credential path — the legacy env-var master key was removed
    // in RFC-066.
    if !provided.is_empty() {
        match state.key_cache.validate(&provided, &state.db).await {
            Ok(validated) => {
                // P1-1: rate-limit DB-backed keys using per-key overrides.
                let outcome = state.rate_limiter.check(
                    validated.api_key_id,
                    validated.rate_limit_rps,
                    validated.rate_limit_burst,
                );
                if !outcome.allowed {
                    return Err(error::AppError::RateLimitExceeded);
                }
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
// RFC-064: SMT version probe
// ---------------------------------------------------------------------------

/// Response body for `GET /v1/contracts/{contract_id}/version`.
#[derive(serde::Serialize)]
struct ContractVersionProbe {
    /// Semver string of the latest stable version.
    version: String,
    /// SHA-256 hex digest of the YAML content.  The SMT polls this and swaps
    /// its cached contract only when the hash changes — avoids a full body
    /// fetch on every poll interval.
    hash: String,
}

/// `GET /v1/contracts/{contract_id}/version` — cheap version probe for the
/// Kafka Connect SMT (RFC-064 dynamic contract reload).
///
/// Returns the latest stable version string and a SHA-256 hash of its YAML
/// content.  The SMT polls this endpoint and triggers a reload only when the
/// hash changes, keeping the hot-path cost to a single small JSON GET per
/// `contractgate.reload.poll.ms`.
///
/// Auth: same `x-api-key` / Bearer JWT as all other protected routes.
async fn v1_contract_version_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
) -> AppResult<Json<ContractVersionProbe>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    // Verify the caller can see this contract (org-scoped check).
    let _ = storage::get_contract_identity(&state.db, contract_id, org_id).await?;

    let v = storage::get_latest_stable_version(&state.db, contract_id)
        .await?
        .ok_or(AppError::NoStableVersion { contract_id })?;

    let hash = hex::encode(Sha256::digest(v.yaml_content.as_bytes()));

    Ok(Json(ContractVersionProbe {
        version: v.version,
        hash,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn build_router(state: Arc<AppState>) -> Router {
    // RFC-050: two CORS layers with different scopes.
    //
    // `protected_cors` — authenticated surface (protected, infer, v1 routers).
    // Origins come from DASHBOARD_ORIGIN (comma-separated).  Unset → warn +
    // fall back to http://localhost:3000 for local dev.
    let allowed_origins: Vec<axum::http::HeaderValue> = std::env::var("DASHBOARD_ORIGIN")
        .unwrap_or_else(|_| {
            tracing::warn!(
                "DASHBOARD_ORIGIN is not set — CORS restricted to http://localhost:3000. \
                 Set DASHBOARD_ORIGIN in production."
            );
            "http://localhost:3000".to_string()
        })
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<axum::http::HeaderValue>().ok())
        .collect();

    let protected_cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed_origins))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);

    // `public_cors` — genuinely public endpoints (no tenant data, no bearer
    // tokens).  Permissive so browsers and third-party tools can embed them.
    let public_cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Public routes — no auth required
    let public = Router::new()
        .route("/health", get(health_handler))
        // RFC-053: readiness probe — SELECT 1 with 2s timeout.
        // Platform health checks should point here; /health is liveness only.
        .route("/ready", get(ready_handler))
        .route("/openapi.json", get(v1_ingest::openapi_handler))
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
        // Evolution diff summarizer (RFC-006)
        .route("/contracts/diff", post(infer_diff::diff_handler))
        // Brownfield contract scaffolder (RFC-024)
        .route(
            "/contracts/scaffold",
            post(scaffold_handler::scaffold_handler),
        )
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
        // Org-scoped top-level quarantine API (RFC-081) — backs the dashboard
        // Quarantine tab: list, replay-by-event-id, and per-event history.
        .route("/quarantine", get(quarantine::list_quarantine_handler))
        .route("/quarantine/replay", post(quarantine::replay_all_handler))
        .route(
            "/quarantine/replay-history",
            get(quarantine::replay_history_all_handler),
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
        // P1-3: playground now requires auth (moved from public router).
        // Previously unauthenticated + unlimited; now gated + capped at 1 MB
        // along with the rest of the protected surface.
        .route("/playground/validate", post(playground_handler))
        // P1-2: 1 MB body limit on all protected routes.
        // /contracts/infer/* is carved out below with a 10 MB cap (RFC-043 fix-2).
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            1024 * 1024, // 1 MB
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    // RFC-043 fix-2: inference routes get 10 MB — real OpenAPI specs (e.g.
    // Stripe's) exceed 1 MB; capping them at 1 MB causes 413 on first real use.
    // Auth middleware applied here so both groups share the same auth path.
    let infer = Router::new()
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
        // URL inference (RFC-037)
        .route("/contracts/infer/url", post(infer_url::infer_url_handler))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            10 * 1024 * 1024, // 10 MB — matches v1_ingest cap
        ))
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
        // RFC-064: SMT version probe — cheap GET for dynamic contract reload.
        // Placed in the v1 router so the SMT can use the same API key and
        // namespace as the ingest route.
        .route(
            "/v1/contracts/{contract_id}/version",
            get(v1_contract_version_handler),
        )
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            10 * 1024 * 1024, // 10 MB — RFC-021
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    // RFC-050: scoped CORS layers.
    // public routes get the permissive wildcard layer;
    // authenticated routers get the origin-allowlist layer.
    // Layer order matters in Axum: .layer() wraps from outermost in,
    // so we apply CORS after merging so it wraps the correct sub-tree.
    let public_router = public.layer(public_cors);
    let auth_router = Router::new()
        .merge(protected)
        .merge(infer)
        .merge(v1)
        .layer(protected_cors);

    Router::new()
        .merge(public_router)
        .merge(auth_router)
        .layer(middleware::from_fn(observability::track_requests))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
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

    // RFC-066: explicit local-dev escape hatch. Defaults off so production is
    // always authenticated (JWT or DB-backed key). The legacy env-var master
    // key was removed.
    let dev_no_auth = std::env::var("CONTRACTGATE_DEV_NO_AUTH")
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE"))
        .unwrap_or(false);
    if dev_no_auth {
        tracing::warn!(
            "CONTRACTGATE_DEV_NO_AUTH is set — authenticated surface is OPEN (local/dev only)"
        );
    } else {
        tracing::info!("Authentication required (JWT + DB-backed API keys)");
    }

    // RFC-039: fetch Supabase JWKS for RS256 dashboard session auth.
    // Resolution order:
    //   1. SUPABASE_URL env var (explicit override — set this on Fly if
    //      DATABASE_URL parsing doesn't yield the correct project URL)
    //   2. Derived from DATABASE_URL (handles direct + pooler formats)
    let jwks_url: Option<String> = std::env::var("SUPABASE_URL")
        .ok()
        .map(|u| format!("{}/auth/v1/.well-known/jwks.json", u.trim_end_matches('/')))
        .or_else(|| jwt_auth::jwks_url_from_database_url(&database_url));

    // RFC-052: initial fetch does not wrap in Arc — AppState::new does that.
    // A fetch failure is non-fatal: the background task will recover once the
    // network is up (today a startup failure was permanent until restart).
    let supabase_jwks: Option<JwkSet> = match &jwks_url {
        Some(url) => match jwt_auth::fetch_jwks(url).await {
            Ok(jwks) => {
                tracing::info!("Supabase JWKS loaded from {url} — Bearer JWT auth enabled");
                Some(jwks)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to fetch Supabase JWKS from {url}: {e} — \
                     JWT auth disabled at startup; background task will retry"
                );
                None
            }
        },
        None => {
            tracing::warn!(
                "Could not derive Supabase URL from DATABASE_URL and SUPABASE_URL is \
                 not set — Bearer JWT auth disabled"
            );
            None
        }
    };

    let state = Arc::new(AppState::new(pool, dev_no_auth, supabase_jwks, jwks_url));

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

    // RFC-051: spawn the API-key cache sweeper — runs every 5 min, evicts
    // expired entries and enforces the MAX_CACHE_ENTRIES cap.
    api_key_auth::ApiKeyCache::spawn_sweeper(state.key_cache.clone());
    tracing::info!("api-key cache sweeper spawned");

    // RFC-052: spawn the JWKS background refresh task (every ~10 min).
    // No-op when supabase_jwks_url is None (no Supabase URL configured).
    jwt_auth::spawn_jwks_refresh_task(
        Arc::clone(&state.supabase_jwks),
        state.supabase_jwks_url.clone(),
    );
    tracing::info!("JWKS background refresh task spawned");

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
