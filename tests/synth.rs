//! Synthetic data generator (RFC-117): every valid row passes the validator,
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

    // Only rules the contract's own logic makes impossible to break alone
    // are skipped, each with the reason. A missing `quantity` also fails
    // the metric on it, a field max of 500 sits above the metric max of 400,
    // and a required email with a completeness rule cannot be empty alone.
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
            "quantity:max",
            "quantity:required",
        ]
    );
    for r in &out.skipped_rules {
        assert!(
            !r.reason.is_empty() && !r.reason.contains("every attempt"),
            "skip must come from the contract's logic, not a failed search: {r:?}"
        );
    }
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
        &(base("    - name: label\n      type: boolean\n")
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

#[test]
fn cli_writes_json_and_csv() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.yaml");
    std::fs::write(&path, RICH).unwrap();
    let bin = env!("CARGO_BIN_EXE_contractgate");
    let run = |extra: &[&str]| {
        std::process::Command::new(bin)
            .arg("synth")
            .arg(&path)
            .args(extra)
            .output()
            .unwrap()
    };

    let a = run(&["--rows", "25", "--seed", "4"]);
    assert_eq!(
        a.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&a.stderr)
    );
    let rows: Vec<Value> = serde_json::from_slice(&a.stdout).unwrap();
    assert_eq!(rows.len(), 25);
    assert_valid(&compile(RICH), &rows);
    assert_eq!(a.stdout, run(&["--rows", "25", "--seed", "4"]).stdout);

    let bad = run(&[
        "--rows",
        "10",
        "--seed",
        "4",
        "--invalid",
        "--format",
        "csv",
    ]);
    assert_eq!(bad.status.code(), Some(0));
    let text = String::from_utf8(bad.stdout).unwrap();
    let header = text.lines().next().unwrap();
    assert!(header.contains("_broken_field") && header.contains("_broken_rule"));
    assert!(header.contains("shipping.country"));
    assert_eq!(text.lines().count(), 11);

    let unsat = dir.path().join("u.yaml");
    std::fs::write(
        &unsat,
        "version: \"1\"\nname: u\nontology:\n  entities:\n    - name: c\n      type: string\n      min_length: 5\n      max_length: 2\n",
    )
    .unwrap();
    let e = std::process::Command::new(bin)
        .args(["synth", unsat.to_str().unwrap(), "--rows", "3"])
        .output()
        .unwrap();
    assert_eq!(e.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&e.stderr).contains("min_length"));
}

const V11: &str = r#"
version: "1.0"
name: synth_v11
envelope:
  records_path: data
  validate_wrapper: true
ontology:
  entities:
    - name: booking_id
      type: string
      format: uuid
    - name: status
      type: string
      enum: [booked, cancelled, refunded]
    - name: refund_reason
      type: string
      required: false
      min_length: 3
    - name: start_date
      type: date
    - name: end_date
      type: date
    - name: created_at
      type: timestamp
    - name: updated_at
      type: timestamp
    - name: min_guests
      type: integer
      min: 1
      max: 4
    - name: max_guests
      type: integer
      min: 1
      max: 8
    - name: guest_email
      type: string
      format: email
    - name: host_email
      type: string
      format: email
    - name: lines
      type: array
      items:
        name: line
        type: object
        properties:
          - name: sku
            type: string
            min_length: 3
            max_length: 8
          - name: qty
            type: integer
            min: 1
            max: 10
          - name: kind
            type: string
            enum: [room, fee]
          - name: code
            type: string
            pattern: "^L-[0-9]{3}$"
          - name: fee_reason
            type: string
            required: false
            min_length: 2
    - name: tags
      type: array
      required: false
      items:
        name: tag
        type: string
        max_length: 6
    - name: last_seen_unix
      type: integer
    - name: seen_at
      type: timestamp
    - name: max_line_qty
      type: integer
      min: 5
      max: 20
quality:
  - field: last_seen_unix
    type: freshness
    max_age_seconds: 600
  - field: seen_at
    type: freshness
    max_age_seconds: 900
