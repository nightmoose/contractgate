//! `contractgate import` subcommand family (RFC-120).

use crate::dbt_yaml;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::{fs, io::Write, path::PathBuf};

#[derive(Debug, Args)]
pub struct ImportArgs {
    #[command(subcommand)]
    pub command: ImportCmd,
}

#[derive(Debug, Subcommand)]
pub enum ImportCmd {
    /// Convert a dbt model contract (`schema.yml`) into a DataContractGate contract.
    ///
    /// Examples:
    ///   contractgate import dbt models/schema.yml --model orders --out contracts/orders.yaml
    ///   contractgate import dbt models/schema.yml --check-against contracts/orders.yaml
    Dbt(DbtImportArgs),
}

#[derive(Debug, Args)]
pub struct DbtImportArgs {
    /// Path to a dbt schema.yml (or other YAML file with a models: list).
    #[arg(value_name = "SCHEMA")]
    pub schema: PathBuf,

    /// Model name when the file contains more than one model.
    #[arg(long)]
    pub model: Option<String>,

    /// Path for the converted contract YAML. Use `-` for stdout. Ignored with `--check-against`.
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,

    /// Compare the dbt file to this DataContractGate contract and exit 1 on drift.
    #[arg(long, value_name = "CONTRACT")]
    pub check_against: Option<PathBuf>,
}

/// Exit codes: 0 = wrote or check passed, 1 = check drift, 2 = load/parse error.
pub fn run_dbt(args: &DbtImportArgs) -> Result<i32> {
    let yaml = fs::read_to_string(&args.schema)
        .with_context(|| format!("reading {}", args.schema.display()))?;

    if let Some(contract_path) = &args.check_against {
        let contract_yaml = match fs::read_to_string(contract_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: reading {}: {e}", contract_path.display());
                return Ok(2);
            }
        };
        let expected: crate::contract::Contract = match serde_yaml::from_str(&contract_yaml) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: parsing {}: {e}", contract_path.display());
                return Ok(2);
            }
        };
        let (drifts, warnings) =
            match dbt_yaml::check_against(&expected, &yaml, args.model.as_deref()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: {e}");
                    return Ok(2);
                }
            };
        for w in &warnings {
            eprintln!("note: {w}");
        }
        if drifts.is_empty() {
            println!(
                "ok: {} matches {}",
                args.schema.display(),
                contract_path.display()
            );
            return Ok(0);
        }
        eprintln!(
            "error: {} differs from {} ({} difference{})",
            args.schema.display(),
            contract_path.display(),
            drifts.len(),
            if drifts.len() == 1 { "" } else { "s" }
        );
        for d in &drifts {
            eprintln!("  {}: expected {}, got {}", d.path, d.expected, d.actual);
        }
        return Ok(1);
    }

    let result = match dbt_yaml::import_dbt(&yaml, args.model.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };
    for w in &result.warnings {
        eprintln!("note: {w}");
    }

    match &args.out {
        Some(p) if p.as_os_str() == "-" => {
            std::io::stdout()
                .write_all(result.yaml_content.as_bytes())
                .context("writing stdout")?;
        }
        Some(p) => {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
            }
            fs::write(p, &result.yaml_content)
                .with_context(|| format!("writing {}", p.display()))?;
            println!("wrote {}", p.display());
        }
        None => {
            let name = args
                .model
                .clone()
                .unwrap_or_else(|| result.contract.name.clone());
            let path = PathBuf::from(format!("{name}.yaml"));
            fs::write(&path, &result.yaml_content)
                .with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }
    Ok(0)
}
