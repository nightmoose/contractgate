//! Integration tests for `contractgate validate`.
//!
//! Tests are purely local (no network, no DB).  They call `validate::run`
//! directly with fixture files from `tests/fixtures/cli/`.

use contractgate::cli::{
    commands::validate::{ValidateArgs, run},
    config::CliConfig,
};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("cli")
}

fn cfg() -> CliConfig {
    CliConfig::default()
}

fn args_for_dir(dir: PathBuf) -> ValidateArgs {
    ValidateArgs {
        dir: Some(dir),
        json: false,
    }
}

fn args_for_dir_json(dir: PathBuf) -> ValidateArgs {
    ValidateArgs {
        dir: Some(dir),
        json: true,
    }
}

// ── Happy path ────────────────────────────────────────────────────────────────

#[test]
fn validate_valid_contracts_exits_0() {
    // Create a temp dir with only valid fixtures.
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = fixtures_dir();
    for entry in ["valid_user_events.yaml", "valid_orders.yaml"] {
        std::fs::copy(src.join(entry), tmp.path().join(entry)).unwrap();
    }

    let args = args_for_dir(tmp.path().to_path_buf());
    let cfg = CliConfig::default();
    let code = run(&args, &cfg).expect("run");
    assert_eq!(code, 0, "all valid → exit 0");
}

#[test]
fn validate_single_valid_file_exits_0() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("valid_user_events.yaml"),
        tmp.path().join("valid_user_events.yaml"),
    )
    .unwrap();

    let args = args_for_dir(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 0);
}

// ── Error path ────────────────────────────────────────────────────────────────

#[test]
fn validate_bad_yaml_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("invalid_bad_yaml.yaml"),
        tmp.path().join("invalid_bad_yaml.yaml"),
    )
    .unwrap();

    let args = args_for_dir(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 1, "bad YAML → exit 1");
}

#[test]
fn validate_missing_required_field_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("invalid_missing_name.yaml"),
        tmp.path().join("invalid_missing_name.yaml"),
    )
    .unwrap();

    let args = args_for_dir(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 1, "missing required field → exit 1");
}

#[test]
fn validate_mixed_valid_invalid_exits_1() {
    // One valid + one invalid → exit 1.
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = fixtures_dir();
    std::fs::copy(
        src.join("valid_user_events.yaml"),
        tmp.path().join("valid_user_events.yaml"),
    )
    .unwrap();
    std::fs::copy(
        src.join("invalid_bad_yaml.yaml"),
        tmp.path().join("invalid_bad_yaml.yaml"),
    )
    .unwrap();

    let args = args_for_dir(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 1, "any failure → exit 1");
}

#[test]
fn validate_empty_dir_exits_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let args = args_for_dir(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 1, "no files matched → exit 1");
}

// ── JSON flag ─────────────────────────────────────────────────────────────────

#[test]
fn validate_json_flag_does_not_panic() {
    // Just verify --json mode doesn't crash.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        fixtures_dir().join("valid_user_events.yaml"),
        tmp.path().join("valid_user_events.yaml"),
    )
    .unwrap();

    let args = args_for_dir_json(tmp.path().to_path_buf());
    let code = run(&args, &cfg()).expect("run");
    assert_eq!(code, 0);
}
