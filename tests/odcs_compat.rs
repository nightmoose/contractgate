//! ODCS v3.x documents work with `validate` and `synth` without an
//! `ontology` block (bug report from the Bitol community Slack).

use contractgate::cli::{
    commands::validate::{run, ValidateArgs},
    config::CliConfig,
};
use contractgate::contract::{Contract, FieldType};
use contractgate::odcs::{
    normalize_contract_yaml, parse_contract_yaml, parse_contract_yaml_with_notes,
};
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::{validate, CompiledContract};

const MINIMAL: &str = include_str!("fixtures/odcs/minimal-no-ontology.odcs.yaml");
const FULL: &str = include_str!("fixtures/odcs/full-example.odcs.yaml");

fn opts(mode: SynthMode) -> SynthOptions {
    SynthOptions {
        rows: 25,
        mode,
        seed: 7,
        now_unix: 1_780_000_000,
        hints: Default::default(),
    }
}

/// Every valid row must pass the validator; every invalid row must fail.
fn assert_synth_round_trips(yaml: &str) -> usize {
    let compiled = synth::compile_yaml(yaml).expect("compiles");
    let out = synth::generate(&compiled, &opts(SynthMode::Valid)).expect("valid rows");
    assert_eq!(out.rows.len(), 25);
    for row in &out.rows {
        let r = validate(&compiled, row);
        assert!(r.passed, "row {row} failed: {:?}", r.violations);
    }
    let bad = synth::generate(&compiled, &opts(SynthMode::Invalid)).expect("invalid rows");
    assert!(!bad.invalid_rows.is_empty());
    for r in bad
        .invalid_rows
        .iter()
        .filter(|r| r.broken_rule.rule != "uniqueness")
    {
        // (uniqueness is a batch rule; a single row cannot show it)
        assert!(
            !validate(&compiled, &r.record).passed,
            "{:?}",
            r.broken_rule
        );
    }
    out.rows.len()
}

#[test]
fn minimal_odcs_without_ontology_parses_and_synths() {
    let c = parse_contract_yaml(MINIMAL).expect("parses");
    assert_eq!(c.name, "orders");
    assert_eq!(c.version, "1.0.0");
    assert_eq!(c.description.as_deref(), Some("Minimal orders contract"));
    let names: Vec<_> = c
        .ontology
        .entities
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["order_id", "amount", "status"]);
    assert!(c.ontology.entities[0].required);
    assert_eq!(c.ontology.entities[1].field_type, FieldType::Float);
    assert_eq!(c.ontology.entities[1].min, Some(0.0));
    assert!(c.ontology.entities[2].allowed_values.is_some());
    assert_synth_round_trips(MINIMAL);
}

#[test]
fn official_full_example_parses_and_synths() {
    let (c, notes) = parse_contract_yaml_with_notes(FULL).expect("parses");
    // description is a map (purpose/limitations/usage) in ODCS 3.x.
    assert_eq!(
        c.description.as_deref(),
        Some("Views built on top of the seller tables.")
    );
    // schema[0] only; the second object is reported, not silently dropped.
    // It also notes the relationships it does not enforce.
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].contains("receivers"), "{notes:?}");
    assert!(notes[1].contains("relationships"), "{notes:?}");
    let cc = c
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "rcvr_cntry_code")
        .expect("enum field");
    assert_eq!(cc.allowed_values.as_ref().map(Vec::len), Some(3));
    assert_eq!(
        c.ontology
            .entities
            .iter()
            .find(|f| f.name == "transaction_reference_date")
            .map(|f| &f.field_type),
        Some(&FieldType::Date)
    );
    // primaryKey implies required.
    assert!(
        c.ontology
            .entities
            .iter()
            .find(|f| f.name == "rcvr_id")
            .unwrap()
            .required
    );
    assert_synth_round_trips(FULL);
}

#[test]
fn native_yaml_is_returned_unchanged() {
    let native = include_str!("../examples/contracts/api/user_events.yaml");
    let (out, notes) = normalize_contract_yaml(native).unwrap();
    assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
    assert!(notes.is_empty());
}

#[test]
fn native_contract_without_ontology_still_parses() {
    let c: Contract =
        serde_yaml::from_str("version: '1.0'\nname: bare\n").expect("ontology optional");
    assert!(c.ontology.entities.is_empty());
}

#[test]
fn native_description_may_be_a_map() {
    let c: Contract = serde_yaml::from_str(
        "version: '1.0'\nname: d\ndescription:\n  purpose: why\n  usage: how\nontology:\n  entities: []\n",
    )
    .unwrap();
    assert_eq!(c.description.as_deref(), Some("why"));
}

