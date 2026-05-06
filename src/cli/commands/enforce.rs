//! `cg enforce --mode shadow` subcommand — run shadow enforcement against a
//! live topic without touching the hot ingest path.  RFC-024 §G.

use crate::contract::Contract;
use crate::scaffold::report::{format_json, format_markdown, push_prometheus, ViolationReport};
use crate::validation::{validate, CompiledContract, Violation};
use clap::Args;
use serde_json::Value;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Debug, clap::ValueEnum, Clone)]
pub enum EnforceMode {
    Shadow,
}

#[derive(Debug, clap::ValueEnum, Clone)]
pub enum ReportFormat {
    Markdown,
    Json,
    Prometheus,
}

#[derive(Debug, Args)]
pub struct EnforceArgs {
    /// Enforcement mode.  Currently only `shadow` is supported.
    #[arg(long, value_enum, default_value_t = EnforceMode::Shadow)]
    pub mode: EnforceMode,

    /// Contract file (path to YAML) or contract ID prefixed with `id:` (e.g. `id:uuid`).
    #[arg(long, value_name = "FILE_OR_ID", required = true)]
    pub contract: String,

    /// Kafka bootstrap server (host:port).  Env: CG_KAFKA_BROKER.
    #[arg(long, env = "CG_KAFKA_BROKER", value_name = "HOST:PORT")]
    pub broker: Option<String>,

    /// Kafka topic to consume.
    #[arg(long, value_name = "TOPIC")]
    pub topic: Option<String>,

    /// Maximum records to check.
    #[arg(long, default_value_t = 1000)]
    pub records: usize,

    /// Wall-clock limit in seconds.
    #[arg(long, default_value_t = 30)]
    pub wall_clock: u64,

    /// Report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Markdown)]
    pub report: ReportFormat,

    /// Output file path for the report.  Defaults to stdout.
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Prometheus Pushgateway URL (required for --report prometheus).
    /// Env: CG_PUSHGATEWAY_URL.
    #[arg(long, env = "CG_PUSHGATEWAY_URL")]
    pub pushgateway_url: Option<String>,

    /// Print report only; do not push to Prometheus.
    #[arg(long)]
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub fn run(args: &EnforceArgs) -> anyhow::Result<i32> {
    // Load and compile the contract.
    let (contract_name, compiled) = load_contract(&args.contract)?;

    // Collect events to validate.
    let events = collect_events(args)?;
    let total = events.len() as u64;

    if total == 0 {
        eprintln!("No events collected — nothing to validate.");
        return Ok(0);
    }

    // Run validation in shadow mode (same validate() call as hot path, zero changes there).
    let all_violations: Vec<Vec<Violation>> = events
        .iter()
        .map(|e| {
            let result = validate(&compiled, e);
            result.violations
        })
        .collect();

    let report = ViolationReport::from_violations(
        &contract_name,
        args.topic.as_deref().unwrap_or("file"),
        total,
        all_violations,
    );

    // Format and write report.
    let report_text = format_report(&report, &args.report, args)?;

    match &args.output {
        Some(path) => {
            std::fs::write(path, &report_text)
                .map_err(|e| anyhow::anyhow!("cannot write report: {e}"))?;
            eprintln!("Report written to {}", path.display());
        }
        None => print!("{report_text}"),
    }

    // Exit code: 0 = clean, 1 = violations found.
    let exit = if report.violated_events > 0 { 1 } else { 0 };
    if exit == 1 {
        eprintln!(
            "Shadow violations: {}/{} events ({:.1}%)",
            report.violated_events,
            report.total_events,
            report.violation_rate() * 100.0
        );
    }
    Ok(exit)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_contract(spec: &str) -> anyhow::Result<(String, CompiledContract)> {
    if spec.starts_with("id:") {
        anyhow::bail!(
            "contract ID lookup (id:uuid) is a Phase 2 feature; \
             pass a local YAML file path instead"
        );
    }
    let yaml =
        std::fs::read_to_string(spec).map_err(|e| anyhow::anyhow!("cannot read {spec}: {e}"))?;
    let contract: Contract =
        serde_yaml::from_str(&yaml).map_err(|e| anyhow::anyhow!("invalid contract YAML: {e}"))?;
    let name = contract.name.clone();
    let compiled = CompiledContract::compile(contract)
        .map_err(|e| anyhow::anyhow!("contract compile failed: {e}"))?;
    Ok((name, compiled))
}

fn collect_events(args: &EnforceArgs) -> anyhow::Result<Vec<Value>> {
    // Phase 2: live Kafka consume.  For MVP, only --topic with the scaffold
    // feature works; otherwise read from a local NDJSON file.
    if let Some(ref topic) = args.topic {
        #[cfg(feature = "scaffold")]
        {
            use crate::scaffold::kafka::{sample_topic, KafkaAuth, KafkaConfig};

            let broker = args.broker.clone().unwrap_or_else(|| {
                std::env::var("CG_KAFKA_BROKER").unwrap_or_else(|_| "localhost:9092".to_string())
            });
            let kafka_cfg = KafkaConfig {
                broker,
                auth: KafkaAuth::Plaintext,
                sasl_username: None,
                sasl_password: None,
                ssl_ca_location: None,
                ssl_cert_location: None,
                ssl_key_location: None,
                schema_registry_url: None,
                sr_username: None,
                sr_password: None,
            };
            let sample = sample_topic(topic, &kafka_cfg, args.records, args.wall_clock)?;
            return Ok(sample.records);
        }

        #[cfg(not(feature = "scaffold"))]
        {
            anyhow::bail!(
                "live Kafka enforce requires --features scaffold; \
                 rebuild with: cargo build --features scaffold"
            );
        }
    }

    anyhow::bail!("specify --topic to select events to validate")
}

fn format_report(
    report: &ViolationReport,
    fmt: &ReportFormat,
    args: &EnforceArgs,
) -> anyhow::Result<String> {
    match fmt {
        ReportFormat::Markdown => Ok(format_markdown(report)),
        ReportFormat::Json => Ok(format_json(report)),
        ReportFormat::Prometheus => {
            if args.dry_run {
                // Dry-run: emit the prometheus text format as stdout instead of pushing.
                return Ok(format_json(report)); // fall back to JSON for --dry-run display
            }
            let url = args.pushgateway_url.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Prometheus push requires --pushgateway-url or CG_PUSHGATEWAY_URL")
            })?;
            push_prometheus(report, url)?;
            Ok(String::new())
        }
    }
}
