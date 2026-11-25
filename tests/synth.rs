//! Synthetic data generator (RFC-120): every valid row passes the validator,
//! and every invalid row fails exactly the rule its label names.

use contractgate::contract::Contract;
use contractgate::synth::{self, SynthMode, SynthOptions};
use contractgate::validation::{check_uniqueness_batch, validate, CompiledContract, Violation};
use serde_json::Value;
use std::path::Path;

const RICH: &str = r#"
version: "1.0"
name: synth_rich
compliance_mode: true
ontology:
  entities:
    - name: order_id
      type: string
      pattern: "^ORD-[0-9]{6}$"
    - name: customer_email
      type: string
      format: email
      max_length: 40
    - name: request_id
      type: string
      format: uuid
    - name: homepage
      type: string
      format: uri
      required: false
    - name: status
      type: string
      enum: [placed, shipped, delivered, cancelled]
    - name: quantity
      type: integer
      min: 1
      max: 500
    - name: unit_price
      type: number
      min: 0.5
      max: 999.99
    - name: discount_code
      type: string
      required: false
      min_length: 4
      max_length: 12
      pattern: "^[A-Z0-9]+$"
    - name: gift
      type: boolean
    - name: order_date
      type: date
    - name: placed_at
      type: timestamp
    - name: ingested_unix
      type: integer
    - name: shipping
      type: object
      properties:
        - name: country
          type: string
          pattern: "^[A-Z]{2}$"
        - name: postcode
          type: string
          min_length: 3
          max_length: 10
        - name: note
          type: string
          required: false
    - name: tags
      type: array
      required: false
      items:
        type: string
        name: tag
    - name: payload
      type: any
      required: false
    - name: nickname
      type: string
      required: false
quality:
  - field: order_id
    type: uniqueness
  - field: customer_email
    type: completeness
  - field: nickname
    type: completeness
  - field: ingested_unix
    type: freshness
    max_age_seconds: 3600
metrics:
  - name: order_value_cap
    field: quantity
    max: 400
"#;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn compile(yaml: &str) -> CompiledContract {
    let c: Contract = serde_yaml::from_str(yaml).expect("yaml");
    CompiledContract::compile(c).expect("compile")
}

fn opts(rows: usize, mode: SynthMode, seed: u64) -> SynthOptions {
    SynthOptions {
        rows,
        mode,
        seed,
        now_unix: now(),
    }
}

/// Per-row violations exactly as ingest sees a batch: per-event checks plus
/// batch uniqueness (second and later occurrences flagged).
fn batch_violations(compiled: &CompiledContract, rows: &[Value]) -> Vec<Vec<Violation>> {
    let mut out: Vec<Vec<Violation>> = rows
        .iter()
        .map(|r| validate(compiled, r).violations)
        .collect();
    for (idx, v) in check_uniqueness_batch(&compiled.contract.quality, rows) {
        out[idx].push(v);
    }
    out
}

fn assert_valid(compiled: &CompiledContract, rows: &[Value]) {
    for (i, vs) in batch_violations(compiled, rows).iter().enumerate() {
        assert!(
            vs.is_empty(),
            "valid row {i} failed: {vs:?}\nrow: {}",
            rows[i]
        );
    }
}

fn assert_invalid(compiled: &CompiledContract, out: &synth::SynthOutput) {
    let records: Vec<Value> = out.invalid_rows.iter().map(|r| r.record.clone()).collect();
    for (i, vs) in batch_violations(compiled, &records).iter().enumerate() {
        let label = &out.invalid_rows[i].broken_rule;
        assert_eq!(
            vs.len(),
            1,
            "invalid row {i} ({label:?}) should break exactly one rule, got {vs:?}\nrow: {}",
            records[i]
        );
        assert_eq!(
            vs[0].kind, label.violation_kind,
            "row {i}: {label:?} vs {vs:?}"
        );
        assert_eq!(vs[0].field, label.field, "row {i}: {label:?} vs {vs:?}");
    }
}

#[test]
fn valid_rows_pass_the_validator() {
    let c = compile(RICH);
    for seed in [1, 7, 42] {
        let out = synth::generate(&c, &opts(2000, SynthMode::Valid, seed)).expect("generate");
        assert_eq!(out.rows.len(), 2000);
        assert_valid(&c, &out.rows);
    }
}

#[test]
fn invalid_rows_break_exactly_their_labeled_rule() {
    let c = compile(RICH);
    let out = synth::generate(&c, &opts(2000, SynthMode::Invalid, 9)).expect("generate");
    assert_eq!(out.invalid_rows.len(), 2000);
    assert_invalid(&c, &out);

    // Only rules that cannot break alone are skipped. For example, a
    // missing `quantity` also fails the metric bound on it, and a field max
    // of 500 sits above the metric max of 400.
    let mut skipped: Vec<String> = out
        .skipped_rules
        .iter()
        .map(|r| format!("{}:{}", r.field, r.rule))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            "customer_email:completeness",
            "customer_email:required",
            "ingested_unix:type",
            "quantity:max",
            "quantity:required",
            "quantity:type",
        ]
    );
    let mut seen: Vec<String> = out
        .invalid_rows
        .iter()
        .map(|r| format!("{}:{}", r.broken_rule.field, r.broken_rule.rule))
        .collect();
    seen.sort();
    seen.dedup();
    for want in [
        "order_id:pattern",
        "order_id:uniqueness",
        "order_id:required",
        "customer_email:format",
        "nickname:completeness",
        "customer_email:max_length",
        "request_id:format",
        "homepage:format",
        "status:enum",
        "quantity:min",
        "quantity:metric_max",
        "unit_price:type",
        "discount_code:min_length",
        "discount_code:max_length",
        "discount_code:pattern",
        "gift:type",
        "order_date:format",
        "placed_at:format",
        "ingested_unix:freshness",
        "shipping.country:pattern",
        "shipping.postcode:required",
        "shipping:type",
    ] {
        assert!(
            seen.iter().any(|s| s == want),
            "no row broke {want}; saw {seen:?}"
        );
    }
    assert!(out
        .invalid_rows
        .iter()
        .any(|r| r.broken_rule.rule == "compliance_mode"));
}

