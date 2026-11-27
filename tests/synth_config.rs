//! `--synth-config`: per-field generation hints outside the contract.
//!
//! Config distributions override in-contract hints. Contract min/max/enum/
//! format/pattern rules still win, and every valid row has to pass them.

use contractgate::cli::commands::synth::{self, Format, SynthArgs};
use contractgate::contract::Distribution;
use contractgate::synth::{self as gen, SynthMode, SynthOptions};
use contractgate::validation::validate;

const CONTRACT: &str = r#"
version: "1.0"
name: hinted
ontology:
  entities:
    - name: "Rate.x"
      type: number
      required: true
      min: 0
      max: 100
      distribution: { type: exponential, rate: 0.5 }
    - name: sku
      type: string
      required: true
    - name: code
      type: string
      required: true
      enum: ["aa", "bb"]
"#;

const CONFIG: &str = r#"
fields:
  "Rate.x":
    distribution: { type: normal, mean: 50, sd: 5 }
  sku:
    prefix: "SKU-"
    suffix: "-A"
    pattern: "^SKU-[A-Z]{4}-A$"
  code:
    prefix: "NOPE-"
"#;

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

#[test]
fn config_overrides_distribution_and_shapes_strings_inside_contract_rules() {
    let mut compiled = gen::compile_yaml(CONTRACT).unwrap();
    assert!(matches!(
        compiled.contract.ontology.entities[0].distribution,
        Some(Distribution::Exponential { rate }) if (rate - 0.5).abs() < 1e-9
    ));
    let hints = gen::apply_synth_config(&mut compiled.contract, CONFIG).unwrap();
    assert!(hints.contains_key("sku"));
    assert!(matches!(
        compiled.contract.ontology.entities[0].distribution,
        Some(Distribution::Normal { mean, sd }) if (mean - 50.0).abs() < 1e-9 && (sd - 5.0).abs() < 1e-9
    ));
    let out = gen::generate(
        &compiled,
        &SynthOptions {
            rows: 400,
            mode: SynthMode::Valid,
            seed: 11,
            now_unix: 1_780_000_000,
            hints,
        },
    )
    .unwrap();
    let sku_re = regex::Regex::new(r"^SKU-[A-Z]{4}-A$").unwrap();
    let mut rates = Vec::new();
    for row in &out.rows {
        assert!(validate(&compiled, row).passed, "{row}");
        let rate = row["Rate.x"].as_f64().unwrap();
        assert!((0.0..=100.0).contains(&rate), "{rate}");
        rates.push(rate);
        let sku = row["sku"].as_str().unwrap();
        assert!(sku.starts_with("SKU-") && sku.ends_with("-A"), "{sku}");
        assert!(sku_re.is_match(sku), "{sku}");
        let code = row["code"].as_str().unwrap();
        assert!(
            code == "aa" || code == "bb",
            "enum wins over prefix, got {code}"
        );
    }
    let m = mean(&rates);
    assert!((m - 50.0).abs() < 2.0, "config normal mean, got {m}");
}

#[test]
fn in_contract_distribution_still_applies_without_a_config_file() {
    let compiled = gen::compile_yaml(CONTRACT).unwrap();
    let out = gen::generate(
        &compiled,
        &SynthOptions {
            rows: 400,
            mode: SynthMode::Valid,
            seed: 11,
            now_unix: 1_780_000_000,
            hints: Default::default(),
        },
    )
    .unwrap();
    let rates: Vec<f64> = out
        .rows
        .iter()
        .map(|r| r["Rate.x"].as_f64().unwrap())
        .collect();
    let m = mean(&rates);
    assert!(m < 15.0, "in-contract exponential stays near 0, got {m}");
}

#[test]
fn contract_pattern_wins_over_a_conflicting_prefix() {
    let y = r#"
version: "1"
name: conflict
ontology:
  entities:
    - name: sku
      type: string
      required: true
      pattern: "^ITEM-[0-9]{3}$"
"#;
    let mut compiled = gen::compile_yaml(y).unwrap();
    let hints = gen::apply_synth_config(
        &mut compiled.contract,
        "fields:\n  sku:\n    prefix: \"SKU-\"\n",
    )
    .unwrap();
    let err = gen::generate(
        &compiled,
        &SynthOptions {
            rows: 2,
            mode: SynthMode::Valid,
            seed: 1,
            now_unix: 1,
            hints,
        },
    )
    .unwrap_err();
    assert!(
        err.message.contains("sku") && err.message.contains("pattern"),
        "{}",
        err.message
    );
}

#[test]
fn unknown_field_and_non_string_hint_are_errors() {
    let mut compiled = gen::compile_yaml(CONTRACT).unwrap();
    let err = gen::apply_synth_config(
        &mut compiled.contract,
        "fields:\n  missing:\n    prefix: x\n",
    )
    .unwrap_err();
    assert!(err.message.contains("missing"), "{}", err.message);

    let err = gen::apply_synth_config(
        &mut compiled.contract,
        "fields:\n  \"Rate.x\":\n    prefix: n\n",
    )
    .unwrap_err();
    assert!(
        err.message.contains("Rate.x") && err.message.contains("string"),
        "{}",
        err.message
    );
}

#[test]
fn cli_synth_config_file() {
    let tmp = tempfile::tempdir().unwrap();
    let contract = tmp.path().join("contract.yaml");
    let config = tmp.path().join("synth.yaml");
    let out = tmp.path().join("rows.json");
    std::fs::write(&contract, CONTRACT).unwrap();
    std::fs::write(&config, CONFIG).unwrap();
    let code = synth::run(&SynthArgs {
        contract,
        rows: 20,
        invalid: false,
        seed: Some(4),
        anchor: None,
        envelope: false,
        format: Format::Json,
        out: Some(out.clone()),
        schema_object: None,
        synth_config: Some(config),
    })
    .unwrap();
    assert_eq!(code, 0);
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(rows.len(), 20);
    for row in &rows {
        let sku = row["sku"].as_str().unwrap();
        assert!(sku.starts_with("SKU-") && sku.ends_with("-A"), "{sku}");
        let rate = row["Rate.x"].as_f64().unwrap();
        assert!((0.0..=100.0).contains(&rate));
    }
}
