//! `cg scaffold` subcommand — derive a draft contract from a Kafka topic or
//! local file.  RFC-024 §F.

use crate::scaffold::{scaffold_from_file, ScaffoldConfig};
use clap::Args;
use std::path::PathBuf;

/// Authentication method for Kafka broker.
#[cfg(feature = "scaffold")]
use crate::scaffold::kafka::KafkaAuth;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ScaffoldArgs {
    /// Kafka topic to sample.  Mutually exclusive with --from-file.
    #[arg(
        value_name = "TOPIC",
        conflicts_with = "from_file",
        help = "Kafka topic name to sample"
    )]
    pub topic: Option<String>,

    /// Derive contract from a local file (.json / .ndjson / .avsc / .proto).
    #[arg(long, value_name = "FILE", conflicts_with = "topic")]
    pub from_file: Option<PathBuf>,

    /// Kafka bootstrap server (host:port).
    /// Env: CG_KAFKA_BROKER.  Required for topic sampling.
    #[arg(long, env = "CG_KAFKA_BROKER", value_name = "HOST:PORT")]
    pub broker: Option<String>,

    /// Schema Registry URL (e.g. http://sr:8081).
    /// Env: CG_SR_URL.  Omit to disable SR-based decoding.
    #[arg(long, env = "CG_SR_URL", value_name = "URL")]
    pub schema_registry: Option<String>,

    /// Kafka authentication method.
    #[cfg(feature = "scaffold")]
    #[arg(long, value_enum, default_value_t = KafkaAuth::Plaintext)]
    pub auth: KafkaAuth,

    /// Maximum records to sample from the topic.
    #[arg(long, default_value_t = 1000, value_name = "N")]
    pub records: usize,

    /// Maximum wall-clock time for topic sampling, in seconds.
    #[arg(long, default_value_t = 30, value_name = "SECS")]
    pub wall_clock: u64,

    /// Skip value profiling (type + field names only).  Faster but less rich.
    #[arg(long)]
    pub fast: bool,

    /// Output file path.  Defaults to stdout.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Contract name embedded in the generated YAML.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Contract description embedded in the generated YAML.
    #[arg(long, value_name = "DESC")]
    pub description: Option<String>,

    /// Print diff against an existing contract; do not write any file.
    #[arg(long)]
    pub dry_run: bool,

    /// Existing contract ID (UUID) for merge mode.  Fetches current contract
    /// from the gateway API for three-way merge.  Phase 2 feature.
    #[arg(long, value_name = "UUID")]
    pub contract_id: Option<String>,

    /// SASL username (for --auth sasl-plain / sasl-scram-256 / sasl-scram-512).
    /// Env: CG_KAFKA_USERNAME.
    #[arg(long, env = "CG_KAFKA_USERNAME")]
    pub sasl_username: Option<String>,

    /// SASL password.  Env: CG_KAFKA_PASSWORD.
    #[arg(long, env = "CG_KAFKA_PASSWORD")]
    pub sasl_password: Option<String>,
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub fn run(args: &ScaffoldArgs) -> anyhow::Result<i32> {
    // --contract-id merge mode: Phase 2, not yet implemented.
    if args.contract_id.is_some() {
        eprintln!(
            "warning: --contract-id merge mode is a Phase 2 feature and not yet active; \
                   running plain re-scaffold instead"
        );
    }

    let name = args
        .name
        .clone()
        .or_else(|| args.topic.as_ref().map(|t| t.replace(['.', '-'], "_")))
        .or_else(|| {
            args.from_file
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unnamed".to_string());

    let config = ScaffoldConfig {
        name,
        description: args.description.clone(),
        pii_threshold: 0.4,
        max_records: args.records,
        wall_clock_secs: args.wall_clock,
        fast: args.fast,
    };

    // ── From-file path ────────────────────────────────────────────────────
    if let Some(ref path) = args.from_file {
        return run_from_file(path, &config, args);
    }

    // ── Live Kafka path ───────────────────────────────────────────────────
    let topic = args
        .topic
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("specify a topic name or --from-file"))?;

    run_kafka_topic(topic, &config, args)
}

fn run_from_file(
    path: &std::path::Path,
    config: &ScaffoldConfig,
    args: &ScaffoldArgs,
) -> anyhow::Result<i32> {
    let result = scaffold_from_file(path, config)?;

    eprintln!(
        "Fields: {}   PII candidates: {}   Samples: {}   Format: {}",
        count_fields(&result.contract_yaml),
        result.pii_candidates.len(),
        result.sample_count,
        result.format.display(),
    );

    if args.dry_run {
        println!("{}", result.contract_yaml);
        return Ok(0);
    }

    write_output(&result.contract_yaml, args.output.as_deref())?;

    let exit = if !result.pii_candidates.is_empty() {
        4
    } else {
        0
    };
    if exit == 4 {
        eprintln!(
            "PII candidates detected ({}).  Review # TODO annotations before promoting.",
            result.pii_candidates.len()
        );
    }
    Ok(exit)
}

#[allow(unused_variables)]
fn run_kafka_topic(
    topic: &str,
    config: &ScaffoldConfig,
    args: &ScaffoldArgs,
) -> anyhow::Result<i32> {
    #[cfg(feature = "scaffold")]
    {
        use crate::scaffold::kafka::KafkaConfig;
        use crate::scaffold::scaffold_from_topic;

        let broker = args.broker.clone().unwrap_or_else(|| {
            std::env::var("CG_KAFKA_BROKER").unwrap_or_else(|_| "localhost:9092".to_string())
        });

        let kafka_cfg = KafkaConfig {
            broker,
            auth: args.auth.clone(),
            sasl_username: args.sasl_username.clone(),
            sasl_password: args.sasl_password.clone(),
            ssl_ca_location: None,
            ssl_cert_location: None,
            ssl_key_location: None,
            schema_registry_url: args.schema_registry.clone(),
            sr_username: None,
            sr_password: None,
        };

        let result = scaffold_from_topic(topic, &kafka_cfg, config)?;

        eprintln!(
            "Sampled {} records.  Fields: {}   PII candidates: {}",
            result.sample_count,
            count_fields(&result.contract_yaml),
            result.pii_candidates.len(),
        );

        if args.dry_run {
            println!("{}", result.contract_yaml);
            return Ok(0);
        }

        write_output(&result.contract_yaml, args.output.as_deref())?;

        let exit = if !result.pii_candidates.is_empty() {
            4
        } else {
            0
        };
        return Ok(exit);
    }

    #[cfg(not(feature = "scaffold"))]
    {
        eprintln!(
            "error: live Kafka topic sampling requires the 'scaffold' feature.\n\
             Rebuild with: cargo build --features scaffold --bin contractgate\n\
             Or use --from-file to scaffold from a local file."
        );
        return Ok(2);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_output(yaml: &str, path: Option<&std::path::Path>) -> anyhow::Result<()> {
    match path {
        Some(p) => {
            std::fs::write(p, yaml).with_context(|| format!("cannot write {}", p.display()))?;
            eprintln!("Wrote {}", p.display());
        }
        None => print!("{yaml}"),
    }
    Ok(())
}

fn count_fields(yaml: &str) -> usize {
    yaml.lines()
        .filter(|l| {
            let trimmed = l.trim_start();
            trimmed.starts_with("- name:") && !trimmed.starts_with('#')
        })
        .count()
}

use anyhow::Context as _;
