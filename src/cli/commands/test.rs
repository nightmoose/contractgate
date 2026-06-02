//! `cg test` subcommand — local contract dry-run against sample data.  RFC-076.
//!
//! Loads a contract YAML and runs every record in a local data file (or stdin)
//! through the existing `validate()` engine.  No server, no Kafka, no network.
//!
//! Exit codes:
//!   0 — all records pass
//!   1 — one or more records fail validation
//!   2 — usage / load error (bad contract YAML, unreadable data file,
//!         unparseable top-level JSON)

use crate::contract::Contract;
use crate::validation::{validate, CompiledContract, Violation};
use clap::Args;
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{self, BufRead, Read},
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

    /// Path to data file (NDJSON, JSON array, or single JSON object).
    /// Use `-` to read from stdin.
    #[arg(long, value_name = "FILE", required = true)]
    pub data: String,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// Stop at the first failing record.
    #[arg(long)]
    pub fail_fast: bool,

    /// Summary line only; suppress per-violation detail.
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Format {
    Human,
    Json,
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
    let (contract_name, compiled) = load_contract(&args.contract).map_err(|e| {
        eprintln!("error: {e:#}");
        // We can't return i32 from map_err; we propagate and handle at call site.
        e
    })?;

    // --- Read records (exit 2 on top-level parse failure) -----------------
    let records = read_records(&args.data).map_err(|e| {
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
                println!(
                    "all {total_seen} records passed   validated in {elapsed_ms:.1}ms"
                );
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

fn load_contract(path: &PathBuf) -> anyhow::Result<(String, CompiledContract)> {
    let yaml = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let contract: Contract =
        serde_yaml::from_str(&yaml).map_err(|e| anyhow::anyhow!("invalid contract YAML: {e}"))?;
    let name = contract.name.clone();
    let compiled = CompiledContract::compile(contract)
        .map_err(|e| anyhow::anyhow!("contract compile failed: {e}"))?;
    Ok((name, compiled))
}

/// Read records from a file path or `-` (stdin).
/// Returns a Vec of `Result<Value, String>` — malformed lines are errors, not hard aborts.
fn read_records(source: &str) -> anyhow::Result<Vec<Result<Value, String>>> {
    let raw = if source == "-" {
        let mut buf = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut buf)
            .map_err(|e| anyhow::anyhow!("cannot read stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(source)
            .map_err(|e| anyhow::anyhow!("cannot read {source}: {e}"))?
    };

    parse_records(&raw)
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
            let results = items
                .into_iter()
                .map(|v| Ok(v))
                .collect();
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
        println!("  record {idx:>4}  {:<20} {:<20} {}", v.field, v.kind, v.message);
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
