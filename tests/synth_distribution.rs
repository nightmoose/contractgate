//! Per-field `distribution` hint for `synth` (integer and number fields).

use contractgate::contract::{Contract, Distribution};
use contractgate::odcs::{normalize_contract_yaml, parse_contract_yaml};
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::validate;
use serde_json::Value;

const ROWS: usize = 5000;

fn contract(field_yaml: &str) -> String {
    format!("version: '1.0'\nname: dist\nontology:\n  entities:\n    - name: x\n      type: number\n{field_yaml}")
}

fn opts(rows: usize, seed: u64) -> SynthOptions {
    SynthOptions {
        rows,
        mode: SynthMode::Valid,
        seed,
        now_unix: 1_780_000_000,
    }
}

fn draw(yaml: &str, rows: usize, seed: u64) -> Vec<f64> {
    let compiled = synth::compile_yaml(yaml).expect("compiles");
    let out = synth::generate(&compiled, &opts(rows, seed)).expect("generates");
    out.rows
        .iter()
        .map(|r| r["x"].as_f64().expect("numeric x"))
        .collect()
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, var.sqrt())
}

#[test]
fn normal_matches_mean_and_sd() {
    let y = contract(
        "      min: 0\n      max: 200\n      distribution: {type: normal, mean: 100, sd: 15}\n",
    );
    let v = draw(&y, ROWS, 11);
    let (m, sd) = mean_sd(&v);
    assert!((m - 100.0).abs() < 0.8, "mean {m}");
    assert!((sd - 15.0).abs() < 0.8, "sd {sd}");
    assert!(v.iter().all(|x| (0.0..=200.0).contains(x)));
}

#[test]
fn normal_on_integers_stays_integer() {
    let y = "version: '1.0'\nname: d\nontology:\n  entities:\n    - name: x\n      type: integer\n      distribution: {type: normal, mean: 1000, sd: 50}\n";
    let compiled = synth::compile_yaml(y).unwrap();
    let out = synth::generate(&compiled, &opts(ROWS, 3)).unwrap();
    let v: Vec<f64> = out
        .rows
        .iter()
        .map(|r| {
            assert!(r["x"].is_i64(), "{}", r["x"]);
            r["x"].as_f64().unwrap()
        })
        .collect();
    let (m, sd) = mean_sd(&v);
    assert!((m - 1000.0).abs() < 3.0, "mean {m}");
    assert!((sd - 50.0).abs() < 3.0, "sd {sd}");
}

#[test]
fn lognormal_parameters_describe_the_underlying_normal() {
    let y = contract("      distribution: {type: lognormal, mean: 3, sd: 0.5}\n");
    let v = draw(&y, ROWS, 5);
    assert!(v.iter().all(|x| *x > 0.0));
    let logs: Vec<f64> = v.iter().map(|x| x.ln()).collect();
    let (m, sd) = mean_sd(&logs);
    assert!((m - 3.0).abs() < 0.05, "log mean {m}");
    assert!((sd - 0.5).abs() < 0.05, "log sd {sd}");
    // Right-skewed: the mean sits above the median.
    let mut sorted = v.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(mean_sd(&v).0 > sorted[ROWS / 2]);
}

#[test]
fn exponential_has_mean_one_over_rate_above_min() {
    let y = contract("      min: 5\n      distribution: {type: exponential, rate: 0.1}\n");
    let v = draw(&y, ROWS, 9);
    assert!(v.iter().all(|x| *x >= 5.0));
    let (m, _) = mean_sd(&v);
    assert!((m - 15.0).abs() < 0.7, "mean {m}");
}

#[test]
fn bounds_truncate_by_redrawing() {
    let y = contract(
        "      min: 45\n      max: 55\n      distribution: {type: normal, mean: 50, sd: 5}\n",
    );
    let v = draw(&y, ROWS, 2);
    assert!(v.iter().all(|x| (45.0..=55.0).contains(x)));
    // Truncated bell, not flat: the middle 4 units hold far more than the outer 2.
    let mid = v.iter().filter(|x| **x >= 48.0 && **x <= 52.0).count();
    let edge = v.iter().filter(|x| **x < 46.0 || **x > 54.0).count();
    assert!(mid > 2 * edge, "mid {mid} edge {edge}");
}

#[test]
fn deterministic_per_seed() {
    let y = contract("      distribution: {type: normal, mean: 0, sd: 1}\n");
    assert_eq!(draw(&y, 200, 42), draw(&y, 200, 42));
    assert_ne!(draw(&y, 200, 42), draw(&y, 200, 43));
}

#[test]
fn uniform_hint_leaves_output_unchanged() {
    let base = contract("      min: 1\n      max: 9\n");
    let hinted = contract("      min: 1\n      max: 9\n      distribution: uniform\n");
    assert_eq!(draw(&base, 300, 8), draw(&hinted, 300, 8));
}

