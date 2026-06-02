//! Starter contract event-pass tests (RFC-017 Step 3).
//!
//! Passes a representative well-formed event through each starter contract
//! and asserts it passes validation with zero violations.
//!
//! Also passes a deliberately malformed event through each starter and
//! asserts it fails — confirming the validator actually enforces the contract.
//!
//! No DB, no network.

use contractgate::{
    contract::Contract,
    validation::{validate, CompiledContract},
};
use serde_json::json;
use std::path::PathBuf;

fn starters_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("contracts")
        .join("starters")
}

fn compile(filename: &str) -> CompiledContract {
    let path = starters_dir().join(filename);
    let yaml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {filename}: {e}"));
    let contract: Contract =
        serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("parse {filename}: {e}"));
    CompiledContract::compile(contract).unwrap_or_else(|e| panic!("compile {filename}: {e}"))
}

// ---------------------------------------------------------------------------
// rest_event
// ---------------------------------------------------------------------------

#[test]
fn rest_event_valid_event_passes() {
    let cc = compile("rest_event.yaml");
    let event = json!({
        "request_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "method": "GET",
        "path": "/api/users/123",
        "status": 200,
        "latency_ms": 42,
        "timestamp": 1700000000
    });
    let result = validate(&cc, &event);
    assert!(
        result.passed,
        "rest_event valid event should pass; violations: {:?}",
        result.violations
    );
    assert!(result.violations.is_empty());
}

#[test]
fn rest_event_invalid_method_fails() {
    let cc = compile("rest_event.yaml");
    let event = json!({
        "request_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "method": "CONNECT",  // not in enum
        "path": "/api/users/123",
        "status": 200,
        "latency_ms": 42,
        "timestamp": 1700000000
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "invalid method should fail");
    assert!(!result.violations.is_empty());
}

#[test]
fn rest_event_missing_required_field_fails() {
    let cc = compile("rest_event.yaml");
    let event = json!({
        // missing request_id
        "method": "POST",
        "path": "/api/orders",
        "status": 201,
        "latency_ms": 15,
        "timestamp": 1700000001
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "missing required field should fail");
}

// ---------------------------------------------------------------------------
// kafka_event
// ---------------------------------------------------------------------------

#[test]
fn kafka_event_valid_event_passes() {
    let cc = compile("kafka_event.yaml");
    let event = json!({
        "topic": "payments.processed",
        "partition": 3,
        "offset": 10042,
        "key": "ord_abc123",
        "producer_id": "payments-svc-01",
        "timestamp": 1700000100
    });
    let result = validate(&cc, &event);
    assert!(
        result.passed,
        "kafka_event valid event should pass; violations: {:?}",
        result.violations
    );
    assert!(result.violations.is_empty());
}

#[test]
fn kafka_event_without_optional_key_passes() {
    let cc = compile("kafka_event.yaml");
    // `key` is optional — omitting it should still pass.
    let event = json!({
        "topic": "payments.processed",
        "partition": 0,
        "offset": 1,
        "producer_id": "payments-svc-01",
        "timestamp": 1700000200
    });
    let result = validate(&cc, &event);
    assert!(
        result.passed,
        "kafka_event without optional key should pass; violations: {:?}",
        result.violations
    );
}

#[test]
fn kafka_event_invalid_producer_id_fails() {
    let cc = compile("kafka_event.yaml");
    let event = json!({
        "topic": "payments.processed",
        "partition": 0,
        "offset": 1,
        "producer_id": "bad producer id!",  // spaces + bang not allowed by pattern
        "timestamp": 1700000200
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "invalid producer_id pattern should fail");
}

#[test]
fn kafka_event_negative_partition_fails() {
    let cc = compile("kafka_event.yaml");
    let event = json!({
        "topic": "payments.processed",
        "partition": -1,  // below min: 0
        "offset": 1,
        "producer_id": "svc-01",
        "timestamp": 1700000200
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "negative partition should fail");
}

// ---------------------------------------------------------------------------
// dbt_model
// ---------------------------------------------------------------------------

#[test]
fn dbt_model_valid_event_passes() {
    let cc = compile("dbt_model.yaml");
    let event = json!({
        "id": "row_a1b2c3d4e5f6",
        "created_at": 1700000000,
        "updated_at": 1700001000,
        "source_system": "postgres"
    });
    let result = validate(&cc, &event);
    assert!(
        result.passed,
        "dbt_model valid event should pass; violations: {:?}",
        result.violations
    );
    assert!(result.violations.is_empty());
}

#[test]
fn dbt_model_with_deleted_at_passes() {
    let cc = compile("dbt_model.yaml");
    let event = json!({
        "id": "row_deadbeef",
        "created_at": 1700000000,
        "updated_at": 1700001000,
        "deleted_at": 1700002000,
        "source_system": "snowflake"
    });
    let result = validate(&cc, &event);
    assert!(
        result.passed,
        "dbt_model with deleted_at should pass; violations: {:?}",
        result.violations
    );
}

#[test]
fn dbt_model_invalid_source_system_fails() {
    let cc = compile("dbt_model.yaml");
    let event = json!({
        "id": "row_xyz",
        "created_at": 1700000000,
        "updated_at": 1700001000,
        "source_system": "oracle"  // not in enum
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "invalid source_system should fail");
}

#[test]
fn dbt_model_missing_source_system_fails() {
    let cc = compile("dbt_model.yaml");
    let event = json!({
        "id": "row_xyz",
        "created_at": 1700000000,
        "updated_at": 1700001000
        // missing required source_system
    });
    let result = validate(&cc, &event);
    assert!(!result.passed, "missing required source_system should fail");
}
