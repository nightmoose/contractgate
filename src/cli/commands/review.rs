//! `contractgate review <file.csv>`: infer a draft contract, validate every
//! row, and write contract.yaml, report.html, and rejected.csv.

use crate::review;
use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct ReviewArgs {
    /// Path to the CSV file to review.
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Directory for contract.yaml, report.html, and rejected.csv.
    /// Defaults to `<file-stem>-review` next to the CSV.
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,

    /// Name for the inferred contract. Defaults to the CSV file stem.
    #[arg(long)]
    pub name: Option<String>,
}

/// Exit codes: 0 = wrote outputs (even if some rows were rejected),
/// 2 = CSV or compile error.
pub fn run(args: &ReviewArgs) -> Result<i32> {
    let csv = fs::read_to_string(&args.file)
        .with_context(|| format!("reading {}", args.file.display()))?;
    let name = args
        .name
        .clone()
        .or_else(|| {
            args.file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "review".to_string());
    let out = args.out.clone().unwrap_or_else(|| {
        let stem = args
            .file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "review".to_string());
        let parent = args
            .file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        parent.join(format!("{stem}-review"))
    });

    let result = match review::review_csv(&csv, &name, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };
    if let Err(e) = review::write_review_outputs(&out, &result) {
        eprintln!("error: {e}");
        return Ok(2);
    }

    println!(
        "checked {} row(s): {} passed, {} rejected",
        result.rows_total, result.rows_passed, result.rows_rejected
    );
    if !result.pii.is_empty() {
        let names: Vec<&str> = result.pii.iter().map(|p| p.field_name.as_str()).collect();
        println!("PII flagged (masked in the report): {}", names.join(", "));
    }
    println!("wrote {}/contract.yaml", out.display());
    println!("wrote {}/report.html", out.display());
    println!("wrote {}/rejected.csv", out.display());
    Ok(0)
}
