use crate::cli::{config::CliConfig, output};
use crate::contract::Contract;
use crate::validation::CompiledContract;
use anyhow::Result;
use clap::Args;
use glob::glob;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Directory containing contract YAML files (overrides config).
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Emit machine-readable JSON output.
    #[arg(long)]
    pub json: bool,

    /// For an ODCS file with several `schema` objects: the one to use, by
    /// name (default: the first).
    #[arg(long, value_name = "NAME")]
    pub schema_object: Option<String>,
}

#[derive(Serialize)]
struct FileResult {
    file: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// ODCS parts that did not map (extra schema objects, ...).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

/// Parse + compile each contract YAML locally. No network. Per-file pass/fail.
/// Exit 0 on all pass, 1 on any failure.
pub fn run(args: &ValidateArgs, cfg: &CliConfig) -> Result<i32> {
    let mode = if args.json {
        output::Mode::Json
    } else {
        output::Mode::Human
    };

    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| cfg.contracts.dir.clone());

    let pattern = format!("{}/{}", dir.display(), cfg.contracts.pattern);
    let files: Vec<PathBuf> = glob(&pattern)
        .map(|paths| paths.filter_map(|p| p.ok()).collect())
        .unwrap_or_default();

    if files.is_empty() {
        let msg = format!("no contract files matched: {pattern}");
        output::err(mode, &msg, &serde_json::json!({"error": msg}));
        return Ok(1);
    }

    let mut any_failure = false;

    for path in &files {
        let result = validate_file(path, args.schema_object.as_deref());
        match result {
            Ok(notes) => {
                let name = path.display().to_string();
                let mut text = format!("PASS  {name}");
                for n in &notes {
                    text.push_str(&format!("\n      note: {n}"));
                }
                output::ok(
                    mode,
                    &text,
                    &FileResult {
                        file: name,
                        status: "pass",
                        error: None,
                        notes,
                    },
                );
            }
            Err(e) => {
                any_failure = true;
                let name = path.display().to_string();
                let msg = format!("{e:#}");
                output::err(
                    mode,
                    &format!("FAIL  {name}\n      {msg}"),
                    &FileResult {
                        file: name,
                        status: "fail",
                        error: Some(msg),
                        notes: Vec::new(),
                    },
                );
            }
        }
    }

    Ok(if any_failure { 1 } else { 0 })
}

/// Native or ODCS v3.x YAML. Returns notes for anything in an ODCS document
/// that did not map.
fn validate_file(path: &Path, schema_object: Option<&str>) -> Result<Vec<String>> {
    let src =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("cannot read file: {e}"))?;
    let (contract, notes): (Contract, Vec<String>) =
        crate::odcs::parse_contract_yaml_with(&src, schema_object)
            .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
    CompiledContract::compile(contract).map_err(|e| anyhow::anyhow!("compile error: {e}"))?;
    Ok(notes)
}
