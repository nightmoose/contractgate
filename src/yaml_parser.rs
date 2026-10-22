//! Parser behavior the gateway relies on when it reads contract YAML.
//!
//! These checks lock the current parser: scalar anchors expand, mapping
//! keys keep their YAML types, and a broken document reports the same
//! location text callers surface to clients.

use crate::contract::{Contract, FieldType};

const USER_EVENTS: &str = include_str!("../examples/contracts/api/user_events.yaml");

#[test]
fn example_contract_round_trips() {
    let original: Contract = serde_yaml::from_str(USER_EVENTS).expect("example contract parses");
    let encoded = serde_yaml::to_string(&original).expect("contract encodes");
    let again: Contract = serde_yaml::from_str(&encoded).expect("encoded contract parses");

    assert_eq!(original.name, again.name);
    assert_eq!(original.version, again.version);
    assert_eq!(
        original.ontology.entities.len(),
        again.ontology.entities.len()
    );
    for (before, after) in original
        .ontology
        .entities
        .iter()
        .zip(again.ontology.entities.iter())
    {
        assert_eq!(before.name, after.name);
        assert_eq!(before.field_type, after.field_type);
        assert_eq!(before.required, after.required);
    }
    assert_eq!(again.ontology.entities[0].name, "user_id");
    assert_eq!(again.ontology.entities[0].field_type, FieldType::String);
    assert!(again.ontology.entities[0].required);
}

#[test]
fn scalar_anchors_expand_to_the_same_value() {
    let value: serde_yaml::Value =
        serde_yaml::from_str("a: &id user_id\nb: *id\n").expect("anchor parses");
    assert_eq!(value["a"].as_str(), Some("user_id"));
    assert_eq!(value["b"].as_str(), Some("user_id"));
}

#[test]
fn mapping_keys_keep_non_string_types() {
    let value: serde_yaml::Value =
        serde_yaml::from_str("? 1\n: one\n? true\n: boolkey\n").expect("explicit keys parse");
    let map = value.as_mapping().expect("mapping");
    let number = serde_yaml::Value::Number(1u64.into());
    let boolean = serde_yaml::Value::Bool(true);
    assert_eq!(
        map.get(&number).and_then(serde_yaml::Value::as_str),
        Some("one")
    );
    assert_eq!(
        map.get(&boolean).and_then(serde_yaml::Value::as_str),
        Some("boolkey")
    );
}

#[test]
fn broken_document_names_the_line_and_column() {
    let err = serde_yaml::from_str::<serde_yaml::Value>("version: [\n  - 1\nname: x\n")
        .expect_err("broken flow node");
    let message = err.to_string();
    assert!(
        message.contains("did not find expected node content"),
        "{message}"
    );
    assert!(message.contains("line 2 column 3"), "{message}");
}

#[test]
fn duplicate_key_is_rejected() {
    let err = serde_yaml::from_str::<serde_yaml::Value>("a: 1\na: 2\n").expect_err("duplicate key");
    let message = err.to_string();
    assert!(
        message.contains("duplicate entry with key \"a\""),
        "{message}"
    );
}
