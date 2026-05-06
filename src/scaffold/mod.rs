//! Brownfield contract scaffolder — RFC-024.
//!
//! Derives a draft ContractGate YAML contract from:
//!   - A live Kafka topic (JSON, Avro, Protobuf) — requires `scaffold` feature.
//!   - A local file (`.json` / `.ndjson` / `.avsc` / `.proto`).
//!
//! Output: native CG YAML with embedded `# scaffold:` stat comments and PII
//! TODO annotations.  ODCS export is available via the existing export endpoint
//! after the contract is created.
//!
//! Developer tooling — not part of the patent-core validation engine.

pub mod merge;
pub mod pii;
pub mod profiler;
pub mod report;

#[cfg(feature = "scaffold")]
pub mod kafka;

use crate::contract::{Contract, FieldDefinition, FieldType, Ontology};
use crate::infer::infer_fields_from_objects_pub;
use crate::infer_avro::walk_avro_schema;
use crate::infer_proto::{build_fields_for_message, parse_proto_source};
use anyhow::{bail, Context, Result};
use pii::PiiCandidate;
use profiler::{FieldStats, Profiler};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public configuration
// ---------------------------------------------------------------------------

/// Configuration for a single scaffold run.
pub struct ScaffoldConfig {
    /// Contract name (used in the `name:` field of the emitted YAML).
    pub name: String,
    /// Optional description embedded in the contract.
    pub description: Option<String>,
    /// PII confidence threshold — fields scoring above this get a TODO.
    /// Default: 0.4 (RFC-024 §D).
    pub pii_threshold: f32,
    /// Maximum records to sample (for the from-file JSON path, this caps the
    /// number of objects processed by the profiler).
    pub max_records: usize,
    /// Wall-clock limit in seconds (Kafka path only).
    pub wall_clock_secs: u64,
    /// Skip value profiling (types + field names only).
    pub fast: bool,
}

impl Default for ScaffoldConfig {
    fn default() -> Self {
        Self {
            name: "unnamed".to_string(),
            description: None,
            pii_threshold: 0.4,
            max_records: 1_000,
            wall_clock_secs: 30,
            fast: false,
        }
    }
}

/// Detected input format.
#[derive(Debug, Clone, PartialEq)]
pub enum InputFormat {
    Json,
    NdJson,
    AvroSchema,
    Proto,
}

impl InputFormat {
    pub fn display(&self) -> &'static str {
        match self {
            InputFormat::Json => "JSON",
            InputFormat::NdJson => "NDJSON",
            InputFormat::AvroSchema => "Avro Schema (.avsc)",
            InputFormat::Proto => "Protocol Buffers (.proto)",
        }
    }
}

/// Full result of a scaffold run.
#[derive(Debug)]
pub struct ScaffoldResult {
    pub contract_yaml: String,
    pub pii_candidates: Vec<PiiCandidate>,
    pub field_stats: Vec<FieldStats>,
    pub sample_count: usize,
    pub format: InputFormat,
    pub sr_unavailable: bool,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Scaffold a contract from a local file.
///
/// Supported extensions: `.json`, `.ndjson`, `.avsc`, `.proto`.
pub fn scaffold_from_file(path: &Path, config: &ScaffoldConfig) -> Result<ScaffoldResult> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let raw =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;

    match ext.as_str() {
        "avsc" => scaffold_avro_schema(&raw, config),
        "proto" => scaffold_proto(&raw, config),
        "ndjson" => scaffold_ndjson(&raw, config),
        "json" => scaffold_json(&raw, config),
        other => {
            bail!("unsupported file extension: .{other}  (supported: .json .ndjson .avsc .proto)")
        }
    }
}

