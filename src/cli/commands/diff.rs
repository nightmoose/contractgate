//! `contractgate diff` subcommand: local contract YAML versus what is deployed.
//!
//! Compares each contract file in the contracts directory with the latest
//! stable version on the gateway (the same version `pull` writes). It makes
//! no changes anywhere. Use it as a drift check in CI.
//!
//! Exit codes:
//!   0 - no drift: every local file matches the deployed stable version
//!   1 - drift: a file differs from the deployed version, or is not deployed
//!       (a deployed contract with no local file counts only with
//!       `--fail-on-remote-only`)
//!   2 - error: a file cannot be read or parsed, the gateway cannot be
//!       reached, nothing matched, or `--name` matched no local file
//!
//! A missing API key still exits 11 and a bad config file 10, as for every
//! other command that talks to the gateway.

use crate::cli::{client::GatewayClient, config::CliConfig, output};
use crate::contract::Contract;
use crate::contract_diff::{diff_contracts, DiffChange};
use anyhow::Result;
use clap::Args;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Exit code: no drift.
pub const EXIT_NO_DRIFT: i32 = 0;
/// Exit code: drift found.
pub const EXIT_DRIFT: i32 = 1;
/// Exit code: the check itself could not be completed.
pub const EXIT_ERROR: i32 = 2;

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// Directory containing contract YAML files (overrides config).
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Check only the contract with this name.
    #[arg(long)]
    pub name: Option<String>,

    /// Also treat a deployed contract with no local file as drift.
    #[arg(long)]
    pub fail_on_remote_only: bool,

    /// Emit machine-readable JSON output (one object on stdout).
    #[arg(long)]
    pub json: bool,
}

#[derive(Deserialize)]
struct ContractSummary {
    id: uuid::Uuid,
    name: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
    yaml_content: String,
}

