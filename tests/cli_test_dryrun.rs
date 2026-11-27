//! Integration tests for `cg test` (RFC-076).
//!
//! All tests are purely local (no network, no DB).
//! They call `test::run` directly with fixture files from `tests/fixtures/cli/`.

use contractgate::cli::commands::test::{run, DataFormat, Format, TestArgs};
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
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: false,
        quiet: false,
        schema_object: None,
    }
}

fn args_json(data: &str) -> TestArgs {
    TestArgs {
        contract: contract_path(),
        data: data.to_string(),
        data_format: DataFormat::Auto,
        format: Format::Json,
        fail_fast: false,
        quiet: false,
        schema_object: None,
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
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: false,
        quiet: false,
        schema_object: None,
    };
    // run() returns Err for load/compile failures (caller maps to exit 2).
    assert!(run(&bad_args).is_err(), "bad contract YAML → Err");
}

#[test]
fn missing_data_file_returns_err() {
    let bad_args = TestArgs {
        contract: contract_path(),
        data: "/nonexistent/path/data.ndjson".to_string(),
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: false,
        quiet: false,
        schema_object: None,
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
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: true,
        quiet: true,
        schema_object: None,
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
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: false,
        quiet: true,
        schema_object: None,
    };
    let code = run(&quiet_args).expect("run");
    assert_eq!(code, 1);
}

// ── CSV data files ────────────────────────────────────────────────────────────

#[test]
fn csv_detected_from_extension_all_pass_exits_0() {
    let tmp = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(
        tmp.path(),
        "user_id,event_type,timestamp,amount\n\
         alice,click,1000,\n\
         bob,purchase,1001,19.99\n",
    )
    .unwrap();
    let code = run(&args(tmp.path().to_str().unwrap())).expect("run");
    assert_eq!(
        code, 0,
        "CSV with a typed number and an empty optional cell should pass"
    );
}

#[test]
fn csv_empty_cell_omits_the_key_so_a_required_field_still_fails() {
    let tmp = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(
        tmp.path(),
        "user_id,event_type,timestamp,amount\n\
         ,click,1000,\n",
    )
    .unwrap();
    let code = run(&args(tmp.path().to_str().unwrap())).expect("run");
    assert_eq!(
        code, 1,
        "empty cell on a required field must fail, not be dropped"
    );
}

#[test]
fn csv_explicit_data_format_works_without_csv_extension() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        "user_id,event_type,timestamp,amount\nalice,click,1000,5\n",
    )
    .unwrap();
    let csv_args = TestArgs {
        contract: contract_path(),
        data: tmp.path().to_str().unwrap().to_string(),
        data_format: DataFormat::Csv,
        format: Format::Human,
        fail_fast: false,
        quiet: false,
        schema_object: None,
    };
    let code = run(&csv_args).expect("run");
    assert_eq!(code, 0);
}

#[test]
fn csv_non_numeric_cell_in_numeric_column_is_a_type_mismatch_not_a_crash() {
    let tmp = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(
        tmp.path(),
        "user_id,event_type,timestamp,amount\nalice,click,not_a_number,5\n",
    )
    .unwrap();
    let code = run(&args(tmp.path().to_str().unwrap())).expect("run");
    assert_eq!(
        code, 1,
        "a non-numeric timestamp should fail validation, not panic"
    );
}

#[test]
fn csv_literal_bracketed_and_dotted_column_names_round_trip() {
    use contractgate::cli::commands::synth::{self as synth_cli, Format as SynthFormat, SynthArgs};

    const CONTRACT: &str = r#"
version: 1.0.0
kind: DataContract
apiVersion: v3.2.0
id: literal-names
name: Literal Names
status: draft
schema:
- name: t
  physicalType: table
  logicalType: object
  physicalName: t
  properties:
  - name: Amount [A]
    physicalType: DOUBLE
    logicalType: number
  - name: Rate.x
    physicalType: DOUBLE
    logicalType: number
  - name: plain
    physicalType: DOUBLE
    logicalType: number
"#;
    let tmp = tempfile::tempdir().unwrap();
    let contract = tmp.path().join("datacontract.yaml");
    std::fs::write(&contract, CONTRACT).unwrap();
    let csv_out = tmp.path().join("rows.csv");
    let code = synth_cli::run(&SynthArgs {
        contract: contract.clone(),
        rows: 15,
        invalid: false,
        seed: Some(5),
        anchor: None,
        envelope: false,
        format: SynthFormat::Csv,
        out: Some(csv_out.clone()),
        schema_object: None,
        synth_config: None,
    })
    .unwrap();
    assert_eq!(code, 0);

    let test_args = TestArgs {
        contract,
        data: csv_out.to_str().unwrap().to_string(),
        data_format: DataFormat::Auto,
        format: Format::Human,
        fail_fast: false,
        quiet: false,
        schema_object: None,
    };
    let code = run(&test_args).expect("run");
    assert_eq!(
        code, 0,
        "synth --format csv output with literal dotted/bracketed column names must round-trip through test"
    );
}
