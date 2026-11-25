//! `contractgate synth` subcommand: synthetic rows from a contract (RFC-117).
//!
//! Runs locally on the contract YAML. No server, no network, and no real
//! data: rows come only from the contract's rules.
//!
//! Exit codes:
//!   0 - rows written
//!   1 - a contract rule cannot be satisfied (the error names the rule)
//!   2 - the contract file cannot be read, parsed or compiled

use crate::synth::{self, flatten_record, SynthMode, SynthOptions};
use anyhow::Result;
use clap::Args;
use serde_json::Value;
use std::{io::Write, path::PathBuf};

#[derive(Debug, Args)]
pub struct SynthArgs {
    /// Path to the contract YAML file.
    #[arg(value_name = "CONTRACT")]
    pub contract: PathBuf,

    /// Number of rows to generate (1 to 10000).
    #[arg(long, default_value_t = 100)]
    pub rows: usize,

    /// Generate rows that each break exactly one rule, labeled with it.
    #[arg(long)]
    pub invalid: bool,

    /// Seed for repeatable output. A random seed is chosen (and printed to
    /// stderr) when omitted.
    #[arg(long)]
    pub seed: Option<u64>,

    /// RFC 3339 time that freshness rules are measured from (default now).
    /// Set it with --seed for output that repeats exactly.
    #[arg(long, value_name = "RFC3339")]
    pub anchor: Option<String>,

    /// Wrap the rows in the contract's `envelope` (JSON only). With
    /// --invalid the output is `{"payload": ..., "labels": [...]}`.
    #[arg(long)]
    pub envelope: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,

    /// Write to this file instead of stdout.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Format {
    Json,
    Csv,
}

pub fn run(args: &SynthArgs) -> Result<i32> {
    let yaml = match std::fs::read_to_string(&args.contract) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", args.contract.display());
            return Ok(2);
        }
    };
    let compiled = match synth::compile_yaml(&yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e.message);
            return Ok(2);
        }
    };
    let now_unix = match &args.anchor {
        Some(t) => match chrono::DateTime::parse_from_rfc3339(t) {
            Ok(d) => d.timestamp(),
            Err(e) => {
                eprintln!("error: --anchor must be RFC 3339 (e.g. 2026-01-01T00:00:00Z): {e}");
                return Ok(2);
            }
        },
        None => chrono::Utc::now().timestamp(),
    };
    if args.envelope && matches!(args.format, Format::Csv) {
        eprintln!("error: --envelope needs --format json");
        return Ok(2);
    }
    let seed = args.seed.unwrap_or_else(rand::random);
    if args.seed.is_none() {
        eprintln!("seed: {seed}");
    }
    let opts = SynthOptions {
        rows: args.rows,
        mode: if args.invalid {
            SynthMode::Invalid
        } else {
            SynthMode::Valid
        },
        seed,
        now_unix,
    };
    let out = match synth::generate(&compiled, &opts) {
        Ok(o) => o,
        Err(e) => {
            let field = e
                .field
                .as_deref()
                .map(|f| format!(" on '{f}'"))
                .unwrap_or_default();
            eprintln!("error: rule '{}'{field}: {}", e.rule, e.message);
            return Ok(1);
        }
    };
    for s in &out.skipped_rules {
        eprintln!(
            "note: no row breaks the {} rule on '{}': {}",
            s.rule, s.field, s.reason
        );
    }

    let text = match args.format {
        Format::Json if args.envelope => {
            let records = if args.invalid {
                out.invalid_rows.iter().map(|r| r.record.clone()).collect()
            } else {
                out.rows.clone()
            };
            let payload = match synth::wrap_envelope(&compiled.contract, records) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {}", e.message);
                    return Ok(2);
                }
            };
            let v = if args.invalid {
                let labels: Vec<_> = out.invalid_rows.iter().map(|r| &r.broken_rule).collect();
                serde_json::json!({"payload": payload, "labels": labels})
            } else {
                payload
            };
            serde_json::to_string_pretty(&v)? + "\n"
        }
        Format::Json => {
            let v = if args.invalid {
                serde_json::to_value(&out.invalid_rows)?
            } else {
                Value::Array(out.rows)
            };
            serde_json::to_string_pretty(&v)? + "\n"
        }
        Format::Csv => to_csv(&out)?,
    };
    match &args.out {
        Some(p) => std::fs::write(p, text)?,
        None => std::io::stdout().lock().write_all(text.as_bytes())?,
    }
    Ok(0)
}

/// Nested objects become dot-path columns; arrays are JSON text. Invalid
/// rows get `_broken_field` and `_broken_rule` columns.
fn to_csv(out: &synth::SynthOutput) -> Result<String> {
    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    if out.mode == SynthMode::Invalid {
        for r in &out.invalid_rows {
            let mut cells = Vec::new();
            flatten_record(&r.record, "", &mut cells);
            cells.push(("_broken_field".into(), r.broken_rule.field.clone()));
            cells.push(("_broken_rule".into(), r.broken_rule.rule.clone()));
            rows.push(cells);
        }
    } else {
        for r in &out.rows {
            let mut cells = Vec::new();
            flatten_record(r, "", &mut cells);
            rows.push(cells);
        }
    }
    let mut header: Vec<String> = Vec::new();
    for cells in &rows {
        for (k, _) in cells {
            if !header.contains(k) {
                header.push(k.clone());
            }
        }
    }
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(&header)?;
    for cells in &rows {
        let rec: Vec<&str> = header
            .iter()
            .map(|h| {
                cells
                    .iter()
                    .find(|(k, _)| k == h)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("")
            })
            .collect();
        w.write_record(&rec)?;
    }
    Ok(String::from_utf8(w.into_inner()?)?)
}
