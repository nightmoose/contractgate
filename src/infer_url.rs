//! URL contract inference — fetch an HTTP endpoint and derive a draft contract.
//!
//! `POST /contracts/infer/url`
//!
//! Fetches the caller-supplied URL, detects whether the response is JSON or
//! CSV, and runs the same inference engine as `infer_csv` / `infer` to produce
//! a draft YAML contract.
//!
//! ## Format detection
//!
//! | Signal | Format |
//! |---|---|
//! | `Content-Type: text/csv` or `text/plain` | CSV |
//! | URL path ends with `.csv` | CSV |
//! | Everything else | JSON |
//!
//! ## JSON shape handling
//!
//! | Response shape | Handling |
//! |---|---|
//! | `[{…}, …]` array of objects | Infer directly |
//! | `{…}` single object | Wrap in `[obj]`, infer |
//! | `{"data":[…]}` / `"items"` / `"results"` / `"records"` / `"rows"` | Unwrap inner array |
//!
//! ## Limits
//!
//! - Max body: 10 MB (`MAX_INFER_URL_BYTES`).
//! - Timeout: `INFER_URL_TIMEOUT_MS` env var (default 10 000 ms).
//! - Max sampled rows: 1 000 (same as CSV inference).
//!
//! ## Security note
//!
//! Caller-supplied headers are forwarded verbatim. SSRF mitigation (blocking
//! private IP ranges) is deferred to a follow-up security pass (RFC-037 OQ1).

use crate::contract::{Contract, EgressLeakageMode, Ontology};
use crate::error::{AppError, AppResult};
use crate::infer::{infer_fields_from_objects_pub, InferResponse};
use crate::infer_csv;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const MAX_INFER_URL_BYTES: usize = 10 * 1024 * 1024; // 10 MB
const MAX_SAMPLE_ROWS: usize = 1_000;
const TIMEOUT_MS_DEFAULT: u64 = 10_000;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct InferUrlRequest {
    /// Name for the generated contract.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// HTTP(S) URL to fetch.
    pub url: String,
    /// Optional headers forwarded to the upstream request (e.g. auth tokens).
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(serde::Serialize)]
pub struct InferUrlResponse {
    pub yaml_content: String,
    pub field_count: usize,
    pub sample_count: usize,
    /// `"json"` or `"csv"`
    pub detected_format: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_url_handler(
    Json(req): Json<InferUrlRequest>,
) -> AppResult<Json<InferUrlResponse>> {
    // 1. Validate URL.
    validate_url(&req.url)?;

    // 2. Fetch upstream.
    let timeout_ms = std::env::var("INFER_URL_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(TIMEOUT_MS_DEFAULT);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| AppError::Internal(format!("failed to build HTTP client: {e}")))?;

    let mut request = client
        .get(&req.url)
        .header("User-Agent", "ContractGate/1.0 (infer-url)");

    if let Some(hdrs) = &req.headers {
        for (k, v) in hdrs {
            request = request.header(k.as_str(), v.as_str());
        }
    }

    let resp = request.send().await.map_err(|e| {
        if e.is_timeout() {
            AppError::GatewayTimeout(format!(
                "upstream timed out after {} ms",
                timeout_ms
            ))
        } else {
            AppError::BadRequest(format!("could not reach URL: {e}"))
        }
    })?;

    if !resp.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "upstream returned HTTP {}",
            resp.status()
        )));
    }

    // 3. Detect format before consuming body.
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_csv = content_type.contains("text/csv")
        || content_type.contains("text/plain")
        || req.url.to_ascii_lowercase().ends_with(".csv");

    // 4. Read body, enforce size limit.
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("failed to read upstream body: {e}")))?;

    if bytes.len() > MAX_INFER_URL_BYTES {
        return Err(AppError::BadRequest(format!(
            "upstream response too large (max {} MB)",
            MAX_INFER_URL_BYTES / 1024 / 1024
        )));
    }

    if bytes.is_empty() {
        return Err(AppError::BadRequest("upstream returned an empty body".into()));
    }

    // 5. Parse & infer.
    let description = req
        .description
        .clone()
        .unwrap_or_else(|| format!("Inferred from {}", req.url));

    if is_csv {
        infer_from_csv_bytes(&bytes, &req.name, &description).map(Json)
    } else {
        infer_from_json_bytes(&bytes, &req.name, &description).map(Json)
    }
}

// ---------------------------------------------------------------------------
// URL validation
// ---------------------------------------------------------------------------

