//! `contractgate` CLI binary.
//!
//! Three subcommands: push, pull, validate.
//! Auth via CONTRACTGATE_API_KEY env var or --api-key flag.
//! Config via .contractgate.yml (walk-up from cwd, stop at git root).

use clap::{Parser, Subcommand};
use contractgate::cli::{
    commands::{pull, push, validate},
    config::CliConfig,
};
use std::{path::PathBuf, process};

#[derive(Parser)]
#[command(
    name = "contractgate",
    about = "ContractGate CLI — push, pull, and validate semantic contracts",
    version
)]
struct Cli {
    /// API key for the ContractGate gateway.
    /// Overrides CONTRACTGATE_API_KEY environment variable.
    #[arg(long, env = "CONTRACTGATE_API_KEY", global = true)]
    api_key: Option<String>,

    /// Path to .contractgate.yml config file.
    /// Overrides automatic walk-up discovery.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Walk contracts dir, parse YAML, push to gateway.
    Push(push::PushArgs),
    /// Pull contracts from gateway and write as YAML files.
    Pull(pull::PullArgs),
    /// Parse + compile each contract YAML locally. No network.
    Validate(validate::ValidateArgs),
    /// Emit the JSON Schema for .contractgate.yml.
    #[command(hide = true)]
    ConfigSchema,
}

fn main() {
    let cli = Cli::parse();

    // Load config (explicit path or walk-up).
    let cfg = match &cli.config {
        Some(p) => CliConfig::load(p),
        None => {
            CliConfig::discover(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        }
    }
    .unwrap_or_else(|e| {
        eprintln!("error loading config: {e:#}");
        process::exit(10);
    });

    let exit_code = match &cli.command {
        Cmd::Validate(args) => validate::run(args, &cfg),

        Cmd::Push(args) => {
            let key = require_api_key(&cli.api_key);
            push::run(args, &cfg, &key)
        }

        Cmd::Pull(args) => {
            let key = require_api_key(&cli.api_key);
            pull::run(args, &cfg, &key)
        }

        Cmd::ConfigSchema => {
            // Emit a minimal JSON Schema describing .contractgate.yml.
            // In future this can be generated via schemars; for now it's hand-written.
            println!("{}", CONFIG_SCHEMA_JSON);
            Ok(0)
        }
    };

    match exit_code {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            process::exit(10);
        }
    }
}

fn require_api_key(key: &Option<String>) -> String {
    key.clone().unwrap_or_else(|| {
        eprintln!("error: API key required. Set CONTRACTGATE_API_KEY or pass --api-key.");
        process::exit(11);
    })
}

const CONFIG_SCHEMA_JSON: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ContractGate CLI Config",
  "description": "Schema for .contractgate.yml",
  "type": "object",
  "properties": {
    "version": { "type": "string", "default": "1.0" },
    "gateway": {
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "Base URL of the ContractGate gateway" }
      },
      "required": ["url"]
    },
    "contracts": {
      "type": "object",
      "properties": {
        "dir":     { "type": "string", "description": "Directory containing contract YAML files" },
        "pattern": { "type": "string", "description": "Glob pattern for contract files", "default": "*.yaml" }
      }
    },
    "defaults": {
      "type": "object",
      "properties": {
        "format": { "type": "string", "enum": ["human", "json"], "default": "human" }
      }
    }
  }
}"#;
