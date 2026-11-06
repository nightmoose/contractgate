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
fn review_findings_stay_fixed() {
    let acs = load_contract("acs-5year-2023-county");
    let acs_fields: std::collections::HashMap<_, _> = acs
        .ontology
        .entities
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    assert_eq!(
        acs_fields["B19013_001M"].field_type,
        crate::contract::FieldType::String,
        "B19013_001M is the numeric margin of error, returned as a string"
    );
    assert_eq!(
        acs_fields["B19013_001MA"].field_type,
        crate::contract::FieldType::String,
        "B19013_001MA is the annotation, a text code"
    );
    assert!(
        acs.null_as_absent,
        "Census JSON null cells must be treated as absent"
    );
    let acs_meta = std::fs::read_to_string(seeds_dir().join("acs-5year-2023-county").join("meta.json")).unwrap();
    assert!(
        acs_meta.contains("https://www.census.gov/data/developers/data-sets/acs-5year.html"),
        "ACS citation must be the working 5-year dataset page"
    );
    assert!(
        !acs_meta.contains("acs/acs5.html"),
        "the old ACS dataset URL 404s"
    );
    assert!(
        acs_meta.contains("not affiliated with the U.S. Census Bureau"),
        "ACS card copy needs an affiliation line"
    );

    let stripe = load_contract("stripe-charges");
    let id_pattern = stripe
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "id")
        .and_then(|f| f.pattern.as_deref())
        .unwrap();
    let id_re = regex::Regex::new(id_pattern).unwrap();
    assert!(id_re.is_match("ch_3Oexample0001AbCdEfGh"));
    assert!(id_re.is_match("py_3Oexample0002IjKlMnOp"));
    assert!(!id_re.is_match("src_example"));

    let shopify = load_contract("shopify-orders");
    let financial = shopify
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "Financial Status")
        .and_then(|f| f.allowed_values.as_ref())
        .unwrap();
    let financial: Vec<_> = financial.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(financial.contains(&"expired"), "admin status list includes expired");
    assert!(!financial.contains(&"unpaid"), "unpaid is a filter grouping, not a status");
    let name_re = regex::Regex::new(
        shopify
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "Name")
            .and_then(|f| f.pattern.as_deref())
            .unwrap(),
    )
    .unwrap();
    assert!(name_re.is_match("#1001"));
    assert!(name_re.is_match("SH1001"));
    assert!(name_re.is_match("1001-A"));
    assert!(!name_re.is_match("example_Name"));

    let square = load_contract("square-payments");
    let sources = square
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "source_type")
        .and_then(|f| f.allowed_values.as_ref())
        .unwrap();
    let sources: Vec<_> = sources.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(sources.contains(&"BUY_NOW_PAY_LATER"));
    assert!(sources.contains(&"SQUARE_ACCOUNT"));
    let square_samples = load_samples("square-payments");
    for sample in &square_samples {
        let id = sample["id"].as_str().unwrap();
        assert!(!id.starts_with("pmt_"), "{id} uses a Stripe-style prefix");
        assert!(!id.starts_with("ord_"), "{id} uses a Stripe-style prefix");
    }

    let generic_meta = std::fs::read_to_string(
        seeds_dir()
            .join("generic-sheets-transactions-csv")
            .join("meta.json"),
    )
    .unwrap();
    assert!(
        !generic_meta.contains("app.datacontractgate.com/catalog/"),
        "generic citation must not point at its own catalog page"
    );
    assert!(
        !generic_meta.contains("not affiliated with DataContractGate"),
        "generic copy must not disclaim affiliation with itself"
    );
    assert!(
        !generic_meta.contains("compatible with DataContractGate"),
        "generic copy must not claim compatibility with itself"
    );

    let xero = load_contract("xero-invoices");
    assert!(
        xero.description.as_deref().unwrap_or("").contains("/Date("),
        "Xero description must say the JSON API emits /Date(...)/"
    );
    let xero_date = xero
        .ontology
        .entities
        .iter()
        .find(|f| f.name == "Date")
        .unwrap();
    assert_eq!(xero_date.field_type, crate::contract::FieldType::String);
    let date_re = regex::Regex::new(xero_date.pattern.as_deref().unwrap()).unwrap();
    assert!(date_re.is_match("/Date(1768003200000+0000)/"));
    assert!(!date_re.is_match("2026-01-10"));

    let hubspot_meta =
        std::fs::read_to_string(seeds_dir().join("hubspot-contacts").join("meta.json")).unwrap();
    assert!(
        !hubspot_meta.contains("https://developers.hubspot.com/docs/api/crm/contacts\""),
        "HubSpot citation must not stay on the legacy contacts URL"
    );
    assert!(hubspot_meta.contains("developers.hubspot.com/docs/api-reference/"));

    // Salesforce catalog samples are already 15–18 character ids. The playground
    // failure was example_Id, which these samples are not.
    let sf_re = regex::Regex::new(r"^[A-Za-z0-9]{15,18}$").unwrap();
    for slug in ["salesforce-account", "salesforce-opportunity"] {
        for sample in load_samples(slug) {
            let id = sample["Id"].as_str().unwrap();
            assert!(sf_re.is_match(id), "{slug} sample Id {id} is not 15–18 alphanumeric");
            assert!(id.to_ascii_lowercase().contains("example") || id.contains("Example"));
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
