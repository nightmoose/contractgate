//! Starter contract parse + compile tests (RFC-017 Step 3).
//!
//! Each test loads a starter YAML from `contracts/starters/`, parses it into
//! `Contract`, and compiles it into a `CompiledContract`.  Any parse error or
//! compile error (bad regex, bad field type, etc.) is a test failure.
//!
//! No DB, no network.  Pure local validation.

use contractgate::{contract::Contract, validation::CompiledContract};
use std::path::PathBuf;

fn starters_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("contracts")
        .join("starters")
}

fn load_and_compile(filename: &str) -> CompiledContract {
    let path = starters_dir().join(filename);
    let yaml =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {filename}: {e}"));
    let contract: Contract =
        serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
    CompiledContract::compile(contract)
        .unwrap_or_else(|e| panic!("failed to compile {filename}: {e}"))
}

#[test]
fn rest_event_parses_and_compiles() {
    let cc = load_and_compile("rest_event.yaml");
    assert_eq!(cc.contract.name, "rest_event");
    assert_eq!(cc.contract.version, "1.0");
    // Six declared fields.
    assert_eq!(cc.contract.ontology.entities.len(), 6);
}

#[test]
fn kafka_event_parses_and_compiles() {
    let cc = load_and_compile("kafka_event.yaml");
    assert_eq!(cc.contract.name, "kafka_event");
    assert_eq!(cc.contract.version, "1.0");
    // Six declared fields.
    assert_eq!(cc.contract.ontology.entities.len(), 6);
}

#[test]
fn dbt_model_parses_and_compiles() {
    let cc = load_and_compile("dbt_model.yaml");
    assert_eq!(cc.contract.name, "dbt_model_row");
    assert_eq!(cc.contract.version, "1.0");
    // Five declared fields.
    assert_eq!(cc.contract.ontology.entities.len(), 5);
}

#[test]
fn all_starters_compile_with_zero_errors() {
    for filename in &["rest_event.yaml", "kafka_event.yaml", "dbt_model.yaml"] {
        // load_and_compile panics on any error — just confirm it returns.
        let _ = load_and_compile(filename);
    }
}