#[test]
fn same_seed_same_rows() {
    let c = compile(RICH);
    let n = now();
    let mk = |seed, mode| SynthOptions {
        rows: 50,
        mode,
        seed,
        now_unix: n,
    };
    for mode in [SynthMode::Valid, SynthMode::Invalid] {
        let a = serde_json::to_value(synth::generate(&c, &mk(5, mode)).unwrap()).unwrap();
        let b = serde_json::to_value(synth::generate(&c, &mk(5, mode)).unwrap()).unwrap();
        let other = serde_json::to_value(synth::generate(&c, &mk(6, mode)).unwrap()).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, other);
    }
}

fn err_rule(yaml: &str, rows: usize, mode: SynthMode) -> synth::SynthError {
    synth::generate(&compile(yaml), &opts(rows, mode, 1)).expect_err("should not satisfy")
}

#[test]
fn unsatisfiable_rules_are_named() {
    let base = |field: &str| format!("version: \"1\"\nname: t\nontology:\n  entities:\n{field}");
    let e = err_rule(
        &base("    - name: code\n      type: string\n      min_length: 5\n      max_length: 3\n"),
        5,
        SynthMode::Valid,
    );
    assert_eq!(e.rule, "min_length");
    assert_eq!(e.field.as_deref(), Some("code"));

    let e = err_rule(
        &base("    - name: code\n      type: string\n      pattern: \"^[a-z]{10}$\"\n      max_length: 5\n"),
        5,
        SynthMode::Valid,
    );
    assert!(e.rule == "pattern" || e.rule == "max_length", "{e:?}");
    assert_eq!(e.field.as_deref(), Some("code"));

    let e = err_rule(
        &base("    - name: n\n      type: integer\n      min: 10\n      max: 2\n"),
        5,
        SynthMode::Valid,
    );
    assert_eq!(e.rule, "min");

    let e = err_rule(
        &(base("    - name: flag\n      type: boolean\n")
            + "quality:\n  - field: flag\n    type: uniqueness\n"),
        5,
        SynthMode::Valid,
    );
    assert_eq!(e.rule, "uniqueness");
    assert_eq!(e.field.as_deref(), Some("flag"));

    let e = err_rule(
        &base("    - name: s\n      type: string\n      enum: [a, bb]\n      min_length: 3\n"),
        5,
        SynthMode::Valid,
    );
    assert_eq!(e.field.as_deref(), Some("s"));

    let e = err_rule(
        &(base("    - name: label\n      type: string\n")
            + "quality:\n  - field: label\n    type: freshness\n    max_age_seconds: 60\n"),
        5,
        SynthMode::Valid,
    );
    assert_eq!(e.field.as_deref(), Some("label"));
}

#[test]
fn row_count_is_bounded() {
    let c = compile(RICH);
    assert_eq!(
        synth::generate(&c, &opts(0, SynthMode::Valid, 1))
            .unwrap_err()
            .rule,
        "rows"
    );
    assert_eq!(
        synth::generate(&c, &opts(synth::MAX_ROWS + 1, SynthMode::Valid, 1))
            .unwrap_err()
            .rule,
        "rows"
    );
    let out = synth::generate(&c, &opts(synth::MAX_ROWS, SynthMode::Valid, 3)).unwrap();
    assert_valid(&c, &out.rows);
}

fn yaml_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            yaml_files(&p, out);
        } else if matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("yaml") | Some("yml")
        ) {
            out.push(p);
        }
    }
}

/// Run both modes over every contract shipped in the repo. Each either
/// generates rows that hold up under the validator or names a rule.
#[test]
fn repo_contracts_hold_up() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in ["contracts", "examples", "tests/fixtures"] {
        yaml_files(&root.join(d), &mut files);
    }
    files.sort();
    let mut generated = 0;
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let Ok(contract) = serde_yaml::from_str::<Contract>(&src) else {
            continue;
        };
        let Ok(c) = CompiledContract::compile(contract) else {
            continue;
        };
        match synth::generate(&c, &opts(200, SynthMode::Valid, 11)) {
            Ok(out) => {
                assert_valid(&c, &out.rows);
                generated += 1;
            }
            Err(e) => eprintln!("{}: valid mode refused: {e:?}", f.display()),
        }
        match synth::generate(&c, &opts(200, SynthMode::Invalid, 11)) {
            Ok(out) => assert_invalid(&c, &out),
            Err(e) => eprintln!("{}: invalid mode refused: {e:?}", f.display()),
        }
    }
    eprintln!(
        "generated rows for {generated} of {} repo YAML files",
        files.len()
    );
    assert!(generated >= 10, "only {generated} repo contracts generated");
}
