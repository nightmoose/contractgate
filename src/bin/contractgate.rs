//! `contractgate` CLI binary.
//!
//! Subcommands: push, pull, diff, validate, scaffold, enforce, infer, test, synth, review, export, import, check-table.
//! Auth via CONTRACTGATE_API_KEY env var or --api-key flag.
//! Config via .contractgate.yml (walk-up from cwd, stop at git root).

use clap::{Parser, Subcommand};
use contractgate::cli::{
    commands::{
        check_table, deploy, diff, enforce, export, import, infer, pull, push, review, scaffold,
        synth, test, validate,
    },
    config::CliConfig,
};
use std::{path::PathBuf, process};

#[derive(Parser)]
#[command(
    name = "contractgate",
    about = "DataContractGate CLI: push, pull, and validate semantic contracts",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    /// API key for the DataContractGate gateway.
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
    ///   contractgate deploy-contract contracts/orders.yaml --source yardi --deployed-by ci-job-42
    ///   contractgate deploy-contract contracts/events.yaml --dry-run
    #[command(name = "deploy-contract")]
    DeployContract(deploy::DeployArgs),
    /// Walk contracts dir, parse YAML, push to gateway.
    Push(push::PushArgs),
    /// Pull contracts from gateway and write as YAML files.
    Pull(pull::PullArgs),
    /// Compare local contract YAML with the deployed stable version. Changes nothing.
    ///
    /// Exit codes: 0 = no drift, 1 = drift (a file differs from the deployed
    /// version or is not deployed), 2 = the check could not run.
    ///
    /// Examples:
    ///   contractgate diff
    ///   contractgate diff --name orders --json
    Diff(diff::DiffArgs),
    /// Parse + compile each contract YAML locally. No network.
    Validate(validate::ValidateArgs),
    /// Derive a draft contract from a Kafka topic or local file (RFC-024).
    ///
    /// Examples:
    ///   contractgate scaffold orders --broker kafka:9092 --output contracts/orders.yaml
    ///   contractgate scaffold --from-file samples.json --name user_events
    ///   contractgate scaffold --from-file schema.avsc --name orders
    ///   contractgate scaffold --from-file events.proto --output contracts/events.yaml
    Scaffold(scaffold::ScaffoldArgs),
    /// Shadow-enforce a contract against live Kafka traffic (RFC-024).
    ///
    /// Examples:
    ///   contractgate enforce --mode shadow --contract contracts/orders.yaml --topic orders
    ///   contractgate enforce --mode shadow --contract my.yaml --topic events --report json
    Enforce(enforce::EnforceArgs),
    /// Infer a DataContractGate contract from a JSON response (RFC-046).
    ///
    /// Two input modes — all processing is local, no network calls:
    ///
    ///   --from-stdin   Pipe raw curl/httpie output directly.
    ///   --from-newman  Read a Newman JSON reporter export file.
    ///
    /// Examples:
    ///   curl "https://api.example.com/users/1" \
    ///     | contractgate infer --from-stdin --name users --out contracts/users.yaml
    ///   curl -H "Authorization: Bearer $TOKEN" "https://api.census.gov/..." \
    ///     | contractgate infer --from-stdin --name census_acs5
    ///   newman run collection.json --reporters json --reporter-json-export out.json
    ///   contractgate infer --from-newman out.json --out contracts/orders.yaml --odcs
    Infer(infer::InferArgs),
    /// Dry-run a contract against local sample data. No server, no Kafka. (RFC-076)
    ///
    /// Loads the contract YAML, runs every record through the validation engine,
    /// and prints a pass/fail summary with per-record violation detail.
    ///
    /// Accepts NDJSON, a JSON array, or a single JSON object.
    /// Use `-` as --data to read from stdin (chainable with `contractgate infer`).
    ///
    /// Exit codes: 0 = all pass, 1 = violations found, 2 = load/parse error.
    ///
    /// Examples:
    ///   contractgate test --contract contracts/events.yaml --data samples.ndjson
    ///   contractgate test --contract c.yaml --data events.json --format json
    ///   contractgate test --contract c.yaml --data '[{"user_id":"x"}]'
    ///   contractgate infer --from-stdin --name orders | contractgate test --contract orders.yaml --data -
    Test(test::TestArgs),
    /// Generate synthetic rows from a contract's rules. Local, no network. (RFC-117)
    ///
    /// Rows come only from the contract: types, required, enums, ranges,
    /// lengths, patterns, formats and quality rules. No real data is read.
    /// `--invalid` makes each row break exactly one rule, labeled with it.
    ///
    /// Exit codes: 0 = rows written, 1 = a rule cannot be satisfied, 2 = load error.
    ///
    /// Examples:
    ///   contractgate synth contracts/orders.yaml --rows 500 --seed 42
    ///   contractgate synth contracts/orders.yaml --rows 50 --invalid --format csv -o bad.csv
    ///   contractgate synth datacontract.yaml --rows 100 --seed 1 --synth-config synth.yaml
    Synth(synth::SynthArgs),
    /// Infer a draft contract from a CSV, validate every row, and write a
    /// Readiness Review report (contract.yaml, report.html, rejected.csv).
    ///
    /// Examples:
    ///   contractgate review data.csv
    ///   contractgate review data.csv --out ./review-out --name orders
    Review(review::ReviewArgs),
    /// Export a contract to another format. (RFC-119, RFC-120)
    Export(export::ExportArgs),
    /// Import a contract from another format. (RFC-120)
    Import(import::ImportArgs),
    /// Run a contract's rules as SQL against a warehouse table and report
    /// violations with plain-English reasons. (RFC-119)
    ///
    /// Examples:
    ///   contractgate check-table contracts/orders.yaml --table orders --dialect snowflake --key order_id
    ///   contractgate check-table contracts/orders.yaml --table orders --dialect postgres \
    ///     --key order_id --execute --conn postgres://localhost/mydb
    #[command(name = "check-table")]
    CheckTable(check_table::CheckTableArgs),
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

        Cmd::Diff(args) => {
            let key = require_api_key(&cli.api_key);
            diff::run(args, &cfg, &key)
        }

        // Scaffold, enforce, infer, synth, and test do not need gateway config or an API key.
        Cmd::Scaffold(args) => scaffold::run(args),
        Cmd::Enforce(args) => enforce::run(args),
        Cmd::Infer(args) => infer::run(args),
        Cmd::Synth(args) => synth::run(args),
        Cmd::Review(args) => review::run(args),
        Cmd::Test(args) => test::run(args).map_err(|e| {
            // Load/parse errors from test::run propagate as exit 2, not 10.
            eprintln!("error: {e:#}");
            process::exit(2);
        }),
        Cmd::Export(args) => match &args.command {
            export::ExportCmd::DbtTests(a) => export::run(a),
            export::ExportCmd::Dbt(a) => export::run_dbt(a),
        },
        Cmd::Import(args) => match &args.command {
            import::ImportCmd::Dbt(a) => import::run_dbt(a),
        },
        Cmd::CheckTable(args) => check_table::run(args),

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
  "title": "DataContractGate CLI Config",
  "description": "Schema for .contractgate.yml",
  "type": "object",
  "properties": {
    "version": { "type": "string", "default": "1.0" },
    "gateway": {
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "Base URL of the DataContractGate gateway" }
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
