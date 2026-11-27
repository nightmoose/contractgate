//! `contractgate test` subcommand — local contract dry-run against sample data.  RFC-076.
//!
//! Loads a contract YAML and runs every record in a local data file (or stdin)
//! through the existing `validate()` engine.  No server, no Kafka, no network.
//!
//! Exit codes:
//!   0 — all records pass
//!   1 — one or more records fail validation
//!   2 — usage / load error (bad contract YAML, unreadable data file,
//!         unparseable top-level JSON)

use crate::contract::{Contract, FieldDefinition, FieldType};
use crate::validation::{validate, CompiledContract, Violation};
use clap::Args;
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    io::{self, Read},
    path::PathBuf,
    time::Instant,
};

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct TestArgs {
    /// Path to contract YAML file.
    #[arg(long, value_name = "FILE", required = true)]
    pub contract: PathBuf,

    /// Path to data file (NDJSON, JSON array, single JSON object, or CSV).
    /// Use `-` to read from stdin.
    #[arg(long, value_name = "FILE", required = true)]
    pub data: String,

    /// Data file format. `auto` (default) detects CSV from a `.csv`
    /// extension and otherwise parses JSON array / object / NDJSON, same as
    /// before this flag existed.
    #[arg(long, value_enum, default_value_t = DataFormat::Auto)]
    pub data_format: DataFormat,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// Stop at the first failing record.
    #[arg(long)]
    pub fail_fast: bool,

    /// Summary line only; suppress per-violation detail.
    #[arg(long)]
    pub quiet: bool,

    /// For an ODCS file with several `schema` objects: the one to use, by
    /// name (default: the first).
    #[arg(long, value_name = "NAME")]
    pub schema_object: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Data file format for `--data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DataFormat {
    /// Detect CSV from a `.csv` extension; anything else is parsed as JSON
    /// (array, single object) or NDJSON, whichever matches.
    Auto,
    Json,
    Ndjson,
    Csv,
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RecordResult {
    record: usize,
    status: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    violations: Vec<ViolationJson>,
}

#[derive(Serialize)]
struct ViolationJson {
    field: String,
    kind: String,
    message: String,
}

#[derive(Serialize)]
struct Summary {
    contract: String,
    data_source: String,
    total: usize,
    pass: usize,
    fail: usize,
    elapsed_ms: f64,
    records: Vec<RecordResult>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(args: &TestArgs) -> anyhow::Result<i32> {
    // --- Load + compile contract (exit 2 on failure) ----------------------
    let (contract_name, compiled) = load_contract(&args.contract, args.schema_object.as_deref())
        .map_err(|e| {
            eprintln!("error: {e:#}");
            // We can't return i32 from map_err; we propagate and handle at call site.
            e
        })?;

    // --- Read records (exit 2 on top-level parse failure) -----------------
    let data_format = resolve_format(&args.data, args.data_format);
    let records = read_records(
        &args.data,
        data_format,
        &compiled.contract.ontology.entities,
    )
    .map_err(|e| {
        eprintln!("error reading data: {e:#}");
        e
    })?;

    let total = records.len();
    let data_label = args.data.clone();
    let t0 = Instant::now();

    // --- Validate each record ---------------------------------------------
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut results: Vec<RecordResult> = Vec::new();

    for (idx, rec) in records.into_iter().enumerate() {
        match rec {
            Err(parse_err) => {
                // Malformed line/record — report as a record-level error, not a hard abort.
                fail += 1;
                let rr = RecordResult {
                    record: idx,
                    status: "fail",
                    violations: vec![ViolationJson {
                        field: "(record)".to_string(),
                        kind: "parse_error".to_string(),
                        message: parse_err.to_string(),
                    }],
                };
                if !args.quiet {
                    print_record_human(idx, &rr.violations, args.format.is_json());
                }
                results.push(rr);
                if args.fail_fast {
                    break;
                }
            }
            Ok(value) => {
                let result = validate(&compiled, &value);
                if result.violations.is_empty() {
                    pass += 1;
                    results.push(RecordResult {
                        record: idx,
                        status: "pass",
                        violations: vec![],
                    });
                } else {
                    fail += 1;
                    let vjs: Vec<ViolationJson> = violation_jsons(&result.violations);
                    let rr = RecordResult {
                        record: idx,
                        status: "fail",
                        violations: vjs,
                    };
                    if !args.quiet {
                        print_record_human(idx, &rr.violations, args.format.is_json());
                    }
                    results.push(rr);
                    if args.fail_fast {
                        break;
                    }
                }
            }
        }
    }

    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // --- Emit output ------------------------------------------------------
    match args.format {
        Format::Human => {
            let total_seen = pass + fail;
            println!();
            println!("contract: {contract_name}");
            println!("data:     {data_label}  ({total} records)");
            println!();
            println!("  PASS  {pass}");
            println!("  FAIL  {fail}");
            if fail > 0 {
                println!();
                println!(
                    "{fail}/{total_seen} records failed ({:.1}%)   validated in {elapsed_ms:.1}ms",
                    fail as f64 / total_seen as f64 * 100.0,
                );
            } else {
                println!();
                println!("all {total_seen} records passed   validated in {elapsed_ms:.1}ms");
            }
        }
        Format::Json => {
            let summary = Summary {
                contract: contract_name,
                data_source: data_label,
                total,
                pass,
                fail,
                elapsed_ms,
                records: if args.quiet {
                    results.into_iter().filter(|r| r.status == "fail").collect()
                } else {
                    results
                },
            };
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
    }

    Ok(if fail > 0 { 1 } else { 0 })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Format {
    fn is_json(&self) -> bool {
        matches!(self, Format::Json)
    }
}

fn load_contract(
    path: &PathBuf,
    schema_object: Option<&str>,
) -> anyhow::Result<(String, CompiledContract)> {
    let yaml = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let (contract, _notes): (Contract, Vec<String>) =
        crate::odcs::parse_contract_yaml_with(&yaml, schema_object)
            .map_err(|e| anyhow::anyhow!("invalid contract YAML: {e}"))?;
    let name = contract.name.clone();
    let compiled = CompiledContract::compile(contract)
        .map_err(|e| anyhow::anyhow!("contract compile failed: {e}"))?;
    Ok((name, compiled))
}

/// `--data-format auto`: detect CSV from a `.csv` extension (case
/// insensitive); everything else keeps going through [`parse_records`],
/// which already auto-detects JSON array / single object / NDJSON.
fn resolve_format(source: &str, explicit: DataFormat) -> DataFormat {
    if explicit != DataFormat::Auto {
        return explicit;
    }
    if source.to_ascii_lowercase().ends_with(".csv") {
        DataFormat::Csv
    } else {
        DataFormat::Auto
    }
}

/// Read records from a file path or `-` (stdin).
/// Returns a Vec of `Result<Value, String>` — malformed lines are errors, not hard aborts.
fn read_records(
    source: &str,
    format: DataFormat,
    fields: &[FieldDefinition],
) -> anyhow::Result<Vec<Result<Value, String>>> {
    let raw = if source == "-" {
        let mut buf = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut buf)
            .map_err(|e| anyhow::anyhow!("cannot read stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(source).map_err(|e| anyhow::anyhow!("cannot read {source}: {e}"))?
    };

    match format {
        DataFormat::Csv => parse_csv_records(&raw, fields),
        DataFormat::Auto | DataFormat::Json | DataFormat::Ndjson => parse_records(&raw),
    }
}

/// Parse CSV into one JSON object per row. Column names are matched to
/// contract fields the same way `synth --format csv` named them: a literal
/// column name wins (`Amount [A]`, `Rate.x`), otherwise the name is walked
/// as a native nested / array path (`shipping.country`, `tags[0]`). Cells
/// are typed against the matching field: integer / float / boolean parse
/// to a JSON number or bool, anything else (and anything that fails to
/// parse) stays a JSON string so the validator can report a clear type
/// mismatch. An empty cell means no value — the key is left out of the
/// row entirely, the same as an optional field `synth --format csv` had
/// nothing to put in that column for. A required field left empty is
/// still reported missing, same as a JSON record that leaves the key out.
fn parse_csv_records(
    raw: &str,
    fields: &[FieldDefinition],
) -> anyhow::Result<Vec<Result<Value, String>>> {
    let mut reader = csv::ReaderBuilder::new().from_reader(raw.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| anyhow::anyhow!("cannot read CSV header: {e}"))?
        .iter()
        .map(str::to_string)
        .collect();
    let header_types: Vec<Option<FieldType>> = headers
        .iter()
        .map(|h| field_type_for_path(fields, h))
        .collect();

    let mut out = Vec::new();
    for rec in reader.records() {
        match rec {
            Ok(rec) => {
                let mut obj = Map::new();
                for (i, cell) in rec.iter().enumerate() {
                    if cell.is_empty() {
                        continue;
                    }
                    let key = headers
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("col_{i}"));
                    let ty = header_types.get(i).and_then(|t| t.clone());
                    obj.insert(key, csv_cell_value(cell, ty));
                }
                out.push(Ok(Value::Object(obj)));
            }
            Err(e) => out.push(Err(format!("CSV parse error: {e}"))),
        }
    }
    Ok(out)
}

/// The contract field type a CSV column name maps to, if any: a literal
/// name match wins, otherwise the name is walked as a dotted / bracketed
/// nested path (mirrors `resolve_indexed`'s rule for data, applied to the
/// field tree instead).
fn field_type_for_path(fields: &[FieldDefinition], path: &str) -> Option<FieldType> {
    if let Some(f) = fields.iter().find(|f| f.name == path) {
        return Some(f.field_type.clone());
    }
    for f in fields {
        if path.len() <= f.name.len() || !path.starts_with(f.name.as_str()) {
            continue;
        }
        let mut rest = &path[f.name.len()..];
        if let Some(stripped) = rest.strip_prefix('.') {
            let Some(props) = f.properties.as_deref() else {
                continue;
            };
            if let Some(ty) = field_type_for_path(props, stripped) {
                return Some(ty);
            }
        } else if rest.starts_with('[') {
            let Some(item) = f.items.as_deref() else {
                continue;
            };
            while rest.starts_with('[') {
                let Some(end) = rest.find(']') else { break };
                rest = &rest[end + 1..];
            }
            let rest = rest.strip_prefix('.').unwrap_or(rest);
            if rest.is_empty() {
                return Some(item.field_type.clone());
            }
            let Some(props) = item.properties.as_deref() else {
                continue;
            };
            if let Some(ty) = field_type_for_path(props, rest) {
                return Some(ty);
            }
        }
    }
    None
}

/// Type one non-empty CSV cell against the field it maps to.
fn csv_cell_value(cell: &str, field_type: Option<FieldType>) -> Value {
    match field_type {
        Some(FieldType::Integer) => cell
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(cell.to_string())),
        Some(FieldType::Float) => {
            serde_json::Number::from_f64(cell.parse::<f64>().unwrap_or(f64::NAN))
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(cell.to_string()))
        }
        Some(FieldType::Boolean) => match cell.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(cell.to_string()),
        },
        _ => Value::String(cell.to_string()),
    }
}

fn parse_records(raw: &str) -> anyhow::Result<Vec<Result<Value, String>>> {
    let trimmed = raw.trim_start();

    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    // Try to parse as a single JSON value first (array or object).
    // This handles both JSON arrays and single objects without ambiguity.
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(items)) => {
            // JSON array — each element is a record.
            let results = items.into_iter().map(Ok).collect();
            return Ok(results);
        }
        Ok(obj @ Value::Object(_)) => {
            // Single JSON object.
            return Ok(vec![Ok(obj)]);
        }
        Ok(other) => {
            // Top-level non-object/array (number, string, null, bool) — exit-2-class error.
            anyhow::bail!("data must be a JSON object, array, or NDJSON; got: {other}");
        }
        Err(_) => {
            // Not valid as a single JSON value — try NDJSON (one object per line).
        }
    }

    // NDJSON path: parse line by line, skip blank lines.
    let results = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .map_err(|e| format!("JSON parse error: {e} (input: {line:.80})"))
        })
        .collect();

    Ok(results)
}

fn violation_jsons(violations: &[Violation]) -> Vec<ViolationJson> {
    violations
        .iter()
        .map(|v| {
            let kind = serde_json::to_string(&v.kind)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            ViolationJson {
                field: v.field.clone(),
                kind,
                message: v.message.clone(),
            }
        })
        .collect()
}

fn print_record_human(idx: usize, violations: &[ViolationJson], _is_json: bool) {
    for v in violations {
        println!(
            "  record {idx:>4}  {:<20} {:<20} {}",
            v.field, v.kind, v.message
        );
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_object() {
        let raw = r#"{"user_id": "abc", "event_type": "click"}"#;
        let records = parse_records(raw).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].is_ok());
    }

    #[test]
    fn parse_json_array() {
        let raw = r#"[{"a": 1}, {"a": 2}, {"a": 3}]"#;
        let records = parse_records(raw).unwrap();
        assert_eq!(records.len(), 3);
        for r in &records {
            assert!(r.is_ok());
        }
    }

    #[test]
    fn parse_ndjson() {
        let raw = "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n";
        let records = parse_records(raw).unwrap();
        assert_eq!(records.len(), 3);
        for r in &records {
            assert!(r.is_ok());
        }
    }

    #[test]
    fn parse_ndjson_skips_blank_lines() {
        let raw = "{\"a\":1}\n\n{\"a\":2}\n";
        let records = parse_records(raw).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn parse_ndjson_bad_line_is_record_err_not_abort() {
        let raw = "{\"a\":1}\nNOT JSON\n{\"a\":3}\n";
        let records = parse_records(raw).unwrap();
        assert_eq!(records.len(), 3);
        assert!(records[0].is_ok());
        assert!(records[1].is_err());
        assert!(records[2].is_ok());
    }

    #[test]
    fn parse_empty_input() {
        let records = parse_records("").unwrap();
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn parse_top_level_non_object_is_bail() {
        let err = parse_records("42").unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }
}
