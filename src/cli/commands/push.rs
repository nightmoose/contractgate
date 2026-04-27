use crate::cli::{client::GatewayClient, config::CliConfig, output};
use crate::contract::Contract;
use anyhow::Result;
use clap::Args;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct PushArgs {
    /// Directory containing contract YAML files (overrides config).
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Parse and validate locally, but do not send to the gateway.
    #[arg(long)]
    pub dry_run: bool,

    /// Emit machine-readable JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Gateway contract listing row (only fields we need for name-lookup).
#[derive(Deserialize)]
struct ContractSummary {
    id: uuid::Uuid,
    name: String,
}

/// Minimal response shapes for create / create-version.
#[derive(Deserialize)]
struct ContractIdResponse {
    id: uuid::Uuid,
}

#[derive(Deserialize)]
struct VersionIdResponse {
    // used only for checking success
    #[allow(dead_code)]
    id: uuid::Uuid,
}

/// Per-file result emitted to stdout.
#[derive(Serialize)]
struct FileResult {
    file: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Walk contracts dir, parse YAML, POST to gateway. Per-file result.
/// Exit 0 on all success, 1 on any failure.
pub fn run(args: &PushArgs, cfg: &CliConfig, api_key: &str) -> Result<i32> {
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

    // In dry-run, skip network entirely.
    if args.dry_run {
        return dry_run_files(&files, mode);
    }

    let client = GatewayClient::new(&cfg.gateway.url, api_key)?;

    // Fetch existing contracts once to check name collisions.
    let existing: Vec<ContractSummary> = client.get("/contracts").unwrap_or_default();

    let mut any_failure = false;

    for path in &files {
        match push_file(path, &client, &existing, mode) {
            Ok(()) => {}
            Err(_) => any_failure = true,
        }
    }

    Ok(if any_failure { 1 } else { 0 })
}

fn dry_run_files(files: &[PathBuf], mode: output::Mode) -> Result<i32> {
    let mut any_failure = false;
    for path in files {
        let result = parse_file(path);
        match result {
            Ok(_) => {
                let name = path.display().to_string();
                output::ok(
                    mode,
                    &format!("DRY-RUN PASS  {name}"),
                    &FileResult {
                        file: name,
                        status: "dry_run_pass",
                        contract_id: None,
                        version: None,
                        action: None,
                        error: None,
                    },
                );
            }
            Err(e) => {
                any_failure = true;
                let name = path.display().to_string();
                let msg = format!("{e:#}");
                output::err(
                    mode,
                    &format!("DRY-RUN FAIL  {name}\n              {msg}"),
                    &FileResult {
                        file: name,
                        status: "dry_run_fail",
                        contract_id: None,
                        version: None,
                        action: None,
                        error: Some(msg),
                    },
                );
            }
        }
    }
    Ok(if any_failure { 1 } else { 0 })
}

/// Parse a YAML file, returning `(yaml_content, contract_name, version)`.
fn parse_file(path: &Path) -> Result<(String, String, String)> {
    let yaml = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read file: {e}"))?;
    let contract: Contract = serde_yaml::from_str(&yaml)
        .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
    Ok((yaml, contract.name.clone(), contract.version.clone()))
}

fn push_file(
    path: &Path,
    client: &GatewayClient,
    existing: &[ContractSummary],
    mode: output::Mode,
) -> Result<()> {
    let file = path.display().to_string();

    let (yaml, name, version) = match parse_file(path) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("{e:#}");
            output::err(
                mode,
                &format!("FAIL  {file}\n      {msg}"),
                &FileResult {
                    file,
                    status: "fail",
                    contract_id: None,
                    version: None,
                    action: None,
                    error: Some(msg),
                },
            );
            return Err(e);
        }
    };

    // Check if a contract with this name already exists.
    let existing_id = existing.iter().find(|c| c.name == name).map(|c| c.id);

    let (contract_id, action): (uuid::Uuid, &'static str) = match existing_id {
        None => {
            // Create new contract (POST /contracts).
            let body = serde_json::json!({
                "name": name,
                "yaml_content": yaml,
            });
            let resp: ContractIdResponse = client.post("/contracts", &body).map_err(|e| {
                let msg = format!("{e:#}");
                output::err(
                    mode,
                    &format!("FAIL  {file}\n      {msg}"),
                    &FileResult {
                        file: file.clone(),
                        status: "fail",
                        contract_id: None,
                        version: None,
                        action: None,
                        error: Some(msg.clone()),
                    },
                );
                anyhow::anyhow!("{msg}")
            })?;
            (resp.id, "created")
        }
        Some(id) => {
            // Contract exists — push a new version (POST /contracts/:id/versions).
            let path_str = format!("/contracts/{id}/versions");
            let body = serde_json::json!({
                "version": version,
                "yaml_content": yaml,
            });
            let _resp: VersionIdResponse =
                client.post(&path_str, &body).map_err(|e| {
                    let msg = format!("{e:#}");
                    output::err(
                        mode,
                        &format!("FAIL  {file}\n      {msg}"),
                        &FileResult {
                            file: file.clone(),
                            status: "fail",
                            contract_id: Some(id.to_string()),
                            version: None,
                            action: None,
                            error: Some(msg.clone()),
                        },
                    );
                    anyhow::anyhow!("{msg}")
                })?;
            (id, "version_added")
        }
    };

    output::ok(
        mode,
        &format!("OK    {file}  ({action})"),
        &FileResult {
            file,
            status: "ok",
            contract_id: Some(contract_id.to_string()),
            version: Some(version),
            action: Some(action),
            error: None,
        },
    );

    Ok(())
}
