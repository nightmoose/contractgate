//! Stripe contract-chain examples and the public catalog family seed.
//!
//! The YAML under `examples/contract-chain/stripe/` is the reference shape for
//! a family (raw → staged → mart → egress). Migration 050 copies those files
//! into the public catalog. These tests need no database.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::contract::Contract;
use crate::validation::{validate, CompiledContract};

const CHAIN: &[(&str, &str)] = &[
    ("stripe_charges_raw", "ingest"),
    ("stripe_charges_staged", "stage"),
    ("revenue_daily_mart", "stage"),
    ("partner_payout_feed", "egress"),
];

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("contract-chain")
        .join("stripe")
}

fn load(name: &str) -> (Contract, Value, Value) {
    let dir = example_dir();
    let yaml = fs::read_to_string(dir.join(format!("{name}.yaml"))).unwrap();
    let contract: Contract = serde_yaml::from_str(&yaml).unwrap();
    let pass: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("{name}.pass.json"))).unwrap())
            .unwrap();
    let fail: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("{name}.fail.json"))).unwrap())
            .unwrap();
    (contract, pass, fail)
}

#[test]
fn stripe_chain_examples_pass_and_fail_as_documented() {
    for (name, _) in CHAIN {
        let (contract, pass, fail) = load(name);
        assert_eq!(contract.name, *name);
        let compiled = CompiledContract::compile(contract).unwrap();
        assert!(
            validate(&compiled, &pass).passed,
            "{name} pass sample should validate"
        );
        assert!(
            !validate(&compiled, &fail).passed,
            "{name} fail sample should violate the contract"
        );
    }
}

fn assert_chain(dir_name: &str, chain: &[(&str, &str)]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("contract-chain")
        .join(dir_name);
    for (name, _) in chain {
        let yaml = fs::read_to_string(dir.join(format!("{name}.yaml"))).unwrap();
        let contract: Contract = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(contract.name, *name);
        let pass: Value = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{name}.pass.json"))).unwrap(),
        )
        .unwrap();
        let fail: Value = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{name}.fail.json"))).unwrap(),
        )
        .unwrap();
        let compiled = CompiledContract::compile(contract).unwrap();
        assert!(
            validate(&compiled, &pass).passed,
            "{dir_name}/{name} pass sample should validate"
        );
        assert!(
            !validate(&compiled, &fail).passed,
            "{dir_name}/{name} fail sample should violate the contract"
        );
    }
}

#[test]
fn shopify_and_ga4_chain_examples_pass_and_fail() {
    assert_chain(
        "shopify",
        &[
            ("shopify_orders_raw", "ingest"),
            ("shopify_orders_staged", "stage"),
            ("shopify_fulfillment_notice", "egress"),
        ],
    );
    assert_chain(
        "ga4",
        &[
            ("ga4_events_raw", "ingest"),
            ("ga4_events_staged", "stage"),
            ("ga4_audience_export", "egress"),
        ],
    );
}

#[test]
fn catalog_family_seed_052_embeds_the_example_contracts() {
    let sql = include_str!("../supabase/migrations/052_shopify_and_ga4_contract_families.sql");
    assert!(sql.contains("shopify-orders-pipeline"));
    assert!(sql.contains("ga4-events-pipeline"));
    for (dir_name, names) in [
        (
            "shopify",
            [
                "shopify_orders_raw",
                "shopify_orders_staged",
                "shopify_fulfillment_notice",
            ],
        ),
        (
            "ga4",
            ["ga4_events_raw", "ga4_events_staged", "ga4_audience_export"],
        ),
    ] {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("contract-chain")
            .join(dir_name);
        for name in names {
            let yaml = fs::read_to_string(dir.join(format!("{name}.yaml"))).unwrap();
            assert!(
                sql.contains(yaml.trim()),
                "migration 052 is missing the {name} contract"
            );
            assert!(
                sql.contains(&format!("child.name = '{name}'")),
                "migration 052 is missing the {name} link"
            );
        }
    }
    assert!(sql.contains("'ingest', 0"));
    assert!(sql.contains("'stage', 1"));
    assert!(sql.contains("'egress', 2"));
}

#[test]
fn catalog_family_seed_matches_the_stripe_chain() {
    let sql = include_str!("../supabase/migrations/050_contract_families.sql");
    assert!(sql.contains("stripe-charges-pipeline"));
    assert!(sql.contains("'ingest', 0"));
    assert!(sql.contains("'stage', 1"));
    assert!(sql.contains("'stage', 2"));
    assert!(sql.contains("'egress', 3"));
    assert!(sql.contains("parent.name = 'stripe_charges_raw'"));
    assert!(sql.contains("parent.name = 'stripe_charges_staged'"));
    assert!(sql.contains("parent.name = 'revenue_daily_mart'"));
    for (name, boundary) in CHAIN {
        assert!(
            sql.contains(&format!("child.name = '{name}'")),
            "seed missing {name}"
        );
        assert!(sql.contains(*boundary));
    }
}
