//! ODCS documents with several `schema` objects: pick one by name, and
//! surface what did not map (ignored objects, composite key, relationships).

use contractgate::odcs::{import_odcs_with, parse_contract_yaml_with};
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::validate;

const MULTI: &str = r#"
apiVersion: v3.1.0
kind: DataContract
version: 1.0.0
dataProduct: shop
schema:
  - name: orders
    properties:
      - name: order_id
        logicalType: string
        primaryKey: true
        required: true
      - name: line_no
        logicalType: integer
        primaryKey: true
        required: true
      - name: customer_id
        logicalType: string
        relationships:
          - to: customers.customer_id
  - name: customers
    physicalName: tbl_customers
    properties:
      - name: customer_id
        logicalType: string
        required: true
      - name: age
        logicalType: integer
        logicalTypeOptions:
          minimum: 18
          maximum: 99
"#;

fn field_names(yaml: &str, obj: Option<&str>) -> Vec<String> {
    let (c, _) = parse_contract_yaml_with(yaml, obj).expect("parses");
    c.ontology.entities.into_iter().map(|f| f.name).collect()
}

#[test]
fn default_is_first_object_and_note_names_the_rest_and_the_flag() {
    assert_eq!(
        field_names(MULTI, None),
        ["order_id", "line_no", "customer_id"]
    );
    let r = import_odcs_with(MULTI, None).unwrap();
    let n = r
        .warnings
        .iter()
        .find(|w| w.contains("objects"))
        .expect("multi-object note");
    assert!(n.contains("imported 'orders'"), "{n}");
    assert!(n.contains("customers"), "{n}");
    assert!(n.contains("schema_object"), "{n}");
}

#[test]
fn schema_object_selects_by_name_physical_name_and_id() {
    assert_eq!(
        field_names(MULTI, Some("customers")),
        ["customer_id", "age"]
    );
    assert_eq!(
        field_names(MULTI, Some("tbl_customers")),
        ["customer_id", "age"]
    );
    let r = import_odcs_with(MULTI, Some("customers")).unwrap();
    let n = r.warnings.iter().find(|w| w.contains("objects")).unwrap();
    assert!(n.contains("imported 'customers' (schema[1])"), "{n}");
    assert!(n.contains("ignored: orders"), "{n}");
}

#[test]
fn unknown_schema_object_lists_the_available_names() {
    let Err(e) = import_odcs_with(MULTI, Some("nope")) else {
        panic!("expected error")
    };
    assert!(e.contains("'nope' not found"), "{e}");
    assert!(e.contains("orders, customers"), "{e}");
}

#[test]
fn composite_key_and_relationships_are_noted_not_enforced() {
    let r = import_odcs_with(MULTI, None).unwrap();
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("composite primary key (order_id, line_no)")),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings.iter().any(|w| w.contains("relationships")),
        "{:?}",
        r.warnings
    );
    // The second object has neither.
    let r2 = import_odcs_with(MULTI, Some("customers")).unwrap();
    assert!(!r2.warnings.iter().any(|w| w.contains("composite")));
    assert!(!r2.warnings.iter().any(|w| w.contains("relationships")));
}

#[test]
fn single_column_key_gets_no_composite_note() {
    let y = "apiVersion: v3.1.0\nkind: DataContract\nversion: 1.0.0\nschema:\n  - name: t\n    properties:\n      - name: id\n        logicalType: string\n        primaryKey: true\n";
    let r = import_odcs_with(y, None).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn selected_object_synths_valid_rows() {
    let (c, _) = parse_contract_yaml_with(MULTI, Some("customers")).unwrap();
    let compiled = contractgate::validation::CompiledContract::compile(c).unwrap();
    let out = synth::generate(
        &compiled,
        &SynthOptions {
            rows: 50,
            mode: SynthMode::Valid,
            seed: 1,
            now_unix: 1_780_000_000,
        },
    )
    .unwrap();
    let mut seen = 0;
    for row in &out.rows {
        assert!(validate(&compiled, row).passed, "{row}");
        if let Some(age) = row.get("age").and_then(|v| v.as_i64()) {
            assert!((18..=99).contains(&age));
            seen += 1;
        }
    }
    assert!(seen > 0, "optional field never generated");
}

#[test]
fn schema_object_is_ignored_for_native_yaml() {
    let y = "version: \"1\"\nname: t\nontology:\n  entities:\n    - name: id\n      type: string\n";
    let (c, notes) = parse_contract_yaml_with(y, Some("whatever")).unwrap();
    assert_eq!(c.name, "t");
    assert!(notes.is_empty());
}