cross_field:
  - field: end_date
    op: gte
    other: start_date
  - field: updated_at
    op: gt
    other: created_at
  - field: max_guests
    op: gte
    other: min_guests
  - field: host_email
    op: ne
    other: guest_email
  - field: refund_reason
    required_if:
      field: status
      equals: refunded
  - field: lines[].qty
    op: lte
    other: max_line_qty
  - field: lines[].fee_reason
    required_if:
      field: lines[].kind
      equals: fee
"#;

#[test]
fn cross_field_items_and_envelope() {
    let c = compile(V11);
    for seed in [1, 2, 3] {
        let out = synth::generate(&c, &opts(1000, SynthMode::Valid, seed)).expect("valid");
        assert_valid(&c, &out.rows);
        for r in &out.rows {
            if r["status"] == "refunded" {
                assert!(r["refund_reason"].is_string(), "{r}");
            }
        }
        let wrapped = synth::wrap_envelope(&c.contract, out.rows.clone()).unwrap();
        let env = contractgate::validation::validate_envelope_batch(
            &c,
            c.contract.envelope.as_ref().unwrap(),
            &wrapped,
        );
        assert_eq!(env.quarantined, 0, "{:?}", env.violations.first());
        assert_eq!(env.passed, 1000);
    }

    let out = synth::generate(&c, &opts(1500, SynthMode::Invalid, 5)).expect("invalid");
    assert_invalid(&c, &out);
    let mut seen: Vec<String> = out
        .invalid_rows
        .iter()
        .map(|r| {
            format!(
                "{}:{}:{}",
                r.broken_rule.field,
                r.broken_rule.rule,
                r.broken_rule.constraint.clone().unwrap_or_default()
            )
        })
        .collect();
    seen.sort();
    seen.dedup();
    for want in [
        "end_date:cross_field:end_date >= start_date",
        "updated_at:cross_field:updated_at > created_at",
        "max_guests:cross_field:max_guests >= min_guests",
        "host_email:cross_field:host_email != guest_email",
        "refund_reason:cross_field:refund_reason required if status == \"refunded\"",
        "lines[0].qty:cross_field:lines[].qty <= max_line_qty",
        "lines[0].fee_reason:cross_field:lines[].fee_reason required if lines[].kind == \"fee\"",
        "lines[0].code:pattern:",
        "last_seen_unix:type:",
        "seen_at:freshness:",
        "seen_at:type:",
        "lines[0].sku:min_length:",
        "lines[0].sku:max_length:",
        "lines[0].sku:required:",
        "lines[0].qty:max:",
        "lines[0].kind:enum:",
        "lines[0]:type:",
        "tags[0]:max_length:",
        "tags[0]:type:",
    ] {
        assert!(
            seen.iter().any(|s| s == want),
            "no row broke {want}; saw {seen:?}"
        );
    }
    // The one rule that is logically impossible to break alone: any
    // max_guests below its min of 1 is below min_guests (required, min 1),
    // so `max_guests >= min_guests` fails too. The reason says so.
    // seen_at:format is the other: freshness parses the same RFC 3339
    // format, so a malformed timestamp always fails freshness too.
    let mut skipped: Vec<(String, String)> = out
        .skipped_rules
        .iter()
        .map(|r| (format!("{}:{}", r.field, r.rule), r.reason.clone()))
        .collect();
    skipped.sort();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert_eq!(skipped[0].0, "max_guests:min");
    assert!(skipped[0].1.contains("max_guests >= min_guests"));
    assert_eq!(skipped[1].0, "seen_at:format");
    assert!(skipped[1].1.contains("also fails freshness"));
}

#[test]
fn anchor_time_makes_freshness_repeat() {
    let c = compile(V11);
    let anchor = 1_800_000_000;
    let mk = |seed| SynthOptions {
        rows: 30,
        mode: SynthMode::Valid,
        seed,
        now_unix: anchor,
    };
    let a = serde_json::to_value(synth::generate(&c, &mk(9)).unwrap()).unwrap();
    let b = serde_json::to_value(synth::generate(&c, &mk(9)).unwrap()).unwrap();
    assert_eq!(a, b);
    for r in a["rows"].as_array().unwrap() {
        let t = r["last_seen_unix"].as_i64().unwrap();
        assert!(t <= anchor && t >= anchor - 600, "{t}");
    }
}

