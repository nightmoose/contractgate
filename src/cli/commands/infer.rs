//! `contractgate infer` — RFC-046: derive a contract from Newman reporter output.
//!
//! Reads Newman's JSON reporter export, extracts response bodies from all
//! executions, runs the same field-inference logic used by `POST /contracts/infer`,
//! and writes a ContractGate YAML (and optionally an ODCS-compatible YAML).
//!
//! All processing is local — no network calls, no credentials required.
//!
//! ## Usage
//!
//! ```sh
//! newman run collection.json --reporters json --reporter-json-export response.json
//! contractgate infer --from-newman response.json --out contracts/my-api.yaml
//! contractgate infer --from-newman response.json --out contracts/my-api.yaml --odcs --odcs-version 2.2.2
//! ```

use crate::infer::infer_fields_from_objects_pub;
use anyhow::{bail, Context, Result};
use clap::Args;
use serde_json::Value;
use std::{fs, path::PathBuf};

// ---------------------------------------------------------------------------
// Supported ODCS versions
// ---------------------------------------------------------------------------

const ODCS_DEFAULT_VERSION: &str = "2.2.2";

const ODCS_KNOWN_VERSIONS: &[&str] = &["2.2.2", "2.1.0", "2.0.0"];

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct InferArgs {
    /// Path to Newman's JSON reporter export file.
    /// Produce with: newman run collection.json --reporters json --reporter-json-export output.json
    #[arg(long, value_name = "FILE")]
    pub from_newman: PathBuf,

    /// Contract name embedded in the generated YAML.
    /// Defaults to the Newman collection name if available.
    #[arg(long, short = 'n', value_name = "NAME")]
    pub name: Option<String>,

    /// Optional description embedded in the contract.
    #[arg(long, short = 'd', value_name = "TEXT")]
    pub description: Option<String>,

    /// Output file for the ContractGate YAML.
    /// Defaults to stdout.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// Also write an ODCS-compatible YAML alongside the ContractGate output.
    #[arg(long)]
    pub odcs: bool,

    /// ODCS schema version to target.
    /// Supported: 2.2.2, 2.1.0, 2.0.0.  Defaults to latest (2.2.2).
    #[arg(long, value_name = "VERSION", default_value = ODCS_DEFAULT_VERSION)]
    pub odcs_version: String,

    /// Emit machine-readable JSON summary to stderr.
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// Newman JSON reporter types (minimal — only what we need)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Debug)]
struct NewmanReport {
    collection: Option<NewmanCollection>,
    run: NewmanRun,
}

#[derive(serde::Deserialize, Debug)]
struct NewmanCollection {
    info: Option<NewmanCollectionInfo>,
}

#[derive(serde::Deserialize, Debug)]
struct NewmanCollectionInfo {
    name: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct NewmanRun {
    executions: Vec<NewmanExecution>,
}

#[derive(serde::Deserialize, Debug)]
struct NewmanExecution {
    response: Option<NewmanResponse>,
}

#[derive(serde::Deserialize, Debug)]
struct NewmanResponse {
    #[serde(default)]
    stream: Option<NewmanStream>,
    #[serde(default)]
    body: Option<String>,
}

/// Newman encodes response body as a byte-array `data` or a `stream` object.
#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
enum NewmanStream {
    Data { data: Vec<u8> },
    Other(Value),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(args: &InferArgs) -> Result<i32> {
    // 1. Validate ODCS version if --odcs is requested.
    if args.odcs && !ODCS_KNOWN_VERSIONS.contains(&args.odcs_version.as_str()) {
        bail!(
            "unsupported ODCS version {:?}; supported: {}",
            args.odcs_version,
            ODCS_KNOWN_VERSIONS.join(", ")
        );
    }

    // 2. Read + parse Newman report.
    let raw = fs::read_to_string(&args.from_newman)
        .with_context(|| format!("reading {:?}", args.from_newman))?;
    let report: NewmanReport =
        serde_json::from_str(&raw).context("parsing Newman JSON reporter output")?;

    // 3. Extract response bodies.
    let samples = extract_samples(&report);
    if samples.is_empty() {
        bail!("no JSON response bodies found in Newman report — ensure the collection has at least one successful request");
    }

    // 4. Infer contract fields.
    let name = args
        .name
        .clone()
        .or_else(|| report.collection.as_ref()?.info.as_ref()?.name.clone())
        .unwrap_or_else(|| "inferred_contract".to_string());
    let description = args
        .description
        .clone()
        .unwrap_or_else(|| format!("Contract inferred from Newman run of {name}"));

    let fields = infer_fields_from_objects_pub(&samples);
    let field_count = fields.len();

    // 5. Render ContractGate YAML.
    let yaml = render_contractgate_yaml(&name, &description, &fields);

    // 6. Write ContractGate YAML.
    match &args.out {
        Some(path) => {
            fs::write(path, &yaml)
                .with_context(|| format!("writing contract YAML to {:?}", path))?;
            eprintln!("✓ Contract written to {:?}", path);
        }
        None => {
            print!("{yaml}");
        }
    }

    // 7. Optionally write ODCS YAML.
    if args.odcs {
        let odcs_yaml = render_odcs_yaml(&name, &description, &fields, &args.odcs_version);
        let odcs_path = match &args.out {
            Some(p) => {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                let dir = p.parent().unwrap_or_else(|| std::path::Path::new("."));
                dir.join(format!("{stem}.odcs.yaml"))
            }
            None => PathBuf::from(format!("{}.odcs.yaml", name)),
        };
        fs::write(&odcs_path, &odcs_yaml)
            .with_context(|| format!("writing ODCS YAML to {:?}", odcs_path))?;
        eprintln!(
            "✓ ODCS YAML (v{}) written to {:?}",
            args.odcs_version, odcs_path
        );
    }

    // 8. Optional JSON summary to stderr.
    if args.json {
        let summary = serde_json::json!({
            "name": name,
            "field_count": field_count,
            "sample_count": samples.len(),
            "odcs": args.odcs,
            "odcs_version": args.odcs_version,
        });
        eprintln!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        eprintln!(
            "✓ Inferred {field_count} fields from {} response samples",
            samples.len()
        );
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk all executions and collect JSON-parseable response bodies.
fn extract_samples(report: &NewmanReport) -> Vec<Value> {
    let mut samples = Vec::new();
    for exec in &report.run.executions {
        let Some(resp) = &exec.response else { continue };
        // Try `stream.data` (byte array) first, then `body` (string).
        let text = if let Some(stream) = &resp.stream {
            match stream {
                NewmanStream::Data { data } => String::from_utf8_lossy(data).into_owned(),
                NewmanStream::Other(v) => v.to_string(),
            }
        } else if let Some(b) = &resp.body {
            b.clone()
        } else {
            continue;
        };

        // Parse JSON; skip non-object / non-array bodies.
        let Ok(val) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match &val {
            Value::Object(_) => samples.push(val),
            Value::Array(arr) => {
                // Flatten top-level arrays — take up to 10 items.
                for item in arr.iter().take(10) {
                    if item.is_object() {
                        samples.push(item.clone());
                    }
                }
            }
            _ => {} // scalar — skip
        }
    }
    samples
}

/// Render a ContractGate YAML contract from inferred field definitions.
fn render_contractgate_yaml(
    name: &str,
    description: &str,
    fields: &[crate::contract::FieldDefinition],
) -> String {
    use crate::contract::FieldType;

    let mut lines = vec![
        format!("version: \"1.0\""),
        format!("name: \"{name}\""),
        format!("description: \"{description}\""),
        String::new(),
        "ontology:".into(),
        "  entities:".into(),
    ];

    for f in fields {
        let type_str = match f.field_type {
            FieldType::String => "string",
            FieldType::Integer => "integer",
            FieldType::Float => "float",
            FieldType::Boolean => "boolean",
            FieldType::Object => "object",
            FieldType::Array => "array",
            FieldType::Any => "any",
            FieldType::Date => "date",
        };
        lines.push(format!("    - name: {}", f.name));
        lines.push(format!("      type: {type_str}"));
        lines.push(format!("      required: {}", f.required));
        if let Some(p) = &f.pattern {
            lines.push(format!("      pattern: \"{p}\""));
        }
        if let Some(vals) = &f.allowed_values {
            if !vals.is_empty() {
                let joined = vals
                    .iter()
                    .map(|v| {
                        if v.is_string() {
                            format!("\"{}\"", v.as_str().unwrap_or_default())
                        } else {
                            v.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("      enum: [{joined}]"));
            }
        }
        if let Some(mn) = f.min {
            lines.push(format!("      min: {mn}"));
        }
        if let Some(mx) = f.max {
            lines.push(format!("      max: {mx}"));
        }
    }

    lines.join("\n")
}

/// Render an ODCS-compatible YAML at the requested spec version.
fn render_odcs_yaml(
    name: &str,
    description: &str,
    fields: &[crate::contract::FieldDefinition],
    version: &str,
) -> String {
    use crate::contract::FieldType;

    // Build a serde_json Value tree, then dump as YAML.
    let mut field_map = serde_json::Map::new();
    for f in fields {
        let odcs_type = match f.field_type {
            FieldType::Integer => "integer",
            FieldType::Float => "number",
            FieldType::Boolean => "boolean",
            FieldType::Array => "array",
            FieldType::Object => "object",
            _ => "string",
        };
        let mut fobj = serde_json::Map::new();
        fobj.insert("type".into(), Value::String(odcs_type.into()));
        fobj.insert("required".into(), Value::Bool(f.required));
        fobj.insert(
            "description".into(),
            Value::String(format!("Field: {}", f.name)),
        );
        if let Some(p) = &f.pattern {
            fobj.insert("pattern".into(), Value::String(p.clone()));
        }
        if let Some(vals) = &f.allowed_values {
            if !vals.is_empty() {
                fobj.insert("enum".into(), Value::Array(vals.clone()));
            }
        }
        field_map.insert(f.name.clone(), Value::Object(fobj));
    }

    let doc = serde_json::json!({
        "dataContractSpecification": version,
        "id": format!("urn:contractgate:{}", name.replace(' ', "-").to_lowercase()),
        "info": {
            "title": name,
            "version": "1.0.0",
            "description": description,
        },
        "models": {
            name: {
                "description": description,
                "fields": field_map,
            }
        }
    });

    {
        // JSON is a valid YAML superset — parseable by any YAML consumer.
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }
}