#[test]
fn unknown_odcs_fields_are_ignored() {
    let y = r#"
apiVersion: v3.1.0
kind: DataContract
version: 2
name: tolerant
someFutureKey: {a: 1}
team: {name: x}
schema:
  - name: t
    futureThing: true
    properties:
      - name: id
        logicalType: integer
        primaryKey: true
        brandNew: [1, 2]
      - name: attrs
        logicalType: map
        map: {key: {logicalType: string}, value: {logicalType: string}}
      - name: price
        physicalType: DECIMAL(10,2)
      - name: label
        physicalType: varchar(20)
"#;
    let c = parse_contract_yaml(y).expect("parses");
    assert_eq!(c.version, "2");
    let t = |n: &str| {
        c.ontology
            .entities
            .iter()
            .find(|f| f.name == n)
            .unwrap()
            .field_type
            .clone()
    };
    assert_eq!(t("id"), FieldType::Integer);
    assert_eq!(t("attrs"), FieldType::Object);
    assert_eq!(t("price"), FieldType::Float);
    assert_eq!(t("label"), FieldType::String);
    // A single-column primary key is unique.
    assert!(c
        .quality
        .iter()
        .any(|q| q.field == "id" && format!("{:?}", q.rule_type) == "Uniqueness"));
    assert_synth_round_trips(y);
}

#[test]
fn unsupported_documents_get_clear_errors() {
    let err =
        parse_contract_yaml("apiVersion: v2.2.2\nkind: DataContract\nversion: 1\nschema: []\n")
            .unwrap_err();
    assert!(err.contains("unsupported ODCS apiVersion"), "{err}");

    let err = parse_contract_yaml(
        "dataContractSpecification: 1.2.0\nid: x\ninfo: {title: t, version: 1.0.0}\nmodels: {}\n",
    )
    .unwrap_err();
    assert!(err.contains("Data Contract Specification"), "{err}");

    let err =
        parse_contract_yaml("apiVersion: v3.1.0\nkind: DataContract\nname: n\nversion: 1.0.0\n")
            .unwrap_err();
    assert!(err.contains("no 'schema' array"), "{err}");

    let err = parse_contract_yaml(
        "apiVersion: v3.1.0\nkind: DataContract\nname: n\nversion: 1.0.0\nschema:\n  - name: t\n",
    )
    .unwrap_err();
    assert!(err.contains("no properties"), "{err}");

    let err = parse_contract_yaml(
        "apiVersion: v3.1.0\nkind: DataContract\nname: n\nschema:\n  - name: t\n    properties:\n      - name: a\n",
    )
    .unwrap_err();
    assert!(err.contains("'version'"), "{err}");
}

#[test]
fn validate_command_accepts_odcs_directory() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("datacontract.yaml"), MINIMAL).unwrap();
    std::fs::write(tmp.path().join("full.yaml"), FULL).unwrap();
    let args = ValidateArgs {
        dir: Some(tmp.path().to_path_buf()),
        json: false,
        schema_object: None,
    };
    assert_eq!(run(&args, &CliConfig::default()).unwrap(), 0);
}

#[test]
fn compiled_odcs_contract_validates_events() {
    let c = parse_contract_yaml(MINIMAL).unwrap();
    let compiled = CompiledContract::compile(c).unwrap();
    let good = serde_json::json!({"order_id": "a", "amount": 3.5, "status": "open"});
    assert!(validate(&compiled, &good).passed);
    let bad = serde_json::json!({"order_id": "a", "amount": -1, "status": "weird"});
    assert!(!validate(&compiled, &bad).passed);
}

#[test]
fn json_style_odcs_is_detected() {
    let y = r#"{"apiVersion":"v3.1.0","kind":"DataContract","version":"1.0.0","name":"j",
        "schema":[{"name":"t","properties":[{"name":"id","logicalType":"integer","required":true}]}]}"#;
    let c = parse_contract_yaml(y).expect("parses");
    assert_eq!(c.ontology.entities.len(), 1);
}

#[test]
fn official_full_example_second_object_by_name() {
    let (c, notes) =
        contractgate::odcs::parse_contract_yaml_with(FULL, Some("receivers")).expect("parses");
    assert!(
        notes.iter().all(|n| !n.contains("objects")),
        "explicit schema_object suppresses the multi-object note: {notes:?}"
    );
    let (_first, default_notes) =
        contractgate::odcs::parse_contract_yaml_with(FULL, None).expect("parses");
    assert!(
        default_notes
            .iter()
            .any(|n| n.contains("ignored: receivers")),
        "{default_notes:?}"
    );
    assert!(!c.ontology.entities.is_empty());
    assert!(c
        .ontology
        .entities
        .iter()
        .all(|f| f.name != "rcvr_cntry_code"));
}
