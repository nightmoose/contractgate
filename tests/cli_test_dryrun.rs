//! Integration tests for `cg test` (RFC-076).
//!
//! All tests are purely local (no network, no DB).
//! They call `test::run` directly with fixture files from `tests/fixtures/cli/`.

use contractgate::cli::commands::test::{Format, TestArgs, run};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("cli")
}

fn contract_path() -> PathBuf {
    fixtures_dir().join("valid_user_events.yaml")
}

fn args(data: &str) -> TestArgs {
    TestArgs {
        contract: contract_path(),
        data: data.to_string(),
        format: Format::Human,
        fail_fast: false,
        quiet: false,
    }
}

fn args_json(data: &str) -> TestArgs {
    TestArgs {
        contract: contract_path(),
        data: data.to_string(),
        format: Format::Json,
        fail_fast: false,
        quiet: false,
    }
}

// ── Happy path ────────────────────────────────────────────────────────────────

#[test]
fn all_pass_ndjson_exits_0() {
    let path = fixtures_dir().join("test_all_pass.ndjson");
    let code = run(&args(path.to_str().unwrap())).expect("run");
    assert_eq!(code, 0, "all pass → exit 0");
}

#[test]
fn all_pass_json_array_exits_0() {
    let path = fixtures_dir().join("test_array.json");
    let code = run(&args(path.to_str().unwrap())).expect("run");
    assert_eq!(code, 0, "JSON array, all pass → exit 0");
}

#[test]
fn single_object_all_pass_exits_0() {
    // Write a temp single-object file.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"{"user_id":"alice","event_type":"click","timestamp":1000}"#,
    )
    .unwrap();
    let code = run(&args(tmp.path().to_str().unwrap())).expect("run");
    assert_eq!(code, 0);
}

// ── Violation path ────────────────────────────────────────────────────────────

#[test]
fn mixed_violations_exits_1() {
    let path = fixtures_dir().join("test_mixed_fail.ndjson");
    let code = run(&args(path.to_str().unwrap())).expect("run");
    assert_eq!(code, 1, "violations found → exit 1");
}

#[test]
fn missing_required_field_exits_1() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    // Missing timestamp (required).
    std::fs::write(tmp.path(), r#"{"user_id":"alice","event_type":"click"}"#).unwrap();
    let code = run(&args(tmp.path().to_str().unwrap())).expect("run");
    assert_eq!(code, 1);
}

// ── Format json ───────────────────────────────────────────────────────────────

#[test]
fn json_format_all_pass_exits_0() {
    let path = fixtures_dir().join("test_all_pass.ndjson");
    let code = run(&args_json(path.to_str().unwrap())).expect("run");
    assert_eq!(code, 0);
}

#[test]
fn json_format_violations_exits_1() {
    let path = fixtures_dir().join("test_mixed_fail.ndjson");
    let code = run(&args_json(path.to_str().unwrap())).expect("run");
    assert_eq!(code, 1);
}

// ── Load/parse errors (exit 2 class — run() returns Err) ─────────────────────

#[test]
fn bad_contract_yaml_returns_err() {
    let tmp_contract = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp_contract.path(), "this: is: not: valid: yaml: :::").unwrap();
    let data_path = fixtures_dir().join("test_all_pass.ndjson");

    let bad_args = TestArgs {
        contract: tmp_contract.path().to_path_buf(),
        data: data_path.to_str().unwrap().to_string(),
        format: Format::Human,
        fail_fast: false,
        quiet: false,
    };
    // run() returns Err for load/compile failures (caller maps to exit 2).
    assert!(run(&bad_args).is_err(), "bad contract YAML → Err");
}

#[test]
fn missing_data_file_returns_err() {
    let bad_args = TestArgs {
        contract: contract_path(),
        data: "/nonexistent/path/data.ndjson".to_string(),
        format: Format::Human,
        fail_fast: false,
        quiet: false,
    };
    assert!(run(&bad_args).is_err(), "missing data file → Err");
}

// ── --fail-fast ───────────────────────────────────────────────────────────────

#[test]
fn fail_fast_exits_1_on_first_failing_record() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    // 3 records, record 0 fails (missing required fields).
    std::fs::write(
        tmp.path(),
        "{}\n{\"user_id\":\"x\",\"event_type\":\"click\",\"timestamp\":1}\n{}\n",
    )
    .unwrap();

    let ff_args = TestArgs {
        contract: contract_path(),
        data: tmp.path().to_str().unwrap().to_string(),
        format: Format::Human,
        fail_fast: true,
        quiet: true,
    };
    let code = run(&ff_args).expect("run");
    assert_eq!(code, 1);
}

// ── Quiet mode ────────────────────────────────────────────────────────────────

#[test]
fn quiet_mode_does_not_panic() {
    let path = fixtures_dir().join("test_mixed_fail.ndjson");
    let quiet_args = TestArgs {
        contract: contract_path(),
        data: path.to_str().unwrap().to_string(),
        format: Format::Human,
        fail_fast: false,
        quiet: true,
    };
    let code = run(&quiet_args).expect("run");
    assert_eq!(code, 1);
}
