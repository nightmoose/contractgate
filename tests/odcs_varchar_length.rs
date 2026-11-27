//! `physicalType: varchar(n)` must become a max-length rule, same as
//! `logicalTypeOptions.maxLength`. Repro from the official ODCS v3.2.0 full
//! example (`bitol-io/open-data-contract-standard`): `tbl.rcvr_id` is
//! `physicalType: varchar(18)` with no `logicalTypeOptions`, and before this
//! fix `synth` produced 36-char UUIDs for it and `test` still passed them.

use contractgate::odcs::parse_contract_yaml_with;
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::validate;

const FULL: &str = include_str!("fixtures/odcs/full-example.odcs.yaml");

fn opts(seed: u64) -> SynthOptions {
    SynthOptions {
        rows: 100,
        mode: SynthMode::Valid,
        seed,
        now_unix: 1_780_000_000,
        hints: Default::default(),
    }
}

#[test]
fn varchar_n_sets_max_length() {
    let (contract, _) = parse_contract_yaml_with(FULL, None).expect("parses");
    let rcvr_id = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "rcvr_id")
        .expect("rcvr_id field");
    assert_eq!(
        rcvr_id.max_length,
        Some(18),
        "physicalType: varchar(18) must set max_length"
    );
}

#[test]
fn synth_never_produces_rcvr_id_longer_than_varchar_18() {
    let compiled = synth::compile_yaml(FULL).expect("compiles");
    for seed in [1u64, 2, 7, 42] {
        let out = synth::generate(&compiled, &opts(seed)).expect("generates");
        for row in &out.rows {
            if let Some(v) = row.get("rcvr_id").and_then(|v| v.as_str()) {
                assert!(
                    v.len() <= 18,
                    "seed {seed}: rcvr_id {v:?} is {} chars, varchar(18) allows at most 18 \
                     (36-char UUIDs were the reported bug)",
                    v.len()
                );
            }
        }
    }
}

#[test]
fn validate_rejects_rcvr_id_over_18_chars() {
    let compiled = synth::compile_yaml(FULL).expect("compiles");
    let too_long = serde_json::json!({
        "rcvr_id": "123456789012345678901234567890123456",
        "rcvr_cntry_code": "US",
    });
    let result = validate(&compiled, &too_long);
    assert!(
        !result.passed,
        "a 36-char rcvr_id must fail varchar(18): {:?}",
        result.violations
    );
    assert!(
        result.violations.iter().any(|v| v.field == "rcvr_id"
            && v.kind == contractgate::validation::ViolationKind::LengthViolation),
        "{:?}",
        result.violations
    );
}

#[test]
fn logical_type_options_max_length_still_wins_over_physical_type() {
    const CONTRACT: &str = r#"
apiVersion: v3.2.0
kind: DataContract
version: "1.0.0"
id: mixed-length
name: mixed_length
status: draft
schema:
- name: t
  physicalType: table
  logicalType: object
  physicalName: t
  properties:
  - name: code
    logicalType: string
    physicalType: varchar(50)
    logicalTypeOptions:
      maxLength: 5
"#;
    let (contract, _) = parse_contract_yaml_with(CONTRACT, None).expect("parses");
    let code = contract
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "code")
        .expect("code field");
    assert_eq!(
        code.max_length,
        Some(5),
        "an explicit logicalTypeOptions.maxLength must win over the physicalType length"
    );
}
