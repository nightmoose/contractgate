//! Bare `minimum`/`maximum` directly on an ODCS property are not part of
//! the official ODCS v3.x schema (`SchemaBaseProperty` has
//! `unevaluatedProperties: false` and does not list `minimum` / `maximum`;
//! only the open `logicalTypeOptions` object carries that kind of bound).
//! This importer still ignores them (matching the spec), but must warn so
//! the silent-ignore isn't a trap. `logicalTypeOptions.minimum` /
//! `.maximum` are the documented, honored mapping and must not warn.

use contractgate::odcs::parse_contract_yaml_with;
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::validate;

const BARE: &str = r#"
apiVersion: v3.2.0
kind: DataContract
version: "1.0.0"
id: bare-bounds
name: bare_bounds
status: draft
schema:
- name: t
  physicalType: table
  logicalType: object
  physicalName: t
  properties:
  - name: rate
    physicalType: DOUBLE
    logicalType: number
    minimum: 0
    maximum: 10
"#;

const LOGICAL_OPTIONS: &str = r#"
apiVersion: v3.2.0
kind: DataContract
version: "1.0.0"
id: logical-bounds
name: logical_bounds
status: draft
schema:
- name: t
  physicalType: table
  logicalType: object
  physicalName: t
  properties:
  - name: rate
    physicalType: DOUBLE
    logicalType: number
    logicalTypeOptions:
      minimum: 0
      maximum: 10
"#;

#[test]
fn bare_minimum_maximum_are_ignored_but_warned_about() {
    let (contract, notes) = parse_contract_yaml_with(BARE, None).expect("parses");
    let field = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "rate")
        .expect("rate field");
    assert_eq!(
        field.min, None,
        "bare 'minimum' is not part of the ODCS v3.x schema and must not be honored"
    );
    assert_eq!(field.max, None);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("minimum") && n.contains("logicalTypeOptions")),
        "expected a note about bare minimum/maximum, got: {notes:?}"
    );

    // Confirmed unbounded: synth can produce a value far outside 0..10 and
    // the contract still considers it valid, since the bound was never
    // turned into a rule.
    let compiled = synth::compile_yaml(BARE).unwrap();
    let wide = serde_json::json!({"rate": 999.0});
    assert!(
        validate(&compiled, &wide).passed,
        "{:?}",
        validate(&compiled, &wide).violations
    );
}

#[test]
fn logical_type_options_minimum_maximum_are_honored_without_a_warning() {
    let (contract, notes) = parse_contract_yaml_with(LOGICAL_OPTIONS, None).expect("parses");
    let field = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "rate")
        .expect("rate field");
    assert_eq!(field.min, Some(0.0));
    assert_eq!(field.max, Some(10.0));
    assert!(
        !notes
            .iter()
            .any(|n| n.contains("minimum") || n.contains("maximum")),
        "logicalTypeOptions bounds are the documented mapping and should not warn: {notes:?}"
    );

    let compiled = synth::compile_yaml(LOGICAL_OPTIONS).unwrap();
    let out_of_range = serde_json::json!({"rate": 999.0});
    assert!(!validate(&compiled, &out_of_range).passed);

    let in_range = synth::generate(
        &compiled,
        &SynthOptions {
            rows: 25,
            mode: SynthMode::Valid,
            seed: 3,
            now_unix: 1_780_000_000,
            hints: Default::default(),
        },
    )
    .unwrap();
    for row in &in_range.rows {
        if let Some(v) = row.get("rate").and_then(|v| v.as_f64()) {
            assert!((0.0..=10.0).contains(&v), "{row}");
        }
    }
}

#[test]
fn bare_bounds_on_a_nested_object_property_are_also_warned_about() {
    const NESTED: &str = r#"
apiVersion: v3.2.0
kind: DataContract
version: "1.0.0"
id: nested-bare-bounds
name: nested_bare_bounds
status: draft
schema:
- name: t
  physicalType: table
  logicalType: object
  physicalName: t
  properties:
  - name: shipping
    logicalType: object
    properties:
    - name: weight
      logicalType: number
      minimum: 0
      maximum: 100
"#;
    let (_, notes) = parse_contract_yaml_with(NESTED, None).expect("parses");
    assert!(
        notes.iter().any(|n| n.contains("weight")),
        "nested property bare bounds should be named in the warning: {notes:?}"
    );
}
