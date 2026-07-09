//! `contractgate` CLI binary.
//!
//! Subcommands: push, pull, validate, scaffold, enforce, infer, test.
//! Auth via CONTRACTGATE_API_KEY env var or --api-key flag.
//! Config via .contractgate.yml (walk-up from cwd, stop at git root).

use clap::{Parser, Subcommand};
use contractgate::cli::{
    commands::{deploy, enforce, infer, pull, push, scaffold, test, validate},
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
    /// Atomically deploy a contract YAML directly to stable (RFC-028).
    ///
    /// Finds-or-creates the contract by name, inserts the version as stable,
    /// and deprecates all prior stable versions.  Rejected if pending
    /// quarantine events exist.  Admin / service-role key required.
    ///
    /// Examples:
    ///   cg deploy-contract contracts/orders.yaml --source yardi --deployed-by ci-job-42
    ///   cg deploy-contract contracts/events.yaml --dry-run
    #[command(name = "deploy-contract")]
    DeployContract(deploy::DeployArgs),
    /// Walk contracts dir, parse YAML, push to gateway.
    Push(push::PushArgs),
    /// Pull contracts from gateway and write as YAML files.
    Pull(pull::PullArgs),
    /// Parse + compile each contract YAML locally. No network.
    Validate(validate::ValidateArgs),
    /// Derive a draft contract from a Kafka topic or local file (RFC-024).
    ///
    /// Examples:
    ///   cg scaffold orders --broker kafka:9092 --output contracts/orders.yaml
    ///   cg scaffold --from-file samples.json --name user_events
    ///   cg scaffold --from-file schema.avsc --name orders
    ///   cg scaffold --from-file events.proto --output contracts/events.yaml
    Scaffold(scaffold::ScaffoldArgs),
    /// Shadow-enforce a contract against live Kafka traffic (RFC-024).
    ///
    /// Examples:
    ///   cg enforce --mode shadow --contract contracts/orders.yaml --topic orders
    ///   cg enforce --mode shadow --contract my.yaml --topic events --report json
    Enforce(enforce::EnforceArgs),
    /// Infer a ContractGate contract from a JSON response (RFC-046).
    ///
    /// Two input modes — all processing is local, no network calls:
    ///
    ///   --from-stdin   Pipe raw curl/httpie output directly.
    ///   --from-newman  Read a Newman JSON reporter export file.
    ///
    /// Examples:
    ///   curl "https://api.example.com/users/1" \
    ///     | cg infer --from-stdin --name users --out contracts/users.yaml
    ///   curl -H "Authorization: Bearer $TOKEN" "https://api.census.gov/..." \
    ///     | cg infer --from-stdin --name census_acs5
    ///   newman run collection.json --reporters json --reporter-json-export out.json
    ///   cg infer --from-newman out.json --out contracts/orders.yaml --odcs
    Infer(infer::InferArgs),
    /// Dry-run a contract against local sample data. No server, no Kafka. (RFC-076)
    ///
    /// Loads the contract YAML, runs every record through the validation engine,
    /// and prints a pass/fail summary with per-record violation detail.
    ///
    /// Accepts NDJSON, a JSON array, or a single JSON object.
    /// Use `-` as --data to read from stdin (chainable with `cg infer`).
    ///
    /// Exit codes: 0 = all pass, 1 = violations found, 2 = load/parse error.
    ///
    /// Examples:
    ///   cg test --contract contracts/events.yaml --data samples.ndjson
    ///   cg test --contract c.yaml --data events.json --format json
    ///   cg test --contract c.yaml --data '[{"user_id":"x"}]'
    ///   cg infer --from-stdin --name orders | cg test --contract orders.yaml --data -
    Test(test::TestArgs),
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

        Cmd::DeployContract(args) => {
            let key = require_api_key(&cli.api_key);
            deploy::run(args, &cfg, &key)
        }

        Cmd::Push(args) => {
            let key = require_api_key(&cli.api_key);
            push::run(args, &cfg, &key)
        }

        Cmd::Pull(args) => {
            let key = require_api_key(&cli.api_key);
            pull::run(args, &cfg, &key)
        }

        // Scaffold, enforce, infer, and test do not need gateway config or an API key.
        Cmd::Scaffold(args) => scaffold::run(args),
        Cmd::Enforce(args) => enforce::run(args),
        Cmd::Infer(args) => infer::run(args),
        Cmd::Test(args) => test::run(args).map_err(|e| {
            // Load/parse errors from test::run propagate as exit 2, not 10.
            eprintln!("error: {e:#}");
            process::exit(2);
        }),

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