fn validate_url(url: &str) -> AppResult<()> {
    if url.is_empty() {
        return Err(AppError::BadRequest("url is required".into()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }
    if url.len() > 2_048 {
        return Err(AppError::BadRequest("url is too long (max 2048 chars)".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON path
// ---------------------------------------------------------------------------

fn infer_from_json_bytes(
    bytes: &[u8],
    name: &str,
    description: &str,
) -> AppResult<InferUrlResponse> {
    let raw: Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::BadRequest(format!("response is not valid JSON: {e}")))?;

    let rows = extract_rows(raw)?;

    if rows.is_empty() {
        return Err(AppError::UnprocessableEntity(
            "no records found in response — cannot infer contract".into(),
        ));
    }

    let sample: Vec<Value> = rows.into_iter().take(MAX_SAMPLE_ROWS).collect();
    let sample_count = sample.len();
    let entities = infer_fields_from_objects_pub(&sample);
    let field_count = entities.len();

    if field_count == 0 {
        return Err(AppError::UnprocessableEntity(
            "could not infer any fields from response".into(),
        ));
    }

    let contract = Contract {
        version: "1.0".to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        compliance_mode: false,
        egress_leakage_mode: EgressLeakageMode::Off,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| AppError::Internal(format!("yaml serialisation failed: {e}")))?;

    Ok(InferUrlResponse {
        yaml_content,
        field_count,
        sample_count,
        detected_format: "json".to_string(),
    })
}

/// Extract a `Vec<Value>` of objects from a JSON response of unknown shape.
///
/// Handles:
/// - `[{…}, …]`  — array of objects, used directly
/// - `{…}`       — single object, wrapped in a one-element vec
/// - `{"data": […], …}` — common API envelope; probes well-known keys
fn extract_rows(value: Value) -> AppResult<Vec<Value>> {
    match value {
        Value::Array(arr) => {
            // Filter to only object elements; skip nulls/primitives.
            let objs: Vec<Value> = arr.into_iter().filter(|v| v.is_object()).collect();
            Ok(objs)
        }
        Value::Object(ref map) => {
            // Try common envelope keys first.
            const ENVELOPE_KEYS: &[&str] = &["data", "items", "results", "records", "rows"];
            for key in ENVELOPE_KEYS {
                if let Some(Value::Array(inner)) = map.get(*key) {
                    let objs: Vec<Value> =
                        inner.iter().filter(|v| v.is_object()).cloned().collect();
                    if !objs.is_empty() {
                        return Ok(objs);
                    }
                }
            }
            // Fall back: treat the single object as a one-row sample.
            Ok(vec![value])
        }
        _ => Err(AppError::BadRequest(
            "unexpected JSON shape — expected an array of objects or an envelope object".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// CSV path — delegate to infer_csv internals via public re-export
// ---------------------------------------------------------------------------

fn infer_from_csv_bytes(
    bytes: &[u8],
    name: &str,
    description: &str,
) -> AppResult<InferUrlResponse> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AppError::BadRequest("upstream CSV is not valid UTF-8".into()))?;

    let InferResponse {
        yaml_content,
        field_count,
        sample_count,
    } = infer_csv::infer_from_text(text, name, Some(description), None)?;

    Ok(InferUrlResponse {
        yaml_content,
        field_count,
        sample_count,
        detected_format: "csv".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_url_rejects_non_http() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("").is_err());
        assert!(validate_url("not-a-url").is_err());
    }

    #[test]
    fn validate_url_accepts_http_and_https() {
        assert!(validate_url("http://example.com/data").is_ok());
        assert!(validate_url("https://api.example.com/v1/events?limit=100").is_ok());
    }

    #[test]
    fn extract_rows_from_array() {
        let v = json!([{"id": 1}, {"id": 2}]);
        let rows = extract_rows(v).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn extract_rows_from_data_envelope() {
        let v = json!({"data": [{"id": 1}, {"id": 2}], "total": 2});
        let rows = extract_rows(v).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn extract_rows_from_items_envelope() {
        let v = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let rows = extract_rows(v).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn extract_rows_single_object_fallback() {
        let v = json!({"id": 1, "name": "alice"});
        let rows = extract_rows(v).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn extract_rows_rejects_primitive() {
        assert!(extract_rows(json!("just a string")).is_err());
        assert!(extract_rows(json!(42)).is_err());
    }

    #[test]
    fn infer_from_json_bytes_basic() {
        let body = serde_json::to_vec(&json!([
            {"user_id": "u1", "amount": 10, "active": true},
            {"user_id": "u2", "amount": 20, "active": false},
        ]))
        .unwrap();
        let res = infer_from_json_bytes(&body, "test_contract", "Test").unwrap();
        assert_eq!(res.field_count, 3);
        assert_eq!(res.sample_count, 2);
        assert_eq!(res.detected_format, "json");
        assert!(res.yaml_content.contains("test_contract"));
    }

    #[test]
    fn infer_from_json_bytes_with_envelope() {
        let body = serde_json::to_vec(&json!({
            "results": [
                {"city": "NYC", "pop": 8_000_000},
                {"city": "LA",  "pop": 4_000_000},
            ],
            "total": 2
        }))
        .unwrap();
        let res = infer_from_json_bytes(&body, "cities", "Desc").unwrap();
        assert_eq!(res.field_count, 2);
        assert_eq!(res.sample_count, 2);
    }
}
