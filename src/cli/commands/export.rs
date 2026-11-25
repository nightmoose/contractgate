//! `contractgate export` subcommand family (RFC-119).

use crate::dbt_export;
use crate::sql_dialect::SqlDialect;
use crate::validation::CompiledContract;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::{fs, path::PathBuf, str::FromStr};

#[derive(Debug, Args)]
pub struct ExportArgs {
    #[command(subcommand)]
    pub command: ExportCmd,
}

#[derive(Debug, Subcommand)]
pub enum ExportCmd {
    /// Generate dbt tests (schema.yml + singular SQL tests) from a contract.
    ///
    /// Examples:
    ///   cg export dbt-tests contracts/orders.yaml --model orders --out dbt/orders
    ///   cg export dbt-tests contracts/orders.yaml --dialect snowflake
    DbtTests(DbtTestsArgs),
    /// Export a contract as a dbt model contract (`schema.yml`).
    ///
    /// Examples:
    ///   cg export dbt contracts/orders.yaml --model orders --out models/schema.yml
    Dbt(DbtArgs),
}

#[derive(Debug, Args)]
pub struct DbtTestsArgs {
    /// Path to the contract YAML file.
    #[arg(value_name = "CONTRACT")]
    pub contract: PathBuf,

    /// dbt model name the generated tests reference (`{{ ref(model) }}`).
    /// Defaults to the contract's `name`.
    #[arg(long)]
    pub model: Option<String>,

    /// SQL dialect for singular test regex/format predicates: postgres,
    /// duckdb, snowflake, or databricks. Range/length/cross-field tests
    /// are dialect-free.
    #[arg(long, default_value = "postgres")]
    pub dialect: String,

    /// Output directory. Writes `<out>/schema.yml` and `<out>/tests/*.sql`.
    #[arg(long, short = 'o', default_value = ".")]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct DbtArgs {
    /// Path to the contract YAML file.
    #[arg(value_name = "CONTRACT")]
    pub contract: PathBuf,

    /// dbt model name. Defaults to the contract's `name`.
    #[arg(long)]
    pub model: Option<String>,

    /// Output path for schema.yml. Use `-` for stdout. Defaults to `schema.yml`.
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,
}

/// Exit codes: 0 = written, 2 = contract load/compile error.
pub fn run(args: &DbtTestsArgs) -> Result<i32> {
    let yaml = fs::read_to_string(&args.contract)
        .with_context(|| format!("reading {}", args.contract.display()))?;
    let contract: crate::contract::Contract = match serde_yaml::from_str(&yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: parsing {}: {e}", args.contract.display());
            return Ok(2);
        }
    };
    let compiled = match CompiledContract::compile(contract) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            return Ok(2);
        }
    };
    let dialect = match SqlDialect::from_str(&args.dialect) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| compiled.contract.name.clone());

    let export = dbt_export::generate_dbt_tests(&compiled.contract, &model, dialect);

    fs::create_dir_all(&args.out).with_context(|| format!("creating {}", args.out.display()))?;
    let schema_path = args.out.join("schema.yml");
    fs::write(&schema_path, &export.schema_yml)
        .with_context(|| format!("writing {}", schema_path.display()))?;

    let tests_dir = args.out.join("tests");
    fs::create_dir_all(&tests_dir).with_context(|| format!("creating {}", tests_dir.display()))?;
    for t in &export.singular_tests {
        let path = tests_dir.join(&t.filename);
        fs::write(&path, &t.sql).with_context(|| format!("writing {}", path.display()))?;
    }

    println!("wrote {}", schema_path.display());
    println!(
        "wrote {} singular test(s) to {}",
        export.singular_tests.len(),
        tests_dir.display()
    );
    for s in &export.skipped {
        eprintln!("note: skipped {}: {}", s.field, s.reason);
    }
    Ok(0)
}

/// Exit codes: 0 = written, 2 = contract load error.
pub fn run_dbt(args: &DbtArgs) -> Result<i32> {
    let yaml = fs::read_to_string(&args.contract)
        .with_context(|| format!("reading {}", args.contract.display()))?;
    let contract: crate::contract::Contract = match serde_yaml::from_str(&yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: parsing {}: {e}", args.contract.display());
            return Ok(2);
        }
    };
    let (schema, warnings) = match crate::dbt_yaml::export_dbt(&contract, args.model.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };
    for w in &warnings {
        eprintln!("note: {w}");
    }
    match &args.out {
        Some(p) if p.as_os_str() == "-" => {
            print!("{schema}");
        }
        Some(p) => {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
            }
            fs::write(p, &schema).with_context(|| format!("writing {}", p.display()))?;
            println!("wrote {}", p.display());
        }
        None => {
            let p = PathBuf::from("schema.yml");
            fs::write(&p, &schema).with_context(|| format!("writing {}", p.display()))?;
            println!("wrote {}", p.display());
        }
    }
    Ok(0)
}
