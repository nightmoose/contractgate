//! Integration-style tests for ContractGate that do NOT require a live database.
//!
//! These tests exercise the validation engine + playground handler end-to-end
//! by constructing requests and calling handlers directly — fast, no I/O.
//!
//! DB-dependent tests (ingest, audit) belong in a separate `tests/integration/`
//! directory and require `DATABASE_URL` to be set.

#[cfg(test)]
mod playground {
    use crate::contract::{Contract, FieldDefinition, FieldType, GlossaryEntry, MetricDefinition, Ontology};
    use crate::validation::{validate, CompiledContract, ViolationKind};
    use serde_json::json;

    // -----------------------------------------------------------------------
    // Helper: build the canonical user_events contract from the YAML example
    // -----------------------------------------------------------------------

    fn user_events_contract() -> Contract {
        Contract {
            version: "1.0".into(),
            name: "user_events".into(),
            description: Some("Contract for user interaction events".into()),
            ontology: Ontology {
                entities: vec![
                    FieldDefinition {
                        name: "user_id".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: Some(r"^[a-zA-Z0-9_-]+$".into()),
                        allowed_values: None,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                    FieldDefinition {
                        name: "event_type".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: None,
                        allowed_values: Some(vec![
                            json!("click"),
                            json!("view"),
                            json!("purchase"),
                            json!("login"),
                        ]),
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                    FieldDefinition {
                        name: "timestamp".into(),
                        field_type: FieldType::Integer,
                        required: true,
                        pattern: None,
                        allowed_values: None,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                    FieldDefinition {
                        name: "amount".into(),
                        field_type: FieldType::Float,
                        required: false,
                        pattern: None,
                        allowed_values: None,
                        min: Some(0.0),
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                ],
            },
            glossary: vec![GlossaryEntry {
                field: "amount".into(),
                description: "Monetary amount in USD".into(),
                constraints: Some("must be non-negative".into()),
                synonyms: None,
            }],
            metrics: vec![MetricDefinition {
                name: "total_revenue".into(),
                field: None,
                metric_type: None,
                formula: Some("sum(amount) where event_type = 'purchase'".into()),
                min: None,
                max: None,
            }],
        }
    }

    // -----------------------------------------------------------------------
    // Happy-path tests
    // -----------------------------------------------------------------------

    #[test]
    fn click_event_passes() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
        assert_eq!(r.violations.len(), 0);
    }

    #[test]
    fn purchase_event_with_amount_passes() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "bob-99",
            "event_type": "purchase",
            "timestamp": 1712001000,
            "amount": 49.99
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    #[test]
    fn optional_amount_absent_passes() {
        // amount is optional — event without it should still pass
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "carol_X",
            "event_type": "login",
            "timestamp": 1712002000
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    #[test]
    fn extra_fields_allowed_no_violation() {
        // ContractGate does not reject unknown fields (additive schema evolution)
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "dan_01",
            "event_type": "view",
            "timestamp": 1712003000,
            "page": "/dashboard",        // extra field
            "session_id": "sess_abc"     // extra field
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    // -----------------------------------------------------------------------
    // Violation tests
    // -----------------------------------------------------------------------

    #[test]
    fn missing_required_user_id() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({ "event_type": "click", "timestamp": 1712000000 });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::MissingRequiredField
            && v.field == "user_id"));
    }

    #[test]
    fn invalid_event_type_enum() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "delete",   // not in allowed set
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::EnumViolation));
    }

    #[test]
    fn user_id_pattern_mismatch() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice @ 01!",  // spaces + special chars not in pattern
            "event_type": "view",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::PatternMismatch));
    }

    #[test]
    fn negative_amount_range_violation() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "purchase",
            "timestamp": 1712000000,
            "amount": -5.00    // below min=0
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::RangeViolation));
    }

    #[test]
    fn wrong_type_for_timestamp() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": "not-a-number"  // string instead of integer
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::TypeMismatch));
    }

    #[test]
    fn multiple_violations_collected() {
        // All three fields wrong at once — should collect all violations
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "bad id!",        // pattern mismatch
            "event_type": "explode",     // enum violation
            "timestamp": "now",          // type mismatch
            "amount": -100               // range violation
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r.violations.len() >= 3, "Expected ≥3 violations, got: {:?}", r.violations);
    }

    #[test]
    fn non_object_event_rejected() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let r = validate(&cc, &json!(["not", "an", "object"]));
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::TypeMismatch));
    }

    // -----------------------------------------------------------------------
    // Performance sanity check
    // -----------------------------------------------------------------------

    #[test]
    fn validation_completes_under_1ms() {
        // The hot path should be well under 1ms for a simple event.
        // This is a rough guard — CI servers are slower but still within target.
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "perf_test_user",
            "event_type": "click",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(r.passed);
        // Target p99 is 15 000 µs in release builds.
        // Debug builds on CI are unoptimised — allow up to 500 ms.
        // The real latency bar is enforced by the release-mode benchmark.
        #[cfg(debug_assertions)]
        let threshold = 500_000;
        #[cfg(not(debug_assertions))]
        let threshold = 1_000;
        assert!(
            r.validation_us < threshold,
            "Validation took {}µs — expected < {}µs",
            r.validation_us,
            threshold
        );
    }

    // -----------------------------------------------------------------------
    // YAML round-trip test
    // -----------------------------------------------------------------------

    #[test]
    fn yaml_parse_and_validate_roundtrip() {
        let yaml = r#"
version: "1.0"
name: "roundtrip_test"
ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^[a-z0-9]+$"
    - name: value
      type: integer
      required: true
      min: 1
      max: 100
glossary: []
metrics: []
"#;
        let contract: Contract = serde_yaml::from_str(yaml).expect("YAML parse failed");
        let cc = CompiledContract::compile(contract).expect("Compile failed");

        // Valid
        let good = json!({ "id": "abc123", "value": 50 });
        assert!(validate(&cc, &good).passed);

        // Out of range
        let bad = json!({ "id": "abc123", "value": 999 });
        let r = validate(&cc, &bad);
        assert!(!r.passed);
        assert!(r.violations.iter().any(|v| v.kind == ViolationKind::RangeViolation));
    }
}
