//! RFC-118 parity corpus: cross-field rules (including per-item `[]`
//! paths), patterns inside array items, per-event quality rules and byte
//! lengths. The Python SDK's local validator runs the same file
//! (`sdks/python/tests/test_parity_rfc118.py`).
//!
//! Regenerate expectations after an intended engine change with
//! `PARITY_WRITE=1 cargo test --test parity_rfc118`.

use contractgate::contract::Contract;
use contractgate::validation::{validate_at, CompiledContract};
use serde_json::{json, Value};

fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity/rfc118.json")
}

#[test]
fn rfc118_parity_corpus() {
    let mut fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(path()).unwrap()).unwrap();
    let c: Contract = serde_yaml::from_str(fixture["contract_yaml"].as_str().unwrap()).unwrap();
    let compiled = CompiledContract::compile(c).unwrap();
    let now = fixture["now"].as_i64().unwrap();
    let write = std::env::var("PARITY_WRITE").is_ok();
    let mut failures = Vec::new();
    for case in fixture["cases"].as_array_mut().unwrap() {
        let got: Vec<Value> = validate_at(&compiled, &case["event"], now)
            .violations
            .iter()
            .map(|v| json!([v.field, v.kind, v.message]))
            .collect();
        let got = Value::Array(got);
        if write {
            case["expected"] = got;
        } else if case["expected"] != got {
            failures.push(format!(
                "{}: expected {} got {}",
                case["name"], case["expected"], got
            ));
        }
    }
    if write {
        std::fs::write(
            path(),
            serde_json::to_string_pretty(&fixture).unwrap() + "\n",
        )
        .unwrap();
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn bad_cross_field_shapes_fail_to_compile() {
    for (rule, msg) in [
        ("  - field: a\n    op: gte\n", "must set either"),
        (
            "  - field: a[].x[].y\n    op: gte\n    other: b\n",
            "more than one",
        ),
        (
            "  - field: a[].x\n    op: gte\n    other: b[].y\n",
            "two different arrays",
        ),
    ] {
        let y = format!(
            "version: \"1\"\nname: t\nontology:\n  entities:\n    - name: a\n      type: integer\ncross_field:\n{rule}"
        );
        let c: Contract = serde_yaml::from_str(&y).unwrap();
        let err = match CompiledContract::compile(c) {
            Ok(_) => panic!("should fail: {rule}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains(msg), "{err}");
    }
}
