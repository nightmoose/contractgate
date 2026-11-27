//! ODCS property names are literal columns. Dots and brackets are not paths.
//!
//! `Rate.x` must not become an object `Rate` with a child `x` (that produced
//! an extra CSV column `Rate`, sometimes filled with `{}`). `Amount [A]` is
//! one column. Native contracts still walk real nested objects by dot path.

use contractgate::cli::commands::synth::{self, Format, SynthArgs};
use contractgate::synth::{self as gen, SynthMode, SynthOptions};
use contractgate::validation::{resolve_indexed, validate, CompiledContract};
use serde_json::json;
use std::path::PathBuf;

const CONTRACT: &str = r#"
version: 1.0.0
kind: DataContract
apiVersion: v3.2.0
id: my-data-contract
name: My Data Contract
status: draft
servers:
  - server: "local-duckdb"
    type: "duckdb"
    database: "./colname_bug.duckdb"
    schema: "main"
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

fn opts(rows: usize) -> SynthOptions {
    SynthOptions {
        rows,
        mode: SynthMode::Valid,
        seed: 7,
        now_unix: 1_780_000_000,
        hints: Default::default(),
    }
}

fn compiled() -> CompiledContract {
    gen::compile_yaml(CONTRACT).expect("compiles")
}

#[test]
fn import_keeps_dotted_and_bracketed_names_as_number_fields() {
    let c = &compiled().contract;
    let names: Vec<&str> = c
        .ontology
        .entities
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["Amount [A]", "Rate.x", "plain"]);
    for f in &c.ontology.entities {
        assert_eq!(
            f.field_type,
            contractgate::contract::FieldType::Float,
            "{}",
            f.name
        );
        assert!(
            !f.required,
            "ODCS properties are optional unless required: true ({})",
            f.name
        );
        assert!(f.properties.is_none(), "{}", f.name);
    }
}

#[test]
fn synth_csv_has_exactly_the_declared_columns_and_no_empty_objects() {
    let c = compiled();
    let out = gen::generate(&c, &opts(40)).expect("generates");
    let mut saw_blank = false;
    for row in &out.rows {
        assert!(validate(&c, row).passed, "{row}");
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in &keys {
            assert!(
                ["Amount [A]", "Rate.x", "plain"].contains(key),
                "unexpected key {key} in {row}"
            );
        }
        if !row.as_object().unwrap().contains_key("Rate.x") {
            saw_blank = true;
        }
        for key in ["Amount [A]", "Rate.x", "plain"] {
            if let Some(v) = row.get(key) {
                assert!(v.is_number(), "{key} must be a number, got {v} in {row}");
                assert_ne!(v.to_string(), "{}", "{row}");
            }
        }
    }
    assert!(
        saw_blank,
        "optional ODCS fields should be absent on some rows"
    );

    let text = rows_to_csv(&out.rows);
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let mut header: Vec<String> = reader
        .headers()
        .unwrap()
        .iter()
        .map(str::to_string)
        .collect();
    header.sort();
    assert_eq!(header, ["Amount [A]", "Rate.x", "plain"]);
    for rec in reader.records() {
        let rec = rec.unwrap();
        assert_eq!(rec.len(), 3);
        for cell in rec.iter() {
            assert_ne!(cell, "{}");
            if !cell.is_empty() {
                cell.parse::<f64>().expect(cell);
            }
        }
    }
}

