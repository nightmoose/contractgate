//! ContractGate — Real-time semantic contract enforcement gateway.
//! Patent Pending.
//!
//! Starts an Axum HTTP server with routes for:
//!   - Contract management (CRUD)
//!   - Ingestion API (POST /ingest/{contract_id})
//!   - Audit log queries
//!   - Health check

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Json,
    routing::{get, post},
    Router,
};
use axum::extract::Request;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
    timeout::TimeoutLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

mod contract;
mod error;
mod ingest;
mod storage;
mod validation;
#[cfg(test)]
mod tests;

use contract::{
    ContractResponse, ContractSummary, CreateContractRequest, UpdateContractRequest,
};
use error::{AppError, AppResult};
use validation::CompiledContract;

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Shared application state passed to every Axum handler.
pub struct AppState {
    pub db: PgPool,
    /// In-memory cache of compiled contracts (contract_id → compiled).
    /// Avoids re-parsing + re-compiling regex on every request.
    contract_cache: RwLock<HashMap<Uuid, Arc<CompiledContract>>>,
    /// Optional API key — if set, all protected routes require `x-api-key: <key>`.
    /// Leave empty (or unset API_KEY env var) to run without auth (dev only).
    pub api_key: String,
}

impl AppState {
    pub fn new(db: PgPool, api_key: String) -> Self {
        AppState {
            db,
            contract_cache: RwLock::new(HashMap::new()),
            api_key,
        }
    }

    /// Retrieve a compiled contract from cache, loading from DB if necessary.
    pub async fn get_compiled_contract(&self, id: Uuid) -> Option<Arc<CompiledContract>> {
        // Fast path: check cache under read lock
        {
            let cache = self.contract_cache.read().unwrap();
            if let Some(cc) = cache.get(&id) {
                return Some(Arc::clone(cc));
            }
        }

        // Slow path: load from DB and compile
        let mut stored = storage::get_contract(&self.db, id).await.ok()?;
        if !stored.active {
            return None;
        }

        let parsed = stored.parsed.take()?;
        let compiled = CompiledContract::compile(parsed).ok()?;
        let arc = Arc::new(compiled);

        // Write into cache
        {
            let mut cache = self.contract_cache.write().unwrap();
            cache.insert(id, Arc::clone(&arc));
        }

        Some(arc)
    }

    /// Invalidate a contract from the cache (called after updates / deletes).
    pub fn invalidate_contract(&self, id: Uuid) {
        let mut cache = self.contract_cache.write().unwrap();
        cache.remove(&id);
    }
}

// ---------------------------------------------------------------------------
// Contract handlers
// ---------------------------------------------------------------------------

async fn create_contract_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateContractRequest>,
) -> AppResult<(StatusCode, Json<ContractResponse>)> {
    let stored = storage::create_contract(&state.db, &req.yaml_content).await?;
    let resp = ContractResponse::from(&stored);
    Ok((StatusCode::CREATED, Json(resp)))
}

async fn get_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ContractResponse>> {
    let stored = storage::get_contract(&state.db, id).await?;
    Ok(Json(ContractResponse::from(&stored)))
}

async fn list_contracts_handler(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Vec<ContractSummary>>> {
    let contracts = storage::list_contracts(&state.db).await?;
    Ok(Json(contracts))
}

async fn update_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateContractRequest>,
) -> AppResult<Json<ContractResponse>> {
    // Handle yaml_content replacement first (re-parses + re-validates the YAML)
    if let Some(ref yaml) = req.yaml_content {
        storage::update_contract_yaml(&state.db, id, yaml).await?;
        // Evict cached compiled contract — it will be rebuilt on next request
        state.invalidate_contract(id);
    }

    // Handle active flag toggle
    if let Some(active) = req.active {
        storage::update_contract_active(&state.db, id, active).await?;
        state.invalidate_contract(id);
    }

    let stored = storage::get_contract(&state.db, id).await?;
    Ok(Json(ContractResponse::from(&stored)))
}

async fn delete_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    storage::delete_contract(&state.db, id).await?;
    state.invalidate_contract(id);
    Ok(StatusCode::NO_CONTENT)
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
) -> AppResult<Json<Vec<storage::AuditEntry>>> {
    let entries =
        storage::recent_audit_entries(&state.db, q.contract_id, q.limit.min(500), q.offset)
            .await?;
    Ok(Json(entries))
}

async fn global_stats_handler(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<storage::IngestionStats>> {
    let stats = storage::ingestion_stats(&state.db, None).await?;
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
// Test: validate a contract YAML + event without ingestion
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct PlaygroundRequest {
    yaml_content: String,
    event: serde_json::Value,
}

async fn playground_handler(
    Json(req): Json<PlaygroundRequest>,
) -> AppResult<Json<validation::ValidationResult>> {
    let contract: contract::Contract = serde_yaml::from_str(&req.yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let compiled = CompiledContract::compile(contract)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let result = validation::validate(&compiled, &req.event);
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

/// Tower middleware that enforces `x-api-key` on all protected routes.
///
/// If `API_KEY` env var is unset or empty the check is skipped (dev mode).
/// In production, set `API_KEY` via `fly secrets set API_KEY=<secret>`.
async fn require_api_key(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<axum::response::Response, error::AppError> {
    // If no API key is configured, allow all traffic (useful for local dev)
    if state.api_key.is_empty() {
        return Ok(next.run(request).await);
    }

    let provided = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided == state.api_key {
        Ok(next.run(request).await)
    } else {
        tracing::warn!("Rejected request: missing or invalid x-api-key");
        Err(error::AppError::Unauthorized)
    }
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
        .route("/playground/validate", post(playground_handler));

    // Protected routes — require x-api-key header
    let protected = Router::new()
        // Contract management
        .route("/contracts", get(list_contracts_handler).post(create_contract_handler))
        .route(
            "/contracts/:id",
            get(get_contract_handler)
                .put(update_contract_handler)
                .delete(delete_contract_handler),
        )
        // Ingestion
        .route("/ingest/:contract_id", post(ingest::ingest_handler))
        .route("/ingest/:contract_id/stats", get(ingest::ingest_stats_handler))
        // Audit + stats
        .route("/audit", get(audit_log_handler))
        .route("/stats", get(global_stats_handler))
        .layer(middleware::from_fn_with_state(state.clone(), require_api_key));

    Router::new()
        .merge(public)
        .merge(protected)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(30)))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present
    dotenvy::dotenv().ok();

    // Tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "contractgate=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Database
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = PgPool::connect(&database_url).await?;
    tracing::info!("Connected to database");

    // API key — warn loudly if unset in production
    let api_key = std::env::var("API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        tracing::warn!("API_KEY is not set — running without authentication (dev mode only)");
    } else {
        tracing::info!("API key authentication enabled");
    }

    let state = Arc::new(AppState::new(pool, api_key));
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