/// Scaffold a contract from a live Kafka topic.
///
/// Only available when compiled with `--features scaffold`.
#[cfg(feature = "scaffold")]
pub fn scaffold_from_topic(
    topic: &str,
    kafka_config: &kafka::KafkaConfig,
    config: &ScaffoldConfig,
) -> Result<ScaffoldResult> {
    use kafka::WireFormat;

    eprintln!(
        "Sampling topic '{topic}' … (max {} records, {}s wall-clock)",
        config.max_records, config.wall_clock_secs
    );

    let sample = kafka::sample_topic(
        topic,
        kafka_config,
        config.max_records,
        config.wall_clock_secs,
    )
    .context("Kafka sampling failed")?;

    if sample.sr_unavailable {
        eprintln!(
            "WARNING: Schema Registry unreachable — Avro/Protobuf payloads decoded as best-effort JSON.\n\
             Quality will be low.  Obtain SR access and re-run, or use --require-sr to abort."
        );
    }

    let (entities, format) = match &sample.detected_format {
        WireFormat::AvroWithSr => {
            // When SR was available, walk_avro_schema was used during decode.
            // Fall back to JSON inference on the decoded objects.
            (
                infer_fields_from_objects_pub(&sample.records),
                InputFormat::Json, // best we can do without schema round-trip here
            )
        }
        _ => (
            infer_fields_from_objects_pub(&sample.records),
            InputFormat::Json,
        ),
    };

    let (field_stats, pii_candidates) = if config.fast {
        (vec![], vec![])
    } else {
        let stats = run_profiler(&sample.records, config);
        let field_names: Vec<String> = entities.iter().map(|f| f.name.clone()).collect();
        let pii = pii::detect_pii(&field_names, &sample.records, config.pii_threshold);
        (stats, pii)
    };

    let pii_map = build_pii_map(&pii_candidates);
    let stats_map = build_stats_map(&field_stats);

    let contract = build_contract(entities, config);
    let yaml = emit_scaffold_yaml(
        &contract,
        &stats_map,
        &pii_map,
        &ScaffoldMeta {
            source: format!("kafka:{topic}"),
            sample_count: sample.records.len(),
            sr_unavailable: sample.sr_unavailable,
        },
    );

    Ok(ScaffoldResult {
        contract_yaml: yaml,
        pii_candidates,
        field_stats,
        sample_count: sample.records.len(),
        format,
        sr_unavailable: sample.sr_unavailable,
    })
}

// ---------------------------------------------------------------------------
// Format-specific scaffolders
// ---------------------------------------------------------------------------

fn scaffold_json(raw: &str, config: &ScaffoldConfig) -> Result<ScaffoldResult> {
    // Accept either a JSON array or a single JSON object.
    let samples: Vec<Value> =
        match serde_json::from_str::<Value>(raw).context("JSON parse failed")? {
            Value::Array(arr) => arr,
            obj @ Value::Object(_) => vec![obj],
            _ => bail!("JSON input must be an object or array of objects"),
        };
    scaffold_from_samples(samples, InputFormat::Json, config, "file")
}

fn scaffold_ndjson(raw: &str, config: &ScaffoldConfig) -> Result<ScaffoldResult> {
    let mut samples: Vec<Value> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .with_context(|| format!("NDJSON parse error on line {}", i + 1))?;
        samples.push(v);
        if samples.len() >= config.max_records {
            break;
        }
    }
    scaffold_from_samples(samples, InputFormat::NdJson, config, "file")
}

fn scaffold_avro_schema(raw: &str, config: &ScaffoldConfig) -> Result<ScaffoldResult> {
    let schema: Value =
        serde_json::from_str(raw).context("Avro schema (.avsc) is not valid JSON")?;
    let entities = walk_avro_schema(&schema).map_err(|e| anyhow::anyhow!("{e}"))?;

    let pii_candidates = if !config.fast {
        let field_names: Vec<String> = entities.iter().map(|f| f.name.clone()).collect();
        // No sample values available for Avro schema path — Signal 1 only.
        pii::detect_pii(&field_names, &[], config.pii_threshold)
    } else {
        vec![]
    };

    let pii_map = build_pii_map(&pii_candidates);
    let contract = build_contract(entities, config);
    let yaml = emit_scaffold_yaml(
        &contract,
        &HashMap::new(),
        &pii_map,
        &ScaffoldMeta {
            source: "avro-schema-file".to_string(),
            sample_count: 0,
            sr_unavailable: false,
        },
    );

    Ok(ScaffoldResult {
        contract_yaml: yaml,
        pii_candidates,
        field_stats: vec![],
        sample_count: 0,
        format: InputFormat::AvroSchema,
        sr_unavailable: false,
    })
}

fn scaffold_proto(raw: &str, config: &ScaffoldConfig) -> Result<ScaffoldResult> {
    let parsed = parse_proto_source(raw).map_err(|e| anyhow::anyhow!("proto parse: {e}"))?;

    let message_name = parsed
        .messages
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no message found in .proto source"))?;

    let entities = build_fields_for_message(&message_name, &parsed)
        .map_err(|e| anyhow::anyhow!("proto field build: {e}"))?;

    let pii_candidates = if !config.fast {
        let field_names: Vec<String> = entities.iter().map(|f| f.name.clone()).collect();
        pii::detect_pii(&field_names, &[], config.pii_threshold)
    } else {
        vec![]
    };

    let pii_map = build_pii_map(&pii_candidates);
    let contract = build_contract(entities, config);
    let yaml = emit_scaffold_yaml(
        &contract,
        &HashMap::new(),
        &pii_map,
        &ScaffoldMeta {
            source: format!("proto-file (message: {message_name})"),
            sample_count: 0,
            sr_unavailable: false,
        },
    );

    Ok(ScaffoldResult {
        contract_yaml: yaml,
        pii_candidates,
        field_stats: vec![],
        sample_count: 0,
        format: InputFormat::Proto,
        sr_unavailable: false,
    })
}

