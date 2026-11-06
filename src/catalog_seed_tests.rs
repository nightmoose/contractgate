//! Public Catalog phase 1 — seed integrity tests.
//!
//! Guards the `catalog/seeds/<slug>/` directories that back the public catalog
//! (and migration 047). For every launch slug we check that:
//!   (a) the slug directory exists,
//!   (b) `contract.yaml` parses into a `Contract` and compiles,
//!   (c) `samples.json` holds exactly 3 synthetic rows, and
//!   (d) every sample validates against the contract with 0 violations.
//!
//! These run under `cargo test --lib` with no database.

use std::fs;
use std::path::PathBuf;

use crate::contract::Contract;
use crate::validation::{validate, CompiledContract};

/// The 12 launch slugs (11 vendor/generic contracts + ACS). Do not drop any
/// to make a test pass — the catalog ships all twelve.
const LAUNCH_SLUGS: &[&str] = &[
    "stripe-charges",
    "stripe-customers",
    "shopify-orders",
    "quickbooks-invoices",
    "hubspot-contacts",
    "salesforce-account",
    "salesforce-opportunity",
    "ga4-bigquery-events",
    "square-payments",
    "xero-invoices",
    "generic-sheets-transactions-csv",
    "acs-5year-2023-county",
];

fn seeds_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("catalog")
        .join("seeds")
}

fn load_contract(slug: &str) -> Contract {
    let path = seeds_dir().join(slug).join("contract.yaml");
    let yaml = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("parse contract.yaml for {slug}: {e}"))
}

fn load_samples(slug: &str) -> Vec<serde_json::Value> {
    let path = seeds_dir().join(slug).join("samples.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse samples.json for {slug}: {e}"))
}

#[test]
fn all_twelve_slugs_present() {
    for slug in LAUNCH_SLUGS {
        let dir = seeds_dir().join(slug);
        assert!(
            dir.join("contract.yaml").is_file(),
            "missing contract.yaml for slug {slug}"
        );
        assert!(
            dir.join("samples.json").is_file(),
            "missing samples.json for slug {slug}"
        );
        assert!(
            dir.join("meta.json").is_file(),
            "missing meta.json for slug {slug}"
        );
    }
    assert_eq!(LAUNCH_SLUGS.len(), 12, "exactly 12 launch slugs expected");
}

#[test]
fn every_contract_yaml_parses_and_compiles() {
    for slug in LAUNCH_SLUGS {
        let contract = load_contract(slug);
        CompiledContract::compile(contract)
            .unwrap_or_else(|e| panic!("compile contract for {slug}: {e}"));
    }
}

#[test]
fn every_meta_json_is_well_formed() {
    for slug in LAUNCH_SLUGS {
        let path = seeds_dir().join(slug).join("meta.json");
        let text = fs::read_to_string(&path).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("parse meta.json for {slug}: {e}"));
        assert_eq!(
            meta.get("slug").and_then(|v| v.as_str()),
            Some(*slug),
            "meta.json slug mismatch for {slug}"
        );
        for key in [
            "vendor",
            "category",
            "verified_source_url",
            "verified_at",
            "license_note",
        ] {
            assert!(
                meta.get(key).and_then(|v| v.as_str()).is_some(),
                "meta.json for {slug} missing string field {key}"
            );
        }
    }
}

#[test]
fn three_samples_validate_with_zero_violations() {
    for slug in LAUNCH_SLUGS {
        let contract = load_contract(slug);
        let compiled = CompiledContract::compile(contract)
            .unwrap_or_else(|e| panic!("compile contract for {slug}: {e}"));
        let samples = load_samples(slug);
        assert_eq!(
            samples.len(),
            3,
            "slug {slug} must have exactly 3 synthetic sample rows"
        );
        for (i, sample) in samples.iter().enumerate() {
            let result = validate(&compiled, sample);
            assert!(
                result.passed,
                "sample {i} for slug {slug} failed validation: {:?}",
                result.violations
            );
        }
    }
}
