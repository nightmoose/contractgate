//! CLI round-trip and check-against tests for `contractgate export dbt`
//! and `contractgate import dbt` (RFC-120).

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_contractgate")
}

fn repo(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn export_import_round_trip_orders_and_check_passes() {
    let dir = tempfile::tempdir().unwrap();
    let schema = dir.path().join("schema.yml");
    let back = dir.path().join("orders.yaml");
    let contract = repo("tests/fixtures/dbt_yaml/orders.yaml");

    let export = Command::new(bin())
        .args([
            "export",
            "dbt",
            contract.to_str().unwrap(),
            "--model",
            "orders",
            "--out",
            schema.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        export.status.code(),
        Some(0),
        "export stderr: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    let yml = std::fs::read_to_string(&schema).unwrap();
    assert!(yml.contains("enforced: true"));
    assert!(yml.contains("contractgate"));
    assert!(yml.contains("order_id"));

    let import = Command::new(bin())
        .args([
            "import",
            "dbt",
            schema.to_str().unwrap(),
            "--model",
            "orders",
            "--out",
            back.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        import.status.code(),
        Some(0),
        "import stderr: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let imported = std::fs::read_to_string(&back).unwrap();
    assert!(imported.contains("name: orders"));
    assert!(imported.contains("pattern:"));

    let check = Command::new(bin())
        .args([
            "import",
            "dbt",
            schema.to_str().unwrap(),
            "--model",
            "orders",
            "--check-against",
            contract.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        check.status.code(),
        Some(0),
        "check stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(stdout.contains("ok:"), "{stdout}");
}

#[test]
fn check_against_detects_drift() {
    let dir = tempfile::tempdir().unwrap();
    let schema = dir.path().join("schema.yml");
    let contract = repo("tests/fixtures/dbt_yaml/orders.yaml");
    let export = Command::new(bin())
        .args([
            "export",
            "dbt",
            contract.to_str().unwrap(),
            "--out",
            schema.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(export.status.code(), Some(0));
    let mut yml = std::fs::read_to_string(&schema).unwrap();
    yml = yml.replace("amount_usd >= 0", "amount_usd >= 10");
    std::fs::write(&schema, yml).unwrap();

    let check = Command::new(bin())
        .args([
            "import",
            "dbt",
            schema.to_str().unwrap(),
            "--check-against",
            contract.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        check.status.code(),
        Some(1),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(stderr.contains("amount_usd.min"), "{stderr}");
}

#[test]
fn import_native_schema_and_unknown_model() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.yaml");
    let schema = repo("tests/fixtures/dbt_yaml/native_orders.yml");
    let import = Command::new(bin())
        .args([
            "import",
            "dbt",
            schema.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        import.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let yaml = std::fs::read_to_string(&out).unwrap();
    assert!(yaml.contains("name: orders"));
    assert!(yaml.contains("amount_usd"));
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("relationships"), "{stderr}");

    let missing = Command::new(bin())
        .args([
            "import",
            "dbt",
            schema.to_str().unwrap(),
            "--model",
            "payments",
        ])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
}

#[test]
fn import_missing_file_exits_nonzero() {
    let result = Command::new(bin())
        .args(["import", "dbt", "/tmp/dcg-dbt-does-not-exist-1125.yml"])
        .output()
        .unwrap();
    assert_ne!(result.status.code(), Some(0));
}
