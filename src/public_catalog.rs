//! Public Catalog — curated gov/open-data sources with ContractGate contracts.
//! (RFC-034)
//!
//! ## Routes (wired in main.rs)
//!
//! | Method | Path | Auth | Description |
//! |--------|------|------|-------------|
//! | GET  | /public-contracts           | none     | List catalog entries |
//! | GET  | /public-contracts/:id       | none     | Get one entry + YAML |
//! | POST | /contracts/:id/fork         | required | Fork into caller's org |
//! | POST | /contracts/:id/export       | required | Fetch + filter + return CSV |
//!
//! ## Source formats
//!
//! | `source_format`  | Upstream shape |
//! |------------------|----------------|
//! | `json_rows`      | `[[hdr…], [val…], …]` — Census API default |
//! | `json`           | `[{col: val}, …]` |
//! | `csv`            | delimited text; delimiter auto-detected |
//!
//! ## Export
//!
//! `POST /contracts/:id/export?format=csv` fetches the upstream source, applies
//! the fork's `fork_filter`, and streams the result as CSV.
//!
//! Parquet output is deferred (heavy dep; reserved via `?format=parquet` which
//! currently returns 501).

use axum::{
    extract::{FromRequest, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row as _};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::fork_filter::ForkFilter;
use crate::org_id_from_req;
use crate::AppState;

const MAX_UPSTREAM_BYTES: usize = 50 * 1024 * 1024; // 50 MB
const UPSTREAM_TIMEOUT_MS_DEFAULT: u64 = 5_000;
const MAX_EXPORT_ROWS: usize = 100_000;

// ---------------------------------------------------------------------------
// DB row types
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct PublicContractRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    source_url: String,
    source_format: String,
    contract_yaml: String,
    version: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PublicContractSummary {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_format: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct PublicContractDetail {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_url: String,
    pub source_format: String,
    pub contract_yaml: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct ForkResponse {
    /// New contract id in the caller's org.
    pub contract_id: Uuid,
    pub name: String,
    pub parent_public_contract_id: Uuid,
    pub fork_filter: Option<ForkFilter>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct ExportResponse {
    /// Only used for non-CSV formats (future).  For CSV the body is streamed.
    pub rows_fetched: usize,
    pub rows_after_filter: usize,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ForkRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub fork_filter: Option<ForkFilter>,
    /// When true, the fork tracks the parent's YAML version (OQ1 — deferred).
    #[serde(default)]
    pub track_parent_version: bool,
}

#[derive(Deserialize)]
pub struct ExportQuery {
    /// `csv` (default) or `parquet` (not yet implemented — returns 501).
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "csv".to_string()
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

async fn db_list_public_contracts(db: &PgPool) -> AppResult<Vec<PublicContractRow>> {
    sqlx::query_as::<_, PublicContractRow>(
        r#"SELECT id, name, description, source_url, source_format,
                  contract_yaml, version, created_at, updated_at
           FROM public_contracts
           ORDER BY name ASC"#,
    )
    .fetch_all(db)
    .await
    .map_err(|e| AppError::Internal(format!("failed to list public contracts: {e}")))
}

async fn db_get_public_contract(db: &PgPool, id: Uuid) -> AppResult<PublicContractRow> {
    sqlx::query_as::<_, PublicContractRow>(
        r#"SELECT id, name, description, source_url, source_format,
                  contract_yaml, version, created_at, updated_at
           FROM public_contracts
           WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .map_err(|e| AppError::Internal(format!("failed to fetch public contract: {e}")))?
    .ok_or_else(|| AppError::NotFound("public contract not found".into()))
}

/// Create a new fork contract in the caller's org.
/// Returns (contract_id, created_at).
async fn db_fork_public_contract(
    db: &PgPool,
    public_id: Uuid,
    name: &str,
    description: Option<&str>,
    fork_filter: Option<&ForkFilter>,
    yaml_content: &str,
    org_id: Option<Uuid>,
) -> AppResult<(Uuid, DateTime<Utc>)> {
    let filter_json = match fork_filter {
        Some(f) => Some(
            serde_json::to_value(f)
                .map_err(|e| AppError::Internal(format!("failed to serialize fork_filter: {e}")))?,
        ),
        None => None,
    };

    let pii_salt: Vec<u8> = {
        use rand::RngCore;
        let mut salt = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        salt
    };

    // Insert into contracts (identity table) with the forked YAML as the
    // initial draft version "1.0.0".
    let row = sqlx::query(
        r#"WITH ins_contract AS (
               INSERT INTO contracts (name, description, pii_salt, org_id,
                                      parent_public_contract_id, fork_filter)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, created_at
           ),
           ins_version AS (
               INSERT INTO contract_versions
                   (contract_id, version, state, yaml_content,
                    compliance_mode, egress_leakage_mode)
               SELECT id, '1.0.0', 'draft', $7, false, 'off'
               FROM ins_contract
               RETURNING contract_id
           )
           SELECT id, created_at FROM ins_contract"#,
    )
    .bind(name)
    .bind(description)
    .bind(&pii_salt)
    .bind(org_id)
    .bind(public_id)
    .bind(filter_json.as_ref())
    .bind(yaml_content)
    .fetch_one(db)
    .await
    .map_err(|e| AppError::Internal(format!("failed to create fork contract: {e}")))?;

    let contract_id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::Internal(format!("missing id from fork insert: {e}")))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::Internal(format!("missing created_at from fork insert: {e}")))?;

    Ok((contract_id, created_at))
}

/// Load the fork metadata needed for export: parent source_url, source_format,
/// and the fork's stored filter.
#[derive(sqlx::FromRow)]
struct ForkExportInfo {
    source_url: String,
    source_format: String,
    fork_filter: Option<Value>, // JSONB
}

async fn db_get_fork_export_info(db: &PgPool, contract_id: Uuid) -> AppResult<ForkExportInfo> {
    sqlx::query_as::<_, ForkExportInfo>(
        r#"SELECT pc.source_url, pc.source_format, c.fork_filter
           FROM contracts c
           JOIN public_contracts pc ON pc.id = c.parent_public_contract_id
           WHERE c.id = $1"#,
    )
    .bind(contract_id)
    .fetch_optional(db)
    .await
    .map_err(|e| AppError::Internal(format!("failed to load fork export info: {e}")))?
    .ok_or_else(|| AppError::BadRequest("contract is not a fork of a public data source".into()))
}

// ---------------------------------------------------------------------------
// Upstream fetch + parse
// ---------------------------------------------------------------------------

async fn fetch_upstream(url: &str) -> AppResult<Vec<u8>> {
    let timeout_ms = std::env::var("UPSTREAM_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(UPSTREAM_TIMEOUT_MS_DEFAULT);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| AppError::Internal(format!("failed to build HTTP client: {e}")))?;

    let resp = client
        .get(url)
        .header("User-Agent", "ContractGate/1.0 (public-catalog-export)")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("upstream fetch failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "upstream returned HTTP {}",
            resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("failed to read upstream body: {e}")))?;

    if bytes.len() > MAX_UPSTREAM_BYTES {
        return Err(AppError::BadRequest(format!(
            "upstream response too large (max {} MB)",
            MAX_UPSTREAM_BYTES / 1024 / 1024
        )));
    }

    Ok(bytes.to_vec())
}

/// Parse upstream response into a Vec of JSON objects based on source_format.
fn parse_upstream(bytes: &[u8], source_format: &str) -> AppResult<Vec<Value>> {
    match source_format {
        // Array-of-arrays: [[header…], [val…], …] — Census API default.
        "json_rows" => {
            let outer: Vec<Vec<Value>> = serde_json::from_slice(bytes)
                .map_err(|e| AppError::Internal(format!("failed to parse json_rows: {e}")))?;

            if outer.is_empty() {
                return Err(AppError::Internal("upstream returned empty array".into()));
            }

            let headers: Vec<&str> = outer[0].iter().map(|v| v.as_str().unwrap_or("")).collect();

            outer[1..]
                .iter()
                .take(MAX_EXPORT_ROWS)
                .map(|row| {
                    let mut obj = serde_json::Map::with_capacity(headers.len());
                    for (h, v) in headers.iter().zip(row.iter()) {
                        obj.insert(h.to_string(), v.clone());
                    }
                    Ok(Value::Object(obj))
                })
                .collect()
        }

        // Array of objects: [{col: val}, …].
        "json" => {
            let rows: Vec<Value> = serde_json::from_slice(bytes)
                .map_err(|e| AppError::Internal(format!("failed to parse json: {e}")))?;

            Ok(rows.into_iter().take(MAX_EXPORT_ROWS).collect())
        }

        // CSV: auto-detect delimiter, parse rows into objects.
        "csv" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| AppError::BadRequest("upstream CSV is not valid UTF-8".into()))?;

            // Auto-detect delimiter.
            let delim = detect_csv_delimiter(bytes).unwrap_or(b',');

            let mut rdr = csv::ReaderBuilder::new()
                .delimiter(delim)
                .flexible(false)
                .trim(csv::Trim::Fields)
                .from_reader(text.as_bytes());

            let headers = rdr
                .headers()
                .map_err(|e| AppError::Internal(format!("CSV header error: {e}")))?
                .clone();

            let header_vec: Vec<&str> = headers.iter().collect();

            let mut rows = Vec::new();
            for (i, result) in rdr.records().enumerate() {
                if i >= MAX_EXPORT_ROWS {
                    break;
                }
                let record = result.map_err(|e| {
                    AppError::Internal(format!("CSV parse error at row {}: {e}", i + 2))
                })?;
                let mut obj = serde_json::Map::new();
                for (h, v) in header_vec.iter().zip(record.iter()) {
                    let val = if v.trim().is_empty() {
                        Value::Null
                    } else {
                        Value::String(v.to_string())
                    };
                    obj.insert(h.to_string(), val);
                }
                rows.push(Value::Object(obj));
            }
            Ok(rows)
        }

        other => Err(AppError::Internal(format!(
            "unknown source_format: {other}"
        ))),
    }
}

fn detect_csv_delimiter(data: &[u8]) -> Option<u8> {
    let snippet = std::str::from_utf8(&data[..data.len().min(4096)]).ok()?;
    let lines: Vec<&str> = snippet.lines().take(10).collect();
    if lines.len() < 2 {
        return None;
    }
    let candidates = [b',', b'\t', b';'];
    candidates.into_iter().max_by_key(|&d| {
        let counts: Vec<usize> = lines
            .iter()
            .map(|l| l.bytes().filter(|&b| b == d).count())
            .collect();
        let max = *counts.iter().max().unwrap_or(&0);
        if max == 0 {
            0
        } else {
            counts.iter().filter(|&&c| c == max).count()
        }
    })
}

// ---------------------------------------------------------------------------
// CSV serialization
// ---------------------------------------------------------------------------

fn rows_to_csv(rows: &[Value]) -> AppResult<String> {
    if rows.is_empty() {
        return Ok(String::new());
    }

    // Collect ordered header from first row.
    let headers: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let mut wtr = csv::WriterBuilder::new().from_writer(vec![]);
    wtr.write_record(&headers)
        .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;

    for row in rows {
        let record: Vec<String> = headers
            .iter()
            .map(|h| match row.get(h) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            })
            .collect();
        wtr.write_record(&record)
            .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
    }

    let bytes = wtr
        .into_inner()
        .map_err(|e| AppError::Internal(format!("CSV flush error: {e}")))?;

    String::from_utf8(bytes).map_err(|e| AppError::Internal(format!("CSV encoding error: {e}")))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /public-contracts` — list catalog (no auth required).
pub async fn list_public_contracts_handler(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Vec<PublicContractSummary>>> {
    let rows = db_list_public_contracts(&state.db).await?;
    let summaries = rows
        .into_iter()
        .map(|r| PublicContractSummary {
            id: r.id,
            name: r.name,
            description: r.description,
            source_format: r.source_format,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();
    Ok(Json(summaries))
}

/// `GET /public-contracts/:id` — get one entry with full YAML (no auth required).
pub async fn get_public_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<PublicContractDetail>> {
    let row = db_get_public_contract(&state.db, id).await?;
    Ok(Json(PublicContractDetail {
        id: row.id,
        name: row.name,
        description: row.description,
        source_url: row.source_url,
        source_format: row.source_format,
        contract_yaml: row.contract_yaml,
        version: row.version,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// `POST /contracts/:id/fork` — fork a public contract into the caller's org.
///
/// `:id` is a `public_contracts.id` (UUID).
pub async fn fork_public_contract_handler(
    State(state): State<Arc<AppState>>,
    Path(public_id): Path<Uuid>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ForkResponse>)> {
    let org_id = org_id_from_req(&req);

    let axum::Json(body): axum::Json<ForkRequest> =
        axum::Json::from_request(req, &state)
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;

    // Load parent to get the canonical YAML (fork starts as a copy).
    let parent = db_get_public_contract(&state.db, public_id).await?;

    let (contract_id, created_at) = db_fork_public_contract(
        &state.db,
        public_id,
        &body.name,
        body.description.as_deref(),
        body.fork_filter.as_ref(),
        &parent.contract_yaml,
        org_id,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(ForkResponse {
            contract_id,
            name: body.name,
            parent_public_contract_id: public_id,
            fork_filter: body.fork_filter,
            created_at,
        }),
    ))
}

/// `POST /contracts/:id/export?format=csv` — fetch, filter, return data.
///
/// `:id` is the forked contract id (`contracts.id`).
pub async fn export_fork_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    Query(q): Query<ExportQuery>,
    _req: axum::extract::Request,
) -> AppResult<Response> {
    // Parquet is not yet implemented.
    if q.format == "parquet" {
        return Err(AppError::BadRequest(
            "parquet export is not yet implemented; use format=csv".into(),
        ));
    }
    if q.format != "csv" {
        return Err(AppError::BadRequest(format!(
            "unsupported format {:?}; supported: csv",
            q.format
        )));
    }

    // Load fork metadata (parent source_url + source_format + filter).
    let info = db_get_fork_export_info(&state.db, contract_id).await?;

    let fork_filter: Option<ForkFilter> = info
        .fork_filter
        .map(|v| {
            serde_json::from_value(v)
                .map_err(|e| AppError::Internal(format!("invalid fork_filter in DB: {e}")))
        })
        .transpose()?;

    // Fetch upstream.
    let bytes = fetch_upstream(&info.source_url).await?;
    let rows_fetched_raw = parse_upstream(&bytes, &info.source_format)?;
    let rows_fetched = rows_fetched_raw.len();

    // Apply fork filter.
    let filtered = match &fork_filter {
        Some(f) => f.apply_batch(&rows_fetched_raw),
        None => rows_fetched_raw,
    };
    let rows_after_filter = filtered.len();

    // Serialize to CSV.
    let csv_body = rows_to_csv(&filtered)?;

    tracing::info!(
        contract_id = %contract_id,
        rows_fetched,
        rows_after_filter,
        "fork export complete"
    );

    // Return CSV response.
    let response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"export.csv\"",
            ),
        ],
        csv_body,
    )
        .into_response();

    Ok(response)
}
