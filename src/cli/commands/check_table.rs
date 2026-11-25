//! `contractgate check-table`: run a contract's rules as SQL against a
//! warehouse table and report violations with plain-English reasons
//! (RFC-119).

use crate::batch_check;
use crate::sql_dialect::SqlDialect;
use crate::validation::CompiledContract;
use anyhow::{Context, Result};
use clap::Args;
use std::{fs, path::PathBuf, str::FromStr};

#[derive(Debug, Args)]
pub struct CheckTableArgs {
    /// Path to the contract YAML file.
    #[arg(value_name = "CONTRACT")]
    pub contract: PathBuf,

    /// Table to check, used verbatim in the generated SQL. Schema-qualify
    /// and quote it yourself if your warehouse needs that
    /// (`analytics.orders`, `"ORDERS"."PUBLIC"."ORDERS"`, ...).
    #[arg(long)]
    pub table: String,

    /// SQL dialect: postgres, duckdb, snowflake, or databricks.
    #[arg(long)]
    pub dialect: String,

    /// Column(s) that identify a row, comma-separated. Required: the
    /// violation report is keyed by these.
    #[arg(long, value_delimiter = ',')]
    pub key: Vec<String>,

    /// Print the generated SQL and exit. Default when --execute is not given.
    #[arg(long)]
    pub print_sql: bool,

    /// Run the query against a live database. Only `--dialect postgres` is
    /// supported for execution in this build (see docs/batch-snowflake-
    /// databricks-reference.md for why duckdb/snowflake/databricks are
    /// generation-only). Other dialects: drop this flag and run the printed
    /// SQL through your own client.
    #[arg(long)]
    pub execute: bool,

    /// Postgres connection string. Defaults to $DATABASE_URL.
    #[arg(long)]
    pub conn: Option<String>,

    /// Write violating rows (key columns + rule + reason) as CSV to this
    /// file. Requires --execute.
    #[arg(long)]
    pub quarantine_out: Option<PathBuf>,

    /// Emit JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Exit codes: 0 = ran clean (no violations, or printed SQL), 1 = violations
/// found, 2 = contract/arg error, 3 = database error.
pub fn run(args: &CheckTableArgs) -> Result<i32> {
    let dialect = match SqlDialect::from_str(&args.dialect) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    if args.execute && !dialect.supports_execute() {
        eprintln!(
            "error: --execute is not supported for --dialect {dialect} in this build. \
             Drop --execute and run the printed SQL through your own client \
             (SnowSQL, Databricks SQL warehouse, dbt test)."
        );
        return Ok(2);
    }

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

    let check_sql = match batch_check::generate_check_sql(
        &compiled.contract,
        dialect,
        &args.table,
        &args.key,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    for s in &check_sql.skipped {
        eprintln!("note: skipped {}: {}", s.field, s.reason);
    }

    if args.quarantine_out.is_some() && !args.execute {
        eprintln!("error: --quarantine-out needs --execute");
        return Ok(2);
    }

    if !args.execute || args.print_sql {
        println!("{}", check_sql.sql);
        return Ok(0);
    }

    let conn = match args
        .conn
        .clone()
        .or_else(|| std::env::var("DATABASE_URL").ok())
    {
        Some(c) => c,
        None => {
            eprintln!("error: --execute needs --conn or $DATABASE_URL");
            return Ok(2);
        }
    };

    let columns: Vec<String> = args
        .key
        .iter()
        .cloned()
        .chain(["rule".to_string(), "reason".to_string()])
        .collect();

    let rows = match execute_postgres(&check_sql.sql, &conn) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return Ok(3);
        }
    };

    if let Some(path) = &args.quarantine_out {
        write_quarantine_csv(path, &columns, &rows)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {} row(s) to {}", rows.len(), path.display());
    }

    if args.json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::Value::Object(
                    columns
                        .iter()
                        .zip(row.iter())
                        .map(|(c, v)| {
                            (
                                c.clone(),
                                v.clone()
                                    .map(serde_json::Value::String)
                                    .unwrap_or(serde_json::Value::Null),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "violations": json_rows.len(),
                "rules_checked": check_sql.rule_count,
                "rows": json_rows,
            }))?
        );
    } else {
        println!(
            "{} violation(s) across {} rule(s) checked",
            rows.len(),
            check_sql.rule_count
        );
        for row in &rows {
            let rendered: Vec<String> = columns
                .iter()
                .zip(row.iter())
                .map(|(c, v)| format!("{c}={}", v.as_deref().unwrap_or("NULL")))
                .collect();
            println!("  {}", rendered.join(" "));
        }
    }

    Ok(if rows.is_empty() { 0 } else { 1 })
}

fn execute_postgres(sql: &str, conn: &str) -> Result<Vec<Vec<Option<String>>>> {
    use sqlx::{postgres::PgPoolOptions, Row};

    let rt = tokio::runtime::Runtime::new().context("starting a tokio runtime")?;
    rt.block_on(async {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(conn)
            .await
            .context("connecting to postgres")?;
        let rows = sqlx::query(sql)
            .fetch_all(&pool)
            .await
            .context("running the check-table query")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut values = Vec::with_capacity(row.len());
            for i in 0..row.len() {
                values.push(row.try_get::<Option<String>, usize>(i).unwrap_or(None));
            }
            out.push(values);
        }
        Ok(out)
    })
}

fn write_quarantine_csv(
    path: &PathBuf,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(columns)?;
    for row in rows {
        writer.write_record(row.iter().map(|v| v.as_deref().unwrap_or("")))?;
    }
    writer.flush()?;
    Ok(())
}
