use crate::cli::{client::GatewayClient, config::CliConfig, output};
use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct PullArgs {
    /// Pull only this contract (by name or UUID).
    #[arg(long)]
    pub name: Option<String>,

    /// Output directory for downloaded YAML files.
    #[arg(long)]
    pub out: Option<PathBuf>,

    /// Emit machine-readable JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Minimal shapes for deserialization (gateway response fields we care about).
#[derive(Deserialize, Clone)]
struct ContractSummary {
    id: uuid::Uuid,
    name: String,
    latest_stable_version: Option<String>,
}

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
    yaml_content: String,
}

#[derive(Serialize)]
struct FileResult {
    file: String,
    contract: String,
    version: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// GET /contracts (list) or GET /contracts/:id (one). Write each as
/// <name>.yaml under out dir. Idempotent.
/// Exit 0 on all success, 1 on any failure.
pub fn run(args: &PullArgs, cfg: &CliConfig, api_key: &str) -> Result<i32> {
    let mode = if args.json {
        output::Mode::Json
    } else {
        output::Mode::Human
    };

    let out_dir = args.out.clone().unwrap_or_else(|| cfg.contracts.dir.clone());
    std::fs::create_dir_all(&out_dir).ok();

    let client = GatewayClient::new(&cfg.gateway.url, api_key)?;

    // Collect target contracts.
    let targets: Vec<ContractSummary> = match &args.name {
        Some(name) => {
            let all: Vec<ContractSummary> = client.get("/contracts").unwrap_or_default();
            let found: Vec<_> = all
                .into_iter()
                .filter(|c| c.name == *name || c.id.to_string() == *name)
                .collect();
            if found.is_empty() {
                let msg = format!("no contract found with name or id: {name}");
                output::err(mode, &msg, &serde_json::json!({"error": msg}));
                return Ok(1);
            }
            found
        }
        None => client.get("/contracts").unwrap_or_default(),
    };

    if targets.is_empty() {
        let msg = "no contracts found on gateway";
        output::err(mode, msg, &serde_json::json!({"error": msg}));
        return Ok(1);
    }

    let mut any_failure = false;

    for summary in &targets {
        match pull_contract(&client, summary, &out_dir, mode) {
            Ok(()) => {}
            Err(_) => any_failure = true,
        }
    }

    Ok(if any_failure { 1 } else { 0 })
}

fn pull_contract(
    client: &GatewayClient,
    summary: &ContractSummary,
    out_dir: &std::path::Path,
    mode: output::Mode,
) -> Result<()> {
    let path_str = format!("/contracts/{}/versions/latest-stable", summary.id);
    let v: VersionResponse = client.get(&path_str).map_err(|e| {
        let msg = format!("{e:#}");
        output::err(
            mode,
            &format!("FAIL  {}  {msg}", summary.name),
            &FileResult {
                file: String::new(),
                contract: summary.name.clone(),
                version: String::new(),
                status: "fail",
                error: Some(msg.clone()),
            },
        );
        anyhow::anyhow!("{msg}")
    })?;

    let filename = format!("{}.yaml", summary.name);
    let dest = out_dir.join(&filename);
    std::fs::write(&dest, &v.yaml_content).map_err(|e| {
        let msg = format!("cannot write {}: {e}", dest.display());
        output::err(
            mode,
            &format!("FAIL  {filename}  {msg}"),
            &FileResult {
                file: filename.clone(),
                contract: summary.name.clone(),
                version: v.version.clone(),
                status: "fail",
                error: Some(msg.clone()),
            },
        );
        anyhow::anyhow!("{msg}")
    })?;

    let file = dest.display().to_string();
    output::ok(
        mode,
        &format!("OK    {file}  ({}@{})", summary.name, v.version),
        &FileResult {
            file,
            contract: summary.name.clone(),
            version: v.version,
            status: "ok",
            error: None,
        },
    );

    Ok(())
}
