//! End-to-end `contractgate review` on the sample checkout CSV.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_contractgate")
}

fn repo(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn review_orders_sample_writes_report_and_masks_pii() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let csv = repo("examples/readiness-review/orders.csv");
    let result = Command::new(bin())
        .args([
            "review",
            csv.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--name",
            "checkout_orders",
        ])
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("rejected"), "{stdout}");

    let yaml = std::fs::read_to_string(out.join("contract.yaml")).unwrap();
    assert!(yaml.contains("name: checkout_orders"));
    assert!(yaml.contains("customer_email"));

    let html = std::fs::read_to_string(out.join("report.html")).unwrap();
    assert!(html.contains("DataContractGate"));
    assert!(html.contains("Readiness Review"));
    // The sample CSV uses @example.com addresses from the synth generator.
    assert!(
        !html.contains("@example.com") || html.contains("***"),
        "emails should be masked in the report"
    );
    // Stronger: raw synth-looking full emails should not appear if PII fired.
    // Use a unique valid email from the file if present.
    let src = std::fs::read_to_string(&csv).unwrap();
    let emails: Vec<&str> = src
        .split([',', '\n'])
        .map(|s| s.trim())
        .filter(|s| s.contains('@') && s.contains("example."))
        .collect();
    for e in emails.iter().take(5) {
        assert!(!html.contains(e), "unmasked email {e} in report");
    }

    let rejected = std::fs::read_to_string(out.join("rejected.csv")).unwrap();
    assert!(rejected.contains("_row"));
    assert!(rejected.contains("_reasons"));
    assert!(!rejected.trim().is_empty());
}

#[test]
fn review_missing_file_exits_nonzero() {
    let result = Command::new(bin())
        .args(["review", "/tmp/dcg-review-does-not-exist-1125.csv"])
        .output()
        .unwrap();
    assert_ne!(result.status.code(), Some(0));
}