fn scaffold_from_samples(
    mut samples: Vec<Value>,
    format: InputFormat,
    config: &ScaffoldConfig,
    source: &str,
) -> Result<ScaffoldResult> {
    samples.truncate(config.max_records);
    let sample_count = samples.len();

    let entities = infer_fields_from_objects_pub(&samples);

    let (field_stats, pii_candidates) = if config.fast || samples.is_empty() {
        (vec![], vec![])
    } else {
        let stats = run_profiler(&samples, config);
        let field_names: Vec<String> = entities.iter().map(|f| f.name.clone()).collect();
        let pii = pii::detect_pii(&field_names, &samples, config.pii_threshold);
        (stats, pii)
    };

    let pii_map = build_pii_map(&pii_candidates);
    let stats_map = build_stats_map(&field_stats);

    let contract = build_contract(entities, config);
    let yaml = emit_scaffold_yaml(
        &contract,
        &stats_map,
        &pii_map,
        &ScaffoldMeta {
            source: source.to_string(),
            sample_count,
            sr_unavailable: false,
        },
    );

    Ok(ScaffoldResult {
        contract_yaml: yaml,
        pii_candidates,
        field_stats,
        sample_count,
        format,
        sr_unavailable: false,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_profiler(samples: &[Value], _config: &ScaffoldConfig) -> Vec<FieldStats> {
    let mut profiler = Profiler::with_default_budget();
    for s in samples {
        profiler.record_event(s);
    }
    if profiler.over_budget() {
        eprintln!("WARNING: profiler exceeded memory budget — some stats are approximate");
    }
    profiler.finalise()
}

fn build_contract(entities: Vec<FieldDefinition>, config: &ScaffoldConfig) -> Contract {
    Contract {
        version: "1.0".to_string(),
        name: config.name.clone(),
        description: config.description.clone(),
        compliance_mode: false,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
    }
}

fn build_pii_map(candidates: &[PiiCandidate]) -> HashMap<String, &PiiCandidate> {
    candidates
        .iter()
        .map(|c| (c.field_name.clone(), c))
        .collect()
}

fn build_stats_map(stats: &[FieldStats]) -> HashMap<String, &FieldStats> {
    stats.iter().map(|s| (s.name.clone(), s)).collect()
}

// ---------------------------------------------------------------------------
// YAML emitter with embedded scaffold comments
// ---------------------------------------------------------------------------

struct ScaffoldMeta {
    source: String,
    sample_count: usize,
    sr_unavailable: bool,
}

/// Emit scaffold YAML with embedded `# scaffold:` stat comments and PII TODOs.
///
/// We build the YAML as a String rather than relying on serde_yaml comment
/// injection because serde_yaml 0.9 does not support YAML comments.
fn emit_scaffold_yaml(
    contract: &Contract,
    stats: &HashMap<String, &FieldStats>,
    pii: &HashMap<String, &PiiCandidate>,
    meta: &ScaffoldMeta,
) -> String {
    let mut out = String::with_capacity(2048);

    // Header comments.
    out.push_str("# Generated by cg scaffold\n");
    out.push_str(&format!("# Source: {}\n", meta.source));
    if meta.sample_count > 0 {
        out.push_str(&format!("# Samples: {}\n", meta.sample_count));
    }
    if meta.sr_unavailable {
        out.push_str(
            "# WARNING: Schema Registry was unavailable; \
                      schema-driven inference skipped (sr-unavailable)\n",
        );
    }
    out.push('\n');

    // Top-level fields.
    out.push_str("version: \"1.0\"\n");
    out.push_str(&format!("name: {}\n", yaml_quote(&contract.name)));
    if let Some(ref desc) = contract.description {
        out.push_str(&format!("description: {}\n", yaml_quote(desc)));
    }

    // Ontology.
    out.push_str("ontology:\n");
    out.push_str("  entities:\n");

    for field in &contract.ontology.entities {
        emit_field(&mut out, field, stats, pii, 2);
    }

    // Glossary (empty by default; user fills in).
    out.push_str("glossary: []\n");

    // Metrics (empty by default).
    out.push_str("metrics: []\n");

    out
}

/// Emit a single FieldDefinition, with optional stats + PII comments.
/// `indent` is the number of leading spaces for the `- name:` line.
fn emit_field(
    out: &mut String,
    field: &FieldDefinition,
    stats: &HashMap<String, &FieldStats>,
    pii: &HashMap<String, &PiiCandidate>,
    indent: usize,
) {
    let pad = " ".repeat(indent);
    let ipad = " ".repeat(indent + 2); // inner padding

    // Stats comment (before the field definition).
    if let Some(s) = stats.get(&field.name) {
        let mut parts = Vec::new();
        parts.push(format!("null_rate={:.2}", s.null_rate()));
        parts.push(format!("distinct=~{}", s.distinct_estimate));
        if let (Some(mn), Some(mx)) = (s.numeric_min, s.numeric_max) {
            parts.push(format!("range=[{mn},{mx}]"));
        }
        if let Some(p50) = s.length_p50 {
            parts.push(format!("len_p50={p50}"));
        }
        if !s.top_k.is_empty() && !s.top_k_saturated {
            let preview: Vec<String> = s
                .top_k
                .iter()
                .take(3)
                .map(|(v, _)| format!("\"{v}\""))
                .collect();
            parts.push(format!("top_k=[{}]", preview.join(",")));
        }
        out.push_str(&format!("{pad}# scaffold: {}\n", parts.join(" ")));
    }

    // PII comment + TODO (before the field definition).
    if let Some(c) = pii.get(&field.name) {
        out.push_str(&format!(
            "{pad}# scaffold: pii-candidate confidence={:.2} reason=\"{}\"\n",
            c.confidence, c.reason
        ));
        out.push_str(&format!("{pad}# TODO: review PII — consider adding:\n"));
        out.push_str(&format!(
            "{pad}#   transform:\n{pad}#     kind: {}\n",
            c.suggested_transform
        ));
    }

    // Field definition.
    out.push_str(&format!("{pad}- name: {}\n", yaml_quote(&field.name)));
    out.push_str(&format!(
        "{ipad}type: {}\n",
        field_type_str(&field.field_type)
    ));
    out.push_str(&format!("{ipad}required: {}\n", field.required));

    if let Some(ref pat) = field.pattern {
        out.push_str(&format!("{ipad}pattern: {}\n", yaml_quote(pat)));
    }
    if let Some(ref av) = field.allowed_values {
        let items: Vec<String> = av.iter().map(|v| yaml_scalar(v)).collect();
        out.push_str(&format!("{ipad}enum: [{}]\n", items.join(", ")));
    }
    if let Some(mn) = field.min {
        out.push_str(&format!("{ipad}min: {mn}\n"));
    }
    if let Some(mx) = field.max {
        out.push_str(&format!("{ipad}max: {mx}\n"));
    }
    if let Some(mn) = field.min_length {
        out.push_str(&format!("{ipad}min_length: {mn}\n"));
    }
    if let Some(mx) = field.max_length {
        out.push_str(&format!("{ipad}max_length: {mx}\n"));
    }

    // Nested object properties.
    if let Some(ref props) = field.properties {
        out.push_str(&format!("{ipad}properties:\n"));
        for child in props {
            emit_field(out, child, stats, pii, indent + 4);
        }
    }

    // Array items.
    if let Some(ref items) = field.items {
        out.push_str(&format!("{ipad}items:\n"));
        out.push_str(&format!(
            "{ipad}  type: {}\n",
            field_type_str(&items.field_type)
        ));
    }
}

fn field_type_str(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::String => "string",
        FieldType::Integer => "integer",
        FieldType::Float => "float",
        FieldType::Boolean => "boolean",
        FieldType::Object => "object",
        FieldType::Array => "array",
        FieldType::Any => "any",
    }
}

/// Minimal YAML scalar quoting: wrap in double-quotes if the value contains
/// special characters or looks like a YAML keyword.
fn yaml_quote(s: &str) -> String {
    const YAML_KEYWORDS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
    let needs_quotes = s.is_empty()
        || YAML_KEYWORDS.contains(&s.to_lowercase().as_str())
        || s.contains(':')
        || s.contains('#')
        || s.contains('"')
        || s.contains('\'')
        || s.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.');

    if needs_quotes {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn yaml_scalar(v: &Value) -> String {
    match v {
        Value::String(s) => yaml_quote(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => yaml_quote(&v.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(ext: &str, content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn scaffold_json_array() {
        let json = r#"[
            {"user_id": "u1", "amount": 10.5, "active": true},
            {"user_id": "u2", "amount": 20.0, "active": false}
        ]"#;
        let f = write_temp("json", json);
        let cfg = ScaffoldConfig {
            name: "test_contract".to_string(),
            ..Default::default()
        };
        let result = scaffold_from_file(f.path(), &cfg).unwrap();
        assert!(result.contract_yaml.contains("name: test_contract"));
        assert!(result.contract_yaml.contains("user_id"));
        assert!(result.contract_yaml.contains("amount"));
        assert_eq!(result.sample_count, 2);
    }

    #[test]
    fn scaffold_ndjson() {
        let ndjson = "{\"x\": 1}\n{\"x\": 2}\n{\"x\": null}\n";
        let f = write_temp("ndjson", ndjson);
        let cfg = ScaffoldConfig {
            name: "ndjson_test".to_string(),
            ..Default::default()
        };
        let result = scaffold_from_file(f.path(), &cfg).unwrap();
        assert!(result.contract_yaml.contains("name: ndjson_test"));
        assert_eq!(result.sample_count, 3);
    }

    #[test]
    fn scaffold_avro_schema() {
        let avsc = r#"{
            "type": "record",
            "name": "Order",
            "fields": [
                {"name": "order_id", "type": "string"},
                {"name": "amount", "type": "double"},
                {"name": "email", "type": ["null", "string"]}
            ]
        }"#;
        let f = write_temp("avsc", avsc);
        let cfg = ScaffoldConfig {
            name: "orders".to_string(),
            ..Default::default()
        };
        let result = scaffold_from_file(f.path(), &cfg).unwrap();
        assert!(result.contract_yaml.contains("order_id"));
        assert!(result.contract_yaml.contains("amount"));
        // email is a PII candidate
        assert!(!result.pii_candidates.is_empty());
        assert_eq!(result.format, InputFormat::AvroSchema);
    }

    #[test]
    fn scaffold_proto() {
        let proto = r#"
            syntax = "proto3";
            message UserEvent {
                string user_id = 1;
                string email = 2;
                int64 timestamp = 3;
                double amount = 4;
            }
        "#;
        let f = write_temp("proto", proto);
        let cfg = ScaffoldConfig {
            name: "user_events".to_string(),
            ..Default::default()
        };
        let result = scaffold_from_file(f.path(), &cfg).unwrap();
        assert!(result.contract_yaml.contains("user_id"));
        assert!(result.contract_yaml.contains("email"));
        assert_eq!(result.format, InputFormat::Proto);
    }

    #[test]
    fn pii_todo_comment_never_live_yaml() {
        let json = r#"[{"email": "alice@example.com"}, {"email": "bob@example.com"}]"#;
        let f = write_temp("json", json);
        let cfg = ScaffoldConfig {
            name: "pii_test".to_string(),
            pii_threshold: 0.1,
            ..Default::default()
        };
        let result = scaffold_from_file(f.path(), &cfg).unwrap();
        // The transform block must appear only as a comment, never as live YAML.
        let yaml = &result.contract_yaml;
        assert!(yaml.contains("# TODO"), "should have PII TODO comment");
        // Make sure we can parse the emitted YAML as a valid Contract.
        let yaml_no_comments: String = yaml
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed: Contract = serde_yaml::from_str(&yaml_no_comments)
            .expect("scaffold YAML (comments stripped) must be valid Contract YAML");
        // No transform block in parsed contract.
        for entity in &parsed.ontology.entities {
            assert!(
                entity.transform.is_none(),
                "transform must not be auto-applied: field {}",
                entity.name
            );
        }
    }

    #[test]
    fn yaml_quote_handles_special_cases() {
        assert_eq!(yaml_quote("hello"), "hello");
        assert_eq!(yaml_quote(""), "\"\"");
        assert_eq!(yaml_quote("true"), "\"true\"");
        assert_eq!(yaml_quote("null"), "\"null\"");
        assert_eq!(yaml_quote("has:colon"), "\"has:colon\"");
        assert_eq!(yaml_quote("123"), "\"123\"");
    }

    #[test]
    fn unknown_extension_returns_error() {
        let f = write_temp("csv", "a,b,c\n1,2,3\n");
        let cfg = ScaffoldConfig::default();
        let result = scaffold_from_file(f.path(), &cfg);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported"));
    }
}
