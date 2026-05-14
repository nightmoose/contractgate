//! `deploy-contract` subcommand — RFC-028.
//!
//! Reads a contract YAML file, parses it locally for fast feedback, then
//! POSTs to `POST /contracts/deploy` on the gateway.  The gateway atomically:
//!   1. Finds or creates the contract identity by name.
//!   2. Rejects if pending quarantine events exist.
//!   3. Inserts the version as `stable` with `parsed_json` + deploy metadata.
//!   4. Deprecates all previously-stable versions for this contract.
//!
//! Admin-only: requires a service-role API key (or legacy env-var key with
//! full access).

use crate::cli::{client::GatewayClient, config::CliConfig, output};
use crate::contract::Contract;
use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct DeployArgs {
    /// Contract YAML file to deploy.
    pub file: PathBuf,

    /// PMS vendor or logical feed name (e.g. yardi, realpage, entrata).
    #[arg(long)]
    pub source: Option<String>,

    /// CI job ID or username to record as the deployer.
    #[arg(long, env = "CONTRACTGATE_DEPLOYED_BY")]
    pub deployed_by: Option<String>,

    /// Parse and validate locally, but do not send to the gateway.
    #[arg(long)]
    pub dry_run: bool,

    /// Emit machine-readable JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Minimal gateway response shape.
#[derive(Deserialize)]
struct DeployResponse {
    contract_id: String,
    version_id: String,
    name: String,
    version: String,
    deprecated_count: i64,
}

#[derive(Serialize)]
struct DeployResult {
    file: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deprecated_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn run(args: &DeployArgs, cfg: &CliConfig, api_key: &str) -> Result<i32> {
    let mode = if args.json {
        output::Mode::Json
    } else {
        output::Mode::Human
    };
    let file = args.file.display().to_string();

    // ── Parse locally first ───────────────────────────────────────────────────
    let yaml = match std::fs::read_to_string(&args.file) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("cannot read file: {e}");
            output::err(
                mode,
                &format!("FAIL  {file}\n      {msg}"),
                &DeployResult {
                    file: file.clone(),
                    status: "fail",
                    contract_id: None,
                    version_id: None,
                    name: None,
                    version: None,
                    deprecated_count: None,
                    error: Some(msg),
                },
            );
            return Ok(1);
        }
    };

    let contract: Contract = match serde_yaml::from_str(&yaml) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("YAML parse error: {e}");
            output::err(
                mode,
                &format!("FAIL  {file}\n      {msg}"),
                &DeployResult {
                    file: file.clone(),
                    status: "fail",
                    contract_id: None,
                    version_id: None,
                    name: None,
                    version: None,
                    deprecated_count: None,
                    error: Some(msg),
                },
            );
            return Ok(1);
        }
    };

    if args.dry_run {
        output::ok(
            mode,
            &format!(
                "DRY-RUN PASS  {file}  ({}@{})",
                contract.name, contract.version
            ),
            &DeployResult {
                file,
                status: "dry_run_pass",
                contract_id: None,
                version_id: None,
                name: Some(contract.name),
                version: Some(contract.version),
                deprecated_count: None,
                error: None,
            },
        );
        return Ok(0);
    }

    // ── POST to gateway ───────────────────────────────────────────────────────
    let client = GatewayClient::new(&cfg.gateway.url, api_key)?;
    let body = serde_json::json!({
        "name": contract.name,
        "yaml_content": yaml,
        "source": args.source,
        "deployed_by": args.deployed_by,
    });

    let resp: DeployResponse = match client.post("/contracts/deploy", &body) {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("{e:#}");
            output::err(
                mode,
                &format!("FAIL  {file}\n      {msg}"),
                &DeployResult {
                    file: file.clone(),
                    status: "fail",
                    contract_id: None,
                    version_id: None,
                    name: None,
                    version: None,
                    deprecated_count: None,
                    error: Some(msg),
                },
            );
            return Ok(1);
        }
    };

    let human_msg = format!(
        "DEPLOYED  {file}  ({name}@{version}, deprecated {n} prior)",
        name = resp.name,
        version = resp.version,
        n = resp.deprecated_count,
    );
    output::ok(
        mode,
        &human_msg,
        &DeployResult {
            file,
            status: "ok",
            contract_id: Some(resp.contract_id),
            version_id: Some(resp.version_id),
            name: Some(resp.name),
            version: Some(resp.version),
            deprecated_count: Some(resp.deprecated_count),
            error: None,
        },
    );

    Ok(0)
}