#[test]
fn cross_field_parse_validate_and_odcs_round_trip() {
    let c = compile(V11);
    let ok = serde_json::json!({"start_date": "2026-01-02", "end_date": "2026-01-01"});
    let r = validate(&c, &ok);
    assert!(r.violations.iter().any(|v| v.field == "end_date"
        && v.kind == contractgate::validation::ViolationKind::CrossFieldViolation));
    // Malformed rule: both forms set.
    let bad = format!(
        "{}  - field: a\n    op: lt\n    other: b\n    required_if: {{field: c, equals: 1}}\n",
        V11
    );
    let parsed: Contract = serde_yaml::from_str(&bad).unwrap();
    assert!(CompiledContract::compile(parsed).is_err());
    // A contract without cross_field serializes without the key.
    let plain: Contract = serde_yaml::from_str(RICH).unwrap();
    assert!(!serde_yaml::to_string(&plain)
        .unwrap()
        .contains("cross_field"));
}

#[test]
fn cli_anchor_and_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v11.yaml");
    std::fs::write(&path, V11).unwrap();
    let bin = env!("CARGO_BIN_EXE_contractgate");
    let run = |extra: &[&str]| {
        std::process::Command::new(bin)
            .arg("synth")
            .arg(&path)
            .args(extra)
            .output()
            .unwrap()
    };
    let args = [
        "--rows",
        "12",
        "--seed",
        "5",
        "--anchor",
        "2026-01-01T00:00:00Z",
        "--envelope",
    ];
    let a = run(&args);
    assert_eq!(
        a.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&a.stderr)
    );
    assert_eq!(a.stdout, run(&args).stdout);
    let v: Value = serde_json::from_slice(&a.stdout).unwrap();
    assert_eq!(v["data"].as_array().unwrap().len(), 12);

    let mut inv = args.to_vec();
    inv.push("--invalid");
    let b = run(&inv);
    assert_eq!(b.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&b.stdout).unwrap();
    assert_eq!(v["labels"].as_array().unwrap().len(), 12);
    assert!(v["payload"]["data"].is_array());

    assert_eq!(run(&["--anchor", "soon"]).status.code(), Some(2));
    assert_eq!(
        run(&["--envelope", "--format", "csv"]).status.code(),
        Some(2)
    );
}

const NAMED: &str = r#"
version: "1.0"
name: named_fields
ontology:
  entities:
    - name: id
      type: integer
    - name: email
      type: string
    - name: customerName
      type: string
    - name: phone
      type: string
    - name: website
      type: string
    - name: country_code
      type: string
    - name: user_id
      type: string
    - name: city
      type: string
      max_length: 3
    - name: contact_email
      type: string
      pattern: "^[0-9]{4}$"
    - name: widget
      type: string
"#;

#[test]
fn free_form_strings_follow_field_names_when_rules_allow() {
    let c = synth::compile_yaml(NAMED).unwrap();
    let out = synth::generate(&c, &opts(200, SynthMode::Valid, 7)).unwrap();
    let again = synth::generate(&c, &opts(200, SynthMode::Valid, 7)).unwrap();
    assert_eq!(out.rows, again.rows, "seeded output must repeat");
    for r in &out.rows {
        assert!(validate(&c, r).passed, "{r}");
        let s = |k: &str| r[k].as_str().unwrap().to_string();
        let email = s("email");
        assert!(
            email.contains('@') && [".com", ".org", ".net"].iter().any(|t| email.ends_with(t)),
            "{email}"
        );
        assert!(s("customerName").contains(' '), "{}", s("customerName"));
        assert!(s("phone").starts_with("+1-"), "{}", s("phone"));
        assert!(s("website").starts_with("https://"), "{}", s("website"));
        assert_eq!(s("country_code").len(), 2);
        assert!(uuid::Uuid::parse_str(&s("user_id")).is_ok());
        // City names do not fit max_length 3, so the generic value is used.
        assert!(s("city").len() <= 3);
        // The declared pattern wins over the name.
        assert_eq!(s("contact_email").len(), 4);
        // No hint in the name: generic string.
        assert!(!s("widget").is_empty());
    }
    // Invalid mode still breaks every format-free rule on these fields.
    let inv = synth::generate(&c, &opts(60, SynthMode::Invalid, 7)).unwrap();
    assert!(inv.skipped_rules.is_empty(), "{:?}", inv.skipped_rules);
}
