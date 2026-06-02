//! HTTP handler for `POST /contracts/scaffold` — RFC-024.
//!
//! Accepts JSON sample objects or raw file content (NDJSON / Avro schema /
//! Protobuf) and returns a scaffolded draft contract YAML with embedded
//! `# scaffold:` stat comments and PII TODO annotations.
//!
//! Developer tooling — not part of the patent-core validation engine.

use crate::error::{AppError, AppResult};
use axum::Json;
use contractgate::scaffold::{scaffold_from_file, ScaffoldConfig, ScaffoldResult};
use serde::{Deserialize, Serialize};
use std::io::Write as _;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ScaffoldRequest {
    /// Name embedded in the generated contract YAML.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Raw file content — for ndjson / avro_schema / proto formats.
    /// Mutually exclusive with `samples`.
    #[serde(default)]
    pub content: Option<String>,
    /// JSON sample objects — shorthand for `format = "json"`.
    /// Mutually exclusive with `content`.
    #[serde(default)]
    pub samples: Option<Vec<serde_json::Value>>,
    /// Input format hint: `"json"`, `"ndjson"`, `"avro_schema"`, `"proto"`.
    /// Auto-detected from content when omitted.
    #[serde(default)]
    pub format: Option<String>,
    /// Skip value profiling (field names + types only). Faster, less rich.
    #[serde(default)]
    pub fast: bool,
    /// PII confidence threshold (0.0–1.0). Default: 0.4.
    #[serde(default = "default_pii_threshold")]
    pub pii_threshold: f32,
    /// Maximum records to process. Default: 1 000.
    #[serde(default = "default_max_records")]
    pub max_records: usize,
}

fn default_pii_threshold() -> f32 {
    0.4
}
fn default_max_records() -> usize {
    1_000
}

#[derive(Serialize)]
pub struct ScaffoldResponse {
    /// Draft contract YAML with embedded `# scaffold:` comments.
    pub yaml_content: String,
    /// Number of top-level fields discovered.
    pub field_count: usize,
    /// Number of sample records processed.
    pub sample_count: usize,
    /// Number of PII candidates detected.
    pub pii_candidate_count: usize,
    /// Per-field PII candidates (for UI highlighting).
    pub pii_candidates: Vec<PiiCandidateDto>,
    /// Detected/used input format label.
    pub format: String,
    /// True when Schema Registry was unreachable during Avro/Proto decoding.
    pub sr_unavailable: bool,
}

#[derive(Serialize)]
pub struct PiiCandidateDto {
    pub field_name: String,
    pub confidence: f32,
    pub reason: String,
    pub suggested_transform: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn scaffold_handler(
    Json(req): Json<ScaffoldRequest>,
) -> AppResult<Json<ScaffoldResponse>> {
    let (ext, content) = resolve_input(&req)?;

    let config = ScaffoldConfig {
        name: req.name.clone(),
        description: req.description.clone(),
        pii_threshold: req.pii_threshold,
        max_records: req.max_records,
        fast: req.fast,
        ..Default::default()
    };

    // scaffold_from_file is synchronous (CPU-bound profiling) — run in a
    // blocking thread so we don't stall the async reactor.
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<ScaffoldResult> {
        let mut tmp = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .map_err(|e| anyhow::anyhow!("temp file creation failed: {e}"))?;
        tmp.write_all(content.as_bytes())
            .map_err(|e| anyhow::anyhow!("temp file write failed: {e}"))?;
        scaffold_from_file(tmp.path(), &config)
    })
    .await
    .map_err(|e| AppError::Internal(format!("scaffold task panicked: {e}")))?
    .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let field_count = count_named_fields(&result.contract_yaml);

    Ok(Json(ScaffoldResponse {
        yaml_content: result.contract_yaml,
        field_count,
        sample_count: result.sample_count,
        pii_candidate_count: result.pii_candidates.len(),
        pii_candidates: result
            .pii_candidates
            .into_iter()
            .map(|c| PiiCandidateDto {
                field_name: c.field_name,
                confidence: c.confidence,
                reason: c.reason,
                suggested_transform: c.suggested_transform.to_string(),
            })
            .collect(),
        format: result.format.display().to_string(),
        sr_unavailable: result.sr_unavailable,
    }))
}

// ---------------------------------------------------------------------------
// Input resolution
// ---------------------------------------------------------------------------

/// Determine the file extension and content string from the request.
fn resolve_input(req: &ScaffoldRequest) -> AppResult<(String, String)> {
    // samples[] → serialize as a JSON array and treat as .json
    if let Some(ref samples) = req.samples {
        if samples.is_empty() {
            return Err(AppError::BadRequest(
                "samples array must not be empty".into(),
            ));
        }
        for (i, s) in samples.iter().enumerate() {
            if !s.is_object() {
                return Err(AppError::BadRequest(format!(
                    "samples[{i}] is not a JSON object"
                )));
            }
        }
        let content = serde_json::to_string(samples)
            .map_err(|e| AppError::Internal(format!("samples serialisation: {e}")))?;
        return Ok(("json".to_string(), content));
    }

    // content string path
    let content = req
        .content
        .clone()
        .ok_or_else(|| AppError::BadRequest("provide either 'samples' or 'content'".into()))?;

    if content.trim().is_empty() {
        return Err(AppError::BadRequest("content must not be empty".into()));
    }

    let ext = match req.format.as_deref() {
        Some("json") => "json",
        Some("ndjson") => "ndjson",
        Some("avro_schema") | Some("avsc") => "avsc",
        Some("proto") => "proto",
        None => infer_format(&content),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "unknown format '{other}' — supported: json, ndjson, avro_schema, proto"
            )))
        }
    };

    Ok((ext.to_string(), content))
}

/// Heuristically detect input format from the raw content string.
fn infer_format(content: &str) -> &'static str {
    let trimmed = content.trim_start();
    if trimmed.starts_with('[') {
        "json" // JSON array of objects
    } else if trimmed.starts_with('{') {
        // JSON object — could be an Avro schema or a single JSON event
        if trimmed.contains("\"type\"") && trimmed.contains("\"record\"") {
            "avsc"
        } else {
            "json"
        }
    } else if trimmed.contains("message ") && trimmed.contains('{') {
        "proto"
    } else {
        "ndjson" // default for line-delimited content
    }
}

/// Count `- name:` field entries in the emitted YAML (top-level + nested).
fn count_named_fields(yaml: &str) -> usize {
    yaml.lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("- name:") && !t.starts_with('#')
        })
        .count()
}
