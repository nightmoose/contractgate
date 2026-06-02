//! Integration tests for the brownfield scaffolder (RFC-024 §I).
//!
//! These tests run entirely offline (no Kafka, no SR).  The Kafka integration
//! tests require the `kafka-test` compose profile and run in CI only.

use contractgate::contract::Contract;
use contractgate::scaffold::{scaffold_from_file, InputFormat, ScaffoldConfig};
use std::io::Write;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/samples")
        .join(name)
}

fn default_cfg(name: &str) -> ScaffoldConfig {
    ScaffoldConfig {
        name: name.to_string(),
        pii_threshold: 0.4,
        ..Default::default()
    }
}

/// Strip comment lines from YAML and parse as a Contract.
fn parse_contract(yaml: &str) -> Contract {
    let clean: String = yaml
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    serde_yaml::from_str(&clean)
        .unwrap_or_else(|e| panic!("YAML parse failed:\n{yaml}\n\nerror: {e}"))
}

// ---------------------------------------------------------------------------
// JSON scaffold
// ---------------------------------------------------------------------------

#[test]
fn scaffold_json_from_fixture_produces_valid_contract() {
    let result = scaffold_from_file(&fixture("user_events.json"), &default_cfg("user_events"))
        .expect("scaffold should succeed");

    assert_eq!(result.format, InputFormat::Json);
    assert_eq!(result.sample_count, 10);

    let contract = parse_contract(&result.contract_yaml);
    assert_eq!(contract.name, "user_events");

    // All four top-level fields must be present.
    let field_names: Vec<&str> = contract
        .ontology
        .entities
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(field_names.contains(&"user_id"), "missing user_id");
    assert!(field_names.contains(&"event_type"), "missing event_type");
    assert!(field_names.contains(&"timestamp"), "missing timestamp");
    assert!(field_names.contains(&"amount"), "missing amount");
}

#[test]
fn scaffold_json_marks_amount_optional_due_to_nulls() {
    let result = scaffold_from_file(&fixture("user_events.json"), &default_cfg("user_events"))
        .expect("scaffold should succeed");
    let contract = parse_contract(&result.contract_yaml);
    let amount = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "amount")
        .expect("amount field should exist");
    assert!(
        !amount.required,
        "amount appears in only some events → required: false"
    );
}

#[test]
fn scaffold_json_stat_comments_present() {
    let result = scaffold_from_file(&fixture("user_events.json"), &default_cfg("user_events"))
        .expect("scaffold should succeed");
    assert!(
        result.contract_yaml.contains("# scaffold: null_rate="),
        "stat comments should be embedded"
    );
}

#[test]
fn scaffold_json_no_transform_in_live_yaml() {
    let result = scaffold_from_file(&fixture("user_events.json"), &default_cfg("user_events"))
        .expect("scaffold should succeed");
    let contract = parse_contract(&result.contract_yaml);
    for field in &contract.ontology.entities {
        assert!(
            field.transform.is_none(),
            "transform must not be auto-applied (field: {})",
            field.name
        );
    }
}

// ---------------------------------------------------------------------------
// Avro schema scaffold
// ---------------------------------------------------------------------------

#[test]
fn scaffold_avro_from_fixture_produces_valid_contract() {
    let result = scaffold_from_file(&fixture("orders.avsc"), &default_cfg("orders"))
        .expect("scaffold should succeed");

    assert_eq!(result.format, InputFormat::AvroSchema);
    assert_eq!(result.sample_count, 0); // schema-only, no sample values

    let contract = parse_contract(&result.contract_yaml);
    let names: Vec<&str> = contract
        .ontology
        .entities
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&"order_id"), "missing order_id");
    assert!(names.contains(&"customer_id"), "missing customer_id");
    assert!(names.contains(&"email"), "missing email");
    assert!(names.contains(&"amount"), "missing amount");
}

#[test]
fn scaffold_avro_detects_email_as_pii() {
    let result = scaffold_from_file(&fixture("orders.avsc"), &default_cfg("orders"))
        .expect("scaffold should succeed");
    assert!(
        result
            .pii_candidates
            .iter()
            .any(|c| c.field_name == "email"),
        "email field should be a PII candidate"
    );
    assert!(
        result.contract_yaml.contains("# TODO"),
        "PII TODO comment should be present"
    );
}

#[test]
fn scaffold_avro_email_is_optional_from_union_null() {
    let result = scaffold_from_file(&fixture("orders.avsc"), &default_cfg("orders"))
        .expect("scaffold should succeed");
    let contract = parse_contract(&result.contract_yaml);
    let email = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "email")
        .expect("email field should exist");
    assert!(
        !email.required,
        "email is nullable in Avro schema → required: false"
    );
}

// ---------------------------------------------------------------------------
// Protobuf schema scaffold
// ---------------------------------------------------------------------------