#[test]
fn rows_pass_the_validator_and_the_hint_is_ignored_by_it() {
    let y = contract(
        "      min: 0\n      max: 10\n      distribution: {type: normal, mean: 5, sd: 2}\n",
    );
    let compiled = synth::compile_yaml(&y).unwrap();
    let out = synth::generate(&compiled, &opts(500, 1)).unwrap();
    for r in &out.rows {
        assert!(validate(&compiled, r).passed);
    }
    // The validator takes the same events with or without the hint.
    let plain = synth::compile_yaml(&contract("      min: 0\n      max: 10\n")).unwrap();
    for ev in [
        serde_json::json!({"x": 5.0}),
        serde_json::json!({"x": 11}),
        serde_json::json!({"x": -1}),
    ] {
        assert_eq!(
            validate(&compiled, &ev).passed,
            validate(&plain, &ev).passed
        );
    }
}

#[test]
fn contracts_without_a_hint_serialize_without_one() {
    let c: Contract = serde_yaml::from_str(&contract("      min: 0\n")).unwrap();
    assert!(c.ontology.entities[0].distribution.is_none());
    assert!(!serde_yaml::to_string(&c).unwrap().contains("distribution"));
}

#[test]
fn invalid_hints_get_clear_errors() {
    let err = |field: &str| {
        serde_yaml::from_str::<Contract>(&contract(field))
            .unwrap_err()
            .to_string()
    };
    assert!(
        err("      distribution: {type: normal, mean: 1, sd: 0}\n").contains("sd greater than 0")
    );
    assert!(err("      distribution: {type: normal, mean: 1}\n").contains("needs a numeric `sd`"));
    assert!(
        err("      distribution: {type: exponential, rate: -1}\n").contains("rate greater than 0")
    );
    assert!(err("      distribution: {type: weibull}\n").contains("unknown distribution type"));
    assert!(
        err("      distribution: {type: normal, mean: 1, sd: 1, rate: 2}\n")
            .contains("does not take `rate`")
    );
    assert!(err("      distribution: 7\n").contains("distribution must be"));
}

#[test]
fn synth_errors_name_the_field() {
    let on_string = "version: '1.0'\nname: d\nontology:\n  entities:\n    - name: s\n      type: string\n      distribution: {type: normal, mean: 1, sd: 1}\n";
    let c = synth::compile_yaml(on_string).unwrap();
    let e = synth::generate(&c, &opts(2, 1)).unwrap_err();
    assert_eq!(e.rule, "distribution");
    assert!(
        e.message.contains("field 's'") && e.message.contains("integer and number"),
        "{}",
        e.message
    );

    let y = contract("      max: 10\n      distribution: {type: normal, mean: 1000, sd: 1}\n");
    let c = synth::compile_yaml(&y).unwrap();
    let e = synth::generate(&c, &opts(2, 1)).unwrap_err();
    assert_eq!(e.rule, "distribution");
    assert!(e.message.contains("almost no values"), "{}", e.message);
}

#[test]
fn enum_wins_over_the_hint() {
    let y =
        contract("      enum: [1, 2, 3]\n      distribution: {type: normal, mean: 100, sd: 1}\n");
    assert!(draw(&y, 100, 1).iter().all(|x| [1.0, 2.0, 3.0].contains(x)));
}

const ODCS: &str = r#"
apiVersion: v3.1.0
kind: DataContract
version: 1.0.0
name: odcs_dist
schema:
  - name: t
    properties:
      - name: x
        logicalType: number
        required: true
        customProperties:
          - property: x-cg-distribution
            value: {type: normal, mean: 50, sd: 5}
"#;

#[test]
fn odcs_custom_property_sets_the_hint_and_round_trips() {
    let c = parse_contract_yaml(ODCS).unwrap();
    assert_eq!(
        c.ontology.entities[0].distribution,
        Some(Distribution::Normal {
            mean: 50.0,
            sd: 5.0
        })
    );
    let v = draw(ODCS, ROWS, 4);
    let (m, sd) = mean_sd(&v);
    assert!((m - 50.0).abs() < 0.5 && (sd - 5.0).abs() < 0.4, "{m} {sd}");
    // Serialized native YAML keeps it.
    let (native, _) = normalize_contract_yaml(ODCS).unwrap();
    assert!(native.contains("distribution"));
    let again: Contract = serde_yaml::from_str(&native).unwrap();
    assert_eq!(
        again.ontology.entities[0].distribution,
        c.ontology.entities[0].distribution
    );
}

#[test]
fn odcs_bad_hint_is_an_error() {
    let bad = ODCS.replace("sd: 5", "sd: -5");
    let e = parse_contract_yaml(&bad).unwrap_err();
    assert!(
        e.contains("property 'x'") && e.contains("sd greater than 0"),
        "{e}"
    );
}

#[test]
fn json_value_form_parses() {
    let v: Value = serde_json::json!({"type": "lognormal", "mean": 1.0, "sd": 0.2});
    let d: Distribution = serde_json::from_value(v).unwrap();
    assert_eq!(d, Distribution::Lognormal { mean: 1.0, sd: 0.2 });
}