/// What the gateway holds for one contract name.
#[derive(Debug, Clone, PartialEq)]
pub enum Deployed {
    /// No contract with this name.
    NoContract,
    /// The contract exists but has no stable version.
    NoStableVersion,
    /// The latest stable version.
    Stable { version: String, yaml: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Local file matches the deployed stable version.
    InSync,
    /// Local file differs from the deployed stable version.
    Drifted,
    /// Local file has no deployed counterpart (no contract, or no stable version).
    NotDeployed,
    /// Deployed contract with no local file.
    RemoteOnly,
    /// The file could not be read or parsed.
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractResult {
    pub contract: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    /// Field-level changes going from deployed to local.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<DiffChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub drift: bool,
    pub exit_code: i32,
    pub results: Vec<ContractResult>,
}

/// Line endings and trailing whitespace do not count as drift.
pub fn normalize(yaml: &str) -> String {
    let lines: Vec<&str> = yaml.lines().map(|l| l.trim_end()).collect();
    lines.join("\n").trim_end_matches('\n').to_string()
}

/// Compare one local file (already parsed) with what is deployed.
pub fn classify(
    file: &str,
    local_yaml: &str,
    local: &Contract,
    deployed: &Deployed,
) -> ContractResult {
    let mut r = ContractResult {
        contract: local.name.clone(),
        status: Status::InSync,
        file: Some(file.to_string()),
        deployed_version: None,
        local_version: Some(local.version.clone()),
        changes: Vec::new(),
        detail: None,
    };
    match deployed {
        Deployed::NoContract => {
            r.status = Status::NotDeployed;
            r.detail = Some("no contract with this name is deployed".into());
        }
        Deployed::NoStableVersion => {
            r.status = Status::NotDeployed;
            r.detail = Some("contract exists but has no stable version".into());
        }
        Deployed::Stable { version, yaml } => {
            r.deployed_version = Some(version.clone());
            if normalize(yaml) != normalize(local_yaml) {
                r.status = Status::Drifted;
                if let Ok(remote) = crate::odcs::parse_contract_yaml(yaml) {
                    r.changes = diff_contracts(&remote, local);
                }
                r.detail = Some(if r.changes.is_empty() {
                    "text differs; no field-level change (comments, metadata or rules)".into()
                } else {
                    format!("{} field-level change(s)", r.changes.len())
                });
            }
        }
    }
    r
}

/// Gateway lookup for one contract name.
fn fetch_deployed(
    client: &GatewayClient,
    listing: &[ContractSummary],
    name: &str,
) -> Result<Deployed> {
    let Some(summary) = listing.iter().find(|c| c.name == name) else {
        return Ok(Deployed::NoContract);
    };
    let path = format!("/contracts/{}/versions/latest-stable", summary.id);
    match client.get::<VersionResponse>(&path) {
        Ok(v) => Ok(Deployed::Stable {
            version: v.version,
            yaml: v.yaml_content,
        }),
        Err(e) => {
            let msg = format!("{e:#}");
            // The gateway answers 409 "has no stable version yet" for a
            // contract that only has drafts, and 404 for an unknown id.
            if msg.contains("no stable version") {
                Ok(Deployed::NoStableVersion)
            } else if msg.contains("not found (404)") {
                Ok(Deployed::NoContract)
            } else {
                Err(e)
            }
        }
    }
}

fn read_local(path: &Path) -> Result<(String, Contract)> {
    let yaml =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("cannot read file: {e}"))?;
    let contract = crate::odcs::parse_contract_yaml(&yaml)
        .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
    Ok((yaml, contract))
}

pub fn exit_code_for(results: &[ContractResult], fail_on_remote_only: bool) -> i32 {
    if results.iter().any(|r| r.status == Status::Error) {
        return EXIT_ERROR;
    }
    let drift = results.iter().any(|r| match r.status {
        Status::Drifted | Status::NotDeployed => true,
        Status::RemoteOnly => fail_on_remote_only,
        _ => false,
    });
    if drift {
        EXIT_DRIFT
    } else {
        EXIT_NO_DRIFT
    }
}

pub fn run(args: &DiffArgs, cfg: &CliConfig, api_key: &str) -> Result<i32> {
    let mode = if args.json {
        output::Mode::Json
    } else {
        output::Mode::Human
    };
    let fail = |msg: String| -> Result<i32> {
        output::err(
            mode,
            &format!("error: {msg}"),
            &serde_json::json!({"error": msg}),
        );
        Ok(EXIT_ERROR)
    };

    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| cfg.contracts.dir.clone());
    let pattern = format!("{}/{}", dir.display(), cfg.contracts.pattern);
    let mut files: Vec<PathBuf> = glob(&pattern)
        .map(|paths| paths.filter_map(|p| p.ok()).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return fail(format!("no contract files matched: {pattern}"));
    }

    let client = GatewayClient::new(&cfg.gateway.url, api_key)?;
    let listing: Vec<ContractSummary> = match client.get("/contracts") {
        Ok(l) => l,
        Err(e) => return fail(format!("cannot list contracts on the gateway: {e:#}")),
    };

    let mut results: Vec<ContractResult> = Vec::new();
    let mut local_names: Vec<String> = Vec::new();
    for path in &files {
        let file = path.display().to_string();
        let (yaml, contract) = match read_local(path) {
            Ok(t) => t,
            Err(e) => {
                if args.name.is_none() {
                    results.push(ContractResult {
                        contract: file.clone(),
                        status: Status::Error,
                        file: Some(file),
                        deployed_version: None,
                        local_version: None,
                        changes: Vec::new(),
                        detail: Some(format!("{e:#}")),
                    });
                }
                continue;
            }
        };
        local_names.push(contract.name.clone());
        if let Some(n) = &args.name {
            if &contract.name != n {
                continue;
            }
        }
        let deployed = match fetch_deployed(&client, &listing, &contract.name) {
            Ok(d) => d,
            Err(e) => {
                return fail(format!(
                    "cannot fetch {} from the gateway: {e:#}",
                    contract.name
                ))
            }
        };
        results.push(classify(&file, &yaml, &contract, &deployed));
    }

    if let Some(n) = &args.name {
        if !local_names.iter().any(|x| x == n) {
            return fail(format!(
                "no local contract named '{n}' in {}",
                dir.display()
            ));
        }
    } else {
        for c in &listing {
            if !local_names.iter().any(|n| n == &c.name) {
                results.push(ContractResult {
                    contract: c.name.clone(),
                    status: Status::RemoteOnly,
                    file: None,
                    deployed_version: None,
                    local_version: None,
                    changes: Vec::new(),
                    detail: Some("deployed, but no local file".into()),
                });
            }
        }
    }

