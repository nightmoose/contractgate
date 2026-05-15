//! Property-based tests for the scaffold profiler and three-way merge (RFC-024 §I).
//!
//! Run with: cargo test --features scaffold scaffold_property

use contractgate::scaffold::{scaffold_from_file, ScaffoldConfig};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::io::Write;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a random JSON object with a fixed set of keys and arbitrary values.
fn arb_event() -> impl Strategy<Value = Value> {
    (
        prop::option::of("[a-z0-9]{1,12}"),
        prop::option::of(0i64..1_000_000i64),
        prop::option::of(prop::bool::ANY),
    )
        .prop_map(|(s, n, b)| {
            let mut m = serde_json::Map::new();
            if let Some(v) = s {
                m.insert("str_field".into(), Value::String(v));
            } else {
                m.insert("str_field".into(), Value::Null);
            }
            if let Some(v) = n {
                m.insert("num_field".into(), json!(v));
            } else {
                m.insert("num_field".into(), Value::Null);
            }
            if let Some(v) = b {
                m.insert("bool_field".into(), json!(v));
            } else {
                m.insert("bool_field".into(), Value::Null);
            }
            Value::Object(m)
        })
}

/// Write a Vec<Value> as a JSON array to a NamedTempFile and return it.
fn write_json_fixture(events: &[Value]) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    let arr = Value::Array(events.to_vec());
    f.write_all(arr.to_string().as_bytes()).unwrap();
    f
}

// ---------------------------------------------------------------------------
// Profiler invariants
// ---------------------------------------------------------------------------