#[test]
fn validate_accepts_literal_keys_and_native_nested_paths() {
    let c = compiled();
    let flat = json!({"Amount [A]": 1.25, "Rate.x": 2.5, "plain": 3.0});
    assert!(
        validate(&c, &flat).passed,
        "{:?}",
        validate(&c, &flat).violations
    );
    let nested = json!({"Amount [A]": 1.0, "Rate": {"x": 2.5}, "plain": 3.0});
    assert!(
        validate(&c, &nested).passed,
        "a dotted name still matches nested JSON when the literal key is absent: {:?}",
        validate(&c, &nested).violations
    );
    let object = json!({"Amount [A]": {}, "Rate.x": 1.0, "plain": 1.0});
    assert!(!validate(&c, &object).passed);

    let native = r#"
version: "1"
name: native_nested
ontology:
  entities:
    - name: customer
      type: object
      required: true
      properties:
        - name: address
          type: object
          required: true
          properties:
            - name: city
              type: string
              required: true
quality:
  - field: customer.address.city
    type: completeness
"#;
    let n = gen::compile_yaml(native).unwrap();
    let good = json!({"customer": {"address": {"city": "Boston"}}});
    assert!(
        validate(&n, &good).passed,
        "{:?}",
        validate(&n, &good).violations
    );
    assert_eq!(
        resolve_indexed(&good, "customer.address.city").and_then(|v| v.as_str()),
        Some("Boston")
    );
    let bad = json!({"customer": {"address": {}}});
    assert!(!validate(&n, &bad).passed);
    let rows = gen::generate(&n, &opts(5)).unwrap();
    for row in &rows.rows {
        assert!(row["customer"]["address"]["city"].is_string(), "{row}");
        assert!(row.get("customer.address.city").is_none(), "{row}");
    }
}

#[test]
fn cli_synth_csv_matches_the_three_columns() {
    let tmp = tempfile::tempdir().unwrap();
    let contract = tmp.path().join("datacontract.yaml");
    let out = tmp.path().join("rows.csv");
    std::fs::write(&contract, CONTRACT).unwrap();
    let code = synth::run(&SynthArgs {
        contract: contract.clone(),
        rows: 10,
        invalid: false,
        seed: Some(3),
        anchor: None,
        envelope: false,
        format: Format::Csv,
        out: Some(out.clone()),
        schema_object: None,
        synth_config: None,
    })
    .unwrap();
    assert_eq!(code, 0);
    let text = std::fs::read_to_string(&out).unwrap();
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let mut header: Vec<String> = reader
        .headers()
        .unwrap()
        .iter()
        .map(str::to_string)
        .collect();
    header.sort();
    assert_eq!(header, ["Amount [A]", "Rate.x", "plain"]);
    let mut n = 0;
    for rec in reader.records() {
        let rec = rec.unwrap();
        assert_eq!(rec.len(), 3);
        for cell in rec.iter() {
            assert_ne!(cell, "{}");
        }
        n += 1;
    }
    assert_eq!(n, 10);
}

#[test]
fn cli_version_comes_from_the_crate() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_contractgate"))
        .arg("--version")
        .output()
        .expect("run contractgate --version");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "stdout {text:?}");
    assert!(text.contains("0.4.3"), "{text:?}");
}

fn rows_to_csv(rows: &[serde_json::Value]) -> String {
    let mut records = Vec::new();
    for row in rows {
        let mut cells = Vec::new();
        contractgate::synth::flatten_record(row, "", &mut cells);
        records.push(cells);
    }
    let mut header = Vec::new();
    for cells in &records {
        for (k, _) in cells {
            if !header.contains(k) {
                header.push(k.clone());
            }
        }
    }
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(&header).unwrap();
    for cells in &records {
        let rec: Vec<&str> = header
            .iter()
            .map(|h| {
                cells
                    .iter()
                    .find(|(k, _)| k == h)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("")
            })
            .collect();
        w.write_record(&rec).unwrap();
    }
    String::from_utf8(w.into_inner().unwrap()).unwrap()
}

#[test]
fn explicit_schema_object_suppresses_the_note_on_the_cli() {
    let bin = env!("CARGO_BIN_EXE_contractgate");
    let tmp = tempfile::tempdir().unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/odcs/full-example.odcs.yaml");
    std::fs::copy(&src, tmp.path().join("full.odcs.yaml")).unwrap();
    let plain = std::process::Command::new(bin)
        .args(["validate", "--dir"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let plain_out = String::from_utf8(plain.stdout).unwrap();
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert!(plain_out.contains("ignored: receivers"), "{plain_out}");
    let picked = std::process::Command::new(bin)
        .args(["validate", "--schema-object", "tbl", "--dir"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let picked_out = String::from_utf8(picked.stdout).unwrap();
    assert!(
        picked.status.success(),
        "{}",
        String::from_utf8_lossy(&picked.stderr)
    );
    assert!(
        !picked_out.contains("objects"),
        "explicit --schema-object should suppress the note:\n{picked_out}"
    );
}