    let code = exit_code_for(&results, args.fail_on_remote_only);
    let report = Report {
        drift: code == EXIT_DRIFT,
        exit_code: code,
        results,
    };
    print_report(&report, mode);
    Ok(code)
}

fn print_report(report: &Report, mode: output::Mode) {
    if mode.is_json() {
        println!("{}", serde_json::to_string(report).unwrap_or_default());
        return;
    }
    for r in &report.results {
        let tag = match r.status {
            Status::InSync => "OK     ",
            Status::Drifted => "DRIFT  ",
            Status::NotDeployed => "NEW    ",
            Status::RemoteOnly => "REMOTE ",
            Status::Error => "ERROR  ",
        };
        let version = match (&r.deployed_version, &r.local_version) {
            (Some(d), Some(l)) if d != l && r.status != Status::InSync => {
                format!(" (deployed {d}, local {l})")
            }
            (Some(d), _) => format!(" ({d})"),
            (None, Some(l)) => format!(" (local {l})"),
            _ => String::new(),
        };
        let detail = r
            .detail
            .as_deref()
            .map(|d| format!("  {d}"))
            .unwrap_or_default();
        println!("{tag}{}{version}{detail}", r.contract);
        for c in &r.changes {
            println!("         {}  {}  {}", c.kind, c.field, c.detail);
        }
    }
    let count = |f: fn(Status) -> bool| report.results.iter().filter(|r| f(r.status)).count();
    let drifted = count(|s| matches!(s, Status::Drifted | Status::NotDeployed));
    let remote_only = count(|s| s == Status::RemoteOnly);
    let errors = count(|s| s == Status::Error);
    let in_sync = count(|s| s == Status::InSync);
    println!(
        "{in_sync} in sync, {drifted} drifted or not deployed, {remote_only} deployed only, {errors} error(s) (exit {})",
        report.exit_code
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(yaml: &str) -> Contract {
        crate::odcs::parse_contract_yaml(yaml).unwrap()
    }

    const A: &str = "version: \"1.0\"\nname: t\nontology:\n  entities:\n    - name: id\n      type: string\n      required: true\n";

    #[test]
    fn normalize_ignores_line_endings_and_trailing_space() {
        assert_eq!(normalize("a: 1  \r\nb: 2\r\n\r\n"), normalize("a: 1\nb: 2"));
        assert_ne!(normalize("a: 1"), normalize("a: 2"));
    }

    #[test]
    fn classify_reports_field_changes() {
        let remote = A.to_string();
        let local_yaml = A.replace("required: true", "required: false");
        let r = classify(
            "t.yaml",
            &local_yaml,
            &contract(&local_yaml),
            &Deployed::Stable {
                version: "1.0".into(),
                yaml: remote,
            },
        );
        assert_eq!(r.status, Status::Drifted);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, "required_changed");
    }

    #[test]
    fn classify_in_sync() {
        let r = classify(
            "t.yaml",
            A,
            &contract(A),
            &Deployed::Stable {
                version: "1.0".into(),
                yaml: A.into(),
            },
        );
        assert_eq!(r.status, Status::InSync);
        assert!(r.changes.is_empty());
    }

    #[test]
    fn exit_codes() {
        let mk = |status| ContractResult {
            contract: "c".into(),
            status,
            file: None,
            deployed_version: None,
            local_version: None,
            changes: vec![],
            detail: None,
        };
        assert_eq!(exit_code_for(&[mk(Status::InSync)], false), 0);
        assert_eq!(exit_code_for(&[mk(Status::Drifted)], false), 1);
        assert_eq!(exit_code_for(&[mk(Status::NotDeployed)], false), 1);
        assert_eq!(exit_code_for(&[mk(Status::RemoteOnly)], false), 0);
        assert_eq!(exit_code_for(&[mk(Status::RemoteOnly)], true), 1);
        assert_eq!(
            exit_code_for(&[mk(Status::Drifted), mk(Status::Error)], false),
            2
        );
    }
}