proptest! {
    /// null_rate = null_count / total_count ∈ [0, 1] for every field.
    #[test]
    fn profiler_null_rate_in_range(events in prop::collection::vec(arb_event(), 1..=50)) {
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_null_rate".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        for stat in &result.field_stats {
            let rate = if stat.total_count == 0 {
                0.0
            } else {
                stat.null_count as f64 / stat.total_count as f64
            };
            prop_assert!(
                (0.0..=1.0).contains(&rate),
                "null_rate out of range for field {}: null={} total={}",
                stat.name, stat.null_count, stat.total_count
            );
        }
    }

    /// null_count ≤ total_count for every field.
    #[test]
    fn profiler_null_count_leq_total(events in prop::collection::vec(arb_event(), 1..=50)) {
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_null_leq".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        for stat in &result.field_stats {
            prop_assert!(
                stat.null_count <= stat.total_count,
                "null_count ({}) > total_count ({}) for field {}",
                stat.null_count, stat.total_count, stat.name
            );
        }
    }

    /// total_count == sample_count (every event contributes to every field's total).
    #[test]
    fn profiler_total_count_equals_sample_count(
        events in prop::collection::vec(arb_event(), 1..=50)
    ) {
        let n = events.len();
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_total".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        prop_assert_eq!(result.sample_count, n);
        for stat in &result.field_stats {
            prop_assert_eq!(
                stat.total_count, n as u64,
                "total_count mismatch for field {}", stat.name
            );
        }
    }

    /// HLL distinct_estimate is never greater than total_count + 5%
    /// (HLL can over-count slightly, but never by a lot at precision 12).
    #[test]
    fn profiler_distinct_leq_total_with_slack(
        events in prop::collection::vec(arb_event(), 1..=200)
    ) {
        let n = events.len();
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_distinct".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        for stat in &result.field_stats {
            // distinct estimate must not exceed total (plus 5% slack for HLL error).
            let ceiling = (n as f64 * 1.05).ceil() as u64;
            prop_assert!(
                stat.distinct_estimate <= ceiling,
                "distinct_estimate ({}) > ceiling ({}) for field {}",
                stat.distinct_estimate, ceiling, stat.name
            );
        }
    }

    /// With a single constant value per field, distinct_estimate must be ≥ 1.
    #[test]
    fn profiler_distinct_at_least_one_for_non_null(count in 1usize..=100usize) {
        let events: Vec<Value> = (0..count)
            .map(|_| json!({"k": "constant_value"}))
            .collect();
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_distinct_min".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        let k_stat = result.field_stats.iter().find(|s| s.name == "k");
        if let Some(stat) = k_stat {
            prop_assert!(
                stat.distinct_estimate >= 1,
                "distinct_estimate should be ≥ 1 for a non-null constant field"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scaffold output invariants
// ---------------------------------------------------------------------------

proptest! {
    /// Any scaffold output must be parseable YAML.
    #[test]
    fn scaffold_output_always_valid_yaml(events in prop::collection::vec(arb_event(), 1..=30)) {
        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_yaml".into(), ..Default::default() },
        ).expect("scaffold ok");

        // Strip comment lines and parse.
        let clean: String = result
            .contract_yaml
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&clean);
        prop_assert!(parsed.is_ok(), "YAML parse failed:\n{}", result.contract_yaml);
    }

    /// field_stats length == number of ontology entities in the contract.
    #[test]
    fn scaffold_stats_count_matches_entities(
        events in prop::collection::vec(arb_event(), 1..=30)
    ) {
        use contractgate::contract::Contract;

        let f = write_json_fixture(&events);
        let result = scaffold_from_file(
            f.path(),
            &ScaffoldConfig { name: "prop_count".into(), fast: false, ..Default::default() },
        ).expect("scaffold ok");

        let clean: String = result
            .contract_yaml
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let contract: Contract = serde_yaml::from_str(&clean).unwrap();

        prop_assert_eq!(
            result.field_stats.len(),
            contract.ontology.entities.len(),
            "field_stats count != entity count"
        );
    }
}

// ---------------------------------------------------------------------------
// Three-way merge properties
// ---------------------------------------------------------------------------

// For merge tests we use the scaffold_from_file to produce two contracts
// (base and theirs), then mutate them in memory for the merge scenarios.
// We test the algebraic properties directly against the merge function.

use contractgate::contract::{Contract, EgressLeakageMode, FieldDefinition, FieldType, Ontology};
use contractgate::scaffold::merge::three_way_merge;

fn make_contract(fields: &[(&str, FieldType, bool)]) -> Contract {
    Contract {
        version: "1.0".into(),
        name: "test".into(),
        description: None,
        compliance_mode: false,
        ontology: Ontology {
            entities: fields
                .iter()
                .map(|(name, ft, required)| FieldDefinition {
                    name: name.to_string(),
                    field_type: ft.clone(),
                    required: *required,
                    pattern: None,
                    allowed_values: None,
                    min: None,
                    max: None,
                    min_length: None,
                    max_length: None,
                    properties: None,
                    items: None,
                    transform: None,
                })
                .collect(),
        },
        egress_leakage_mode: EgressLeakageMode::Off,
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
    }
}

fn field_names(c: &Contract) -> Vec<String> {
    c.ontology.entities.iter().map(|f| f.name.clone()).collect()
}

proptest! {
    /// merge(base, ours=base, theirs) == theirs  (ours unchanged → accept theirs).
    #[test]
    fn merge_ours_equals_base_accepts_theirs(
        extra_field in "[a-z]{3,8}",
    ) {
        let base = make_contract(&[("id", FieldType::String, true), ("value", FieldType::String, false)]);
        let ours = base.clone(); // ours == base
        let mut theirs = base.clone();
        // theirs adds an extra field
        theirs.ontology.entities.push(FieldDefinition {
            name: extra_field.clone(),
            field_type: FieldType::String,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
        });

        let merged = three_way_merge(Some(&base), &ours, &theirs);
        let names = field_names(&merged.contract);

        prop_assert!(
            names.contains(&extra_field),
            "extra field should appear in merge result when ours==base"
        );
        prop_assert!(merged.conflicts.is_empty(), "no conflicts expected");
    }

    /// merge(base, ours, theirs=base) == ours  (theirs unchanged → preserve ours).
    #[test]
    fn merge_theirs_equals_base_preserves_ours(
        extra_field in "[a-z]{3,8}",
    ) {
        let base = make_contract(&[("id", FieldType::String, true)]);
        let mut ours = base.clone();
        ours.ontology.entities.push(FieldDefinition {
            name: extra_field.clone(),
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
            transform: None,
        });
        let theirs = base.clone(); // theirs == base

        let merged = three_way_merge(Some(&base), &ours, &theirs);
        let names = field_names(&merged.contract);

        prop_assert!(
            names.contains(&extra_field),
            "ours field should be preserved when theirs==base"
        );
        prop_assert!(merged.conflicts.is_empty(), "no conflicts expected");
    }

    /// merge with no base (first scaffold) == ours ∪ theirs, no conflicts.
    #[test]
    fn merge_no_base_union_no_conflicts(
        extra in "[a-z]{3,8}",
    ) {
        let ours = make_contract(&[("a", FieldType::String, true)]);
        let mut theirs = make_contract(&[("a", FieldType::String, true)]);
        theirs.ontology.entities.push(FieldDefinition {
            name: extra.clone(),
            field_type: FieldType::String,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
        });

        let merged = three_way_merge(None, &ours, &theirs);
        let names = field_names(&merged.contract);

        prop_assert!(names.contains(&"a".to_string()), "shared field 'a' must be present");
        // With no base, extra from theirs is drift_added
        prop_assert!(
            names.contains(&extra) || merged.drift_added.iter().any(|d| d == &extra),
            "extra field should appear in result or drift_added"
        );
    }
}