#[test]
fn scaffold_proto_from_fixture_produces_valid_contract() {
    let result = scaffold_from_file(&fixture("page_views.proto"), &default_cfg("page_views"))
        .expect("scaffold should succeed");

    assert_eq!(result.format, InputFormat::Proto);

    let contract = parse_contract(&result.contract_yaml);
    let names: Vec<&str> = contract
        .ontology
        .entities
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&"session_id"), "missing session_id");
    assert!(names.contains(&"user_id"), "missing user_id");
    assert!(names.contains(&"url"), "missing url");
    assert!(names.contains(&"timestamp"), "missing timestamp");
    assert!(names.contains(&"duration_ms"), "missing duration_ms");
    assert!(names.contains(&"ip_address"), "missing ip_address");
}

#[test]
fn scaffold_proto_detects_ip_address_as_pii() {
    let result = scaffold_from_file(&fixture("page_views.proto"), &default_cfg("page_views"))
        .expect("scaffold should succeed");
    assert!(
        result
            .pii_candidates
            .iter()
            .any(|c| c.field_name == "ip_address"),
        "ip_address should be a PII candidate"
    );
}

// ---------------------------------------------------------------------------
// SR unavailable fallback annotation
// ---------------------------------------------------------------------------

#[test]
fn scaffold_avro_schema_file_has_no_sr_unavailable_warning() {
    // Schema file path: no SR needed — this annotation should NOT appear.
    let result = scaffold_from_file(&fixture("orders.avsc"), &default_cfg("orders"))
        .expect("scaffold should succeed");
    assert!(
        !result.sr_unavailable,
        "schema-file path should not mark SR as unavailable"
    );
}

// ---------------------------------------------------------------------------
// Golden contract round-trip
// ---------------------------------------------------------------------------

#[test]
fn scaffolded_yaml_round_trips_through_serde_yaml() {
    // Any scaffold output must parse as a Contract without errors.
    for fixture_file in &["user_events.json", "orders.avsc", "page_views.proto"] {
        let result = scaffold_from_file(
            &fixture(fixture_file),
            &ScaffoldConfig {
                name: "round_trip_test".to_string(),
                ..Default::default()
            },
        )
        .expect("scaffold should succeed");

        let contract = parse_contract(&result.contract_yaml);
        assert!(
            !contract.ontology.entities.is_empty(),
            "contract from {fixture_file} should have at least one field"
        );
    }
}

// ---------------------------------------------------------------------------
// --fast flag: no profiler stats, still valid output
// ---------------------------------------------------------------------------

#[test]
fn scaffold_fast_flag_skips_stats_comments() {
    let result = scaffold_from_file(
        &fixture("user_events.json"),
        &ScaffoldConfig {
            name: "fast_test".to_string(),
            fast: true,
            ..Default::default()
        },
    )
    .expect("scaffold should succeed");

    assert!(
        !result.contract_yaml.contains("# scaffold: null_rate"),
        "--fast should suppress stat comments"
    );
    // Output should still be parseable.
    let _contract = parse_contract(&result.contract_yaml);
}

// ---------------------------------------------------------------------------
// NDJSON input
// ---------------------------------------------------------------------------

#[test]
fn scaffold_ndjson_inline() {
    let ndjson = "{\"k\": \"v1\"}\n{\"k\": \"v2\"}\n{\"k\": null}\n";
    let mut f = tempfile::Builder::new()
        .suffix(".ndjson")
        .tempfile()
        .unwrap();
    f.write_all(ndjson.as_bytes()).unwrap();

    let result = scaffold_from_file(
        f.path(),
        &ScaffoldConfig {
            name: "ndjson_inline".to_string(),
            ..Default::default()
        },
    )
    .expect("scaffold should succeed");

    assert_eq!(result.format, InputFormat::NdJson);
    assert_eq!(result.sample_count, 3);
    let contract = parse_contract(&result.contract_yaml);
    assert!(contract.ontology.entities.iter().any(|f| f.name == "k"));
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn unsupported_extension_returns_error() {
    let mut f = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    f.write_all(b"a,b,c\n1,2,3\n").unwrap();
    let result = scaffold_from_file(f.path(), &ScaffoldConfig::default());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("unsupported"));
}

#[test]
fn malformed_json_returns_error() {
    let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    f.write_all(b"{ not valid json }").unwrap();
    let result = scaffold_from_file(f.path(), &ScaffoldConfig::default());
    assert!(result.is_err());
}

#[test]
fn malformed_avsc_returns_error() {
    let mut f = tempfile::Builder::new().suffix(".avsc").tempfile().unwrap();
    f.write_all(b"{ \"type\": \"not_record\" }").unwrap();
    let result = scaffold_from_file(f.path(), &ScaffoldConfig::default());
    assert!(result.is_err());
}
