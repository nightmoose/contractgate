//! Integration tests for `contractgate push` and `contractgate pull`.
//!
//! These tests require a live ContractGate gateway (with a real database).
//! They are marked `#[ignore]` and run via:
//!
//!   cargo test --test cli_push_pull -- --ignored
//!
//! The gateway URL and API key are read from the CONTRACTGATE_URL and
//! CONTRACTGATE_API_KEY environment variables (or a local .env file).
//!
//! Per the RFC-014 test plan (step §7), a full in-process axum server variant
//! (tokio::test + axum::serve) is tracked under a follow-up task once the
//! shared DB integration harness (see RFC-002/RFC-003 deferred items in
//! MAINTENANCE_LOG.md) is available.

use contractgate::cli::{
    commands::{
        pull::{run as pull_run, PullArgs},
        push::{run as push_run, PushArgs},
    },
    config::{CliConfig, GatewayConfig},
};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("cli")
}

fn live_cfg() -> CliConfig {
    let url = std::env::var("CONTRACTGATE_URL").unwrap_or_else(|_| "http://localhost:3000".into());

    CliConfig {
        gateway: GatewayConfig { url },
        ..Default::default()
    }
}

fn api_key() -> String {
    std::env::var("CONTRACTGATE_API_KEY").unwrap_or_else(|_| "test-key".into())
}

/// Push 3 contracts to a live gateway, then pull them back and assert the
/// pulled YAML round-trips through the validate path without error.
#[test]
#[ignore = "requires live gateway + DB (see docs/rfcs/014-cli-core.md §Test plan)"]
fn push_and_pull_round_trip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = fixtures_dir();

    // Copy 2 valid fixtures into a push source dir.
    let push_dir = tmp.path().join("push_src");
    std::fs::create_dir_all(&push_dir).unwrap();
    for entry in ["valid_user_events.yaml", "valid_orders.yaml"] {
        std::fs::copy(src.join(entry), push_dir.join(entry)).unwrap();
    }

    let cfg = live_cfg();
    let key = api_key();

    // Push.
    let push_args = PushArgs {
        dir: Some(push_dir),
        dry_run: false,
        json: false,
    };
    let code = push_run(&push_args, &cfg, &key).expect("push::run");
    assert_eq!(code, 0, "push should succeed against live gateway");

    // Pull into a fresh dir.
    let pull_dir = tmp.path().join("pull_out");
    std::fs::create_dir_all(&pull_dir).unwrap();
    let pull_args = PullArgs {
        name: None,
        out: Some(pull_dir.clone()),
        json: false,
    };
    let code = pull_run(&pull_args, &cfg, &key).expect("pull::run");
    assert_eq!(code, 0, "pull should succeed against live gateway");

    // Pulled files must be valid contracts.
    let pulled: Vec<_> = std::fs::read_dir(&pull_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "yaml").unwrap_or(false))
        .collect();

    assert!(
        !pulled.is_empty(),
        "pull should write at least one YAML file"
    );

    for entry in pulled {
        let path = entry.path();
        let yaml = std::fs::read_to_string(&path).unwrap();
        let _contract: contractgate::contract::Contract = serde_yaml::from_str(&yaml)
            .unwrap_or_else(|e| panic!("pulled file {} failed to parse: {e}", path.display()));
    }
}

/// Dry-run push should succeed and exit 0 without hitting the network.
#[test]
fn push_dry_run_exits_0_no_network() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("valid_user_events.yaml"),
        tmp.path().join("valid_user_events.yaml"),
    )
    .unwrap();

    // api_key doesn't matter — dry-run skips auth.
    let push_args = PushArgs {
        dir: Some(tmp.path().to_path_buf()),
        dry_run: true,
        json: false,
    };
    // Use a bogus URL — dry-run must not make network calls.
    let cfg = CliConfig {
        gateway: GatewayConfig {
            url: "http://127.0.0.1:1".into(),
        },
        ..Default::default()
    };

    let code = push_run(&push_args, &cfg, "no-key").expect("push::run dry-run");
    assert_eq!(code, 0, "dry-run of valid contract → exit 0");
}

/// Dry-run push with an invalid YAML exits 1.
#[test]
fn push_dry_run_invalid_yaml_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("invalid_bad_yaml.yaml"),
        tmp.path().join("invalid_bad_yaml.yaml"),
    )
    .unwrap();

    let push_args = PushArgs {
        dir: Some(tmp.path().to_path_buf()),
        dry_run: true,
        json: false,
    };

    let cfg = CliConfig {
        gateway: GatewayConfig {
            url: "http://127.0.0.1:1".into(),
        },
        ..Default::default()
    };

    let code = push_run(&push_args, &cfg, "no-key").expect("push::run dry-run invalid");
    assert_eq!(code, 1, "dry-run of invalid contract → exit 1");
}
