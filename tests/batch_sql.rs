//! RFC-119 golden-file tests for batch SQL generation, plus an optional
//! Postgres end-to-end run (ignored; needs a live DATABASE_URL).
//!
//! Regenerating goldens:
//!
//!   UPDATE_GOLDENS=1 cargo test --test batch_sql -- goldens_match
//!
//! The Postgres test is `#[ignore]` so plain `cargo test` stays
//! hermetic. Run it against any Postgres (the query talks to a throwaway
//! table, not DataContractGate's own schema):
//!
//!   DATABASE_URL=postgres://... cargo test --test batch_sql -- --ignored --nocapture

use contractgate::batch_check;
use contractgate::contract::Contract;
use contractgate::dbt_export;
use contractgate::sql_dialect::SqlDialect;
use contractgate::validation::CompiledContract;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_contract() -> Contract {
    let path = manifest().join("tests/fixtures/batch_sql/orders.yaml");
    let yaml =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let contract: Contract = serde_yaml::from_str(&yaml).expect("orders.yaml parses");
    CompiledContract::compile(contract)
        .expect("orders.yaml compiles")
        .contract
}

fn golden_dir() -> PathBuf {
    manifest().join("tests/batch_sql_golden")
}

fn maybe_update(path: &Path, actual: &str) -> bool {
    if std::env::var("UPDATE_GOLDENS").ok().as_deref() == Some("1") {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("creating golden dir");
        }
        fs::write(path, actual).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        true
    } else {
        false
    }
}

fn assert_golden(rel: &str, actual: &str) {
    let path = golden_dir().join(rel);
    if maybe_update(&path, actual) {
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading golden {}: {e}\nRe-run with UPDATE_GOLDENS=1 to create it.\nActual was:\n{actual}",
            path.display()
        )
    });
    assert_eq!(
        expected,
        actual,
        "golden file {} drifted; re-run with UPDATE_GOLDENS=1 if the change is intended",
        path.display()
    );
}

#[test]
fn goldens_match_check_table_sql_for_all_dialects() {
    let contract = fixture_contract();
    let keys = vec!["order_id".to_string()];
    for dialect in SqlDialect::all() {
        let out = batch_check::generate_check_sql(&contract, dialect, "orders", &keys)
            .expect("check-table SQL generates");
        assert!(
            out.rule_count > 0,
            "{dialect} should compile at least one rule"
        );
        assert!(
            out.skipped.is_empty(),
            "{dialect} fixture should have no skipped rules, got {:?}",
            out.skipped
        );
        assert_golden(&format!("check_table_{dialect}.sql"), &out.sql);
    }
}

#[test]
fn goldens_match_dbt_export() {
    let contract = fixture_contract();
    let postgres = dbt_export::generate_dbt_tests(&contract, "orders", SqlDialect::Postgres);
    assert!(postgres.skipped.is_empty());
    assert_golden("dbt_schema.yml", &postgres.schema_yml);

    // schema.yml is dialect-independent; only singular SQL tests differ.
    for dialect in SqlDialect::all() {
        let out = dbt_export::generate_dbt_tests(&contract, "orders", dialect);
        assert_eq!(
            out.schema_yml, postgres.schema_yml,
            "schema.yml must be the same for every dialect"
        );
        let mut tests = out.singular_tests;
        tests.sort_by(|a, b| a.filename.cmp(&b.filename));
        let mut combined = String::new();
        for t in &tests {
            combined.push_str(&format!("-- ===== {} =====\n{}", t.filename, t.sql));
            if !t.sql.ends_with('\n') {
                combined.push('\n');
            }
        }
        assert_golden(&format!("dbt_singular_{dialect}.sql"), &combined);
    }
}

/// Live Postgres check of the generated SQL against planted good and bad
/// rows. Ignored so CI's default `cargo test` stays hermetic; the RFC-119
/// "proven for batch" evidence is this test plus the goldens.
#[test]
#[ignore]
fn postgres_check_table_reports_planted_violations() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!("DATABASE_URL is required for this ignored test (any Postgres; no DataContractGate migrations needed)")
    });

    let contract = fixture_contract();
    let keys = vec!["order_id".to_string()];
    let out =
        batch_check::generate_check_sql(&contract, SqlDialect::Postgres, "rfc119_orders", &keys)
            .expect("check-table SQL generates");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        use sqlx::{postgres::PgPoolOptions, Row};

        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connecting to postgres");

        sqlx::query("drop table if exists rfc119_orders")
            .execute(&pool)
            .await
            .expect("drop");
        sqlx::query(
            r#"
            create table rfc119_orders (
                order_id text,
                customer_id text,
                amount_usd double precision,
                status text,
                created_at timestamptz,
                updated_at timestamptz,
                discount_code text
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create table");

        // One fully valid row.
        sqlx::query(
            r#"
            insert into rfc119_orders values
            ('ord_abcdefghij', 'cus_abcdefgh', 12.50, 'paid',
             now() - interval '1 hour', now() - interval '30 minutes', null)
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert valid");

        // Planted violations:
        // - bad_pattern: order_id does not match ^ord_...
        // - negative: amount_usd < 0
        // - bad_status: status not in enum
        // - stale: created_at older than 7 days (freshness)
        // - backwards: updated_at < created_at
        // - cancelled_no_discount: status = cancelled and discount_code is null
        // - too_short: discount_code shorter than min_length 4
        sqlx::query(
            r#"
            insert into rfc119_orders values
            ('BADID', 'cus_abcdefgh', 12.50, 'paid',
             now() - interval '1 hour', now() - interval '30 minutes', null),
            ('ord_negative00', 'cus_abcdefgh', -1, 'paid',
             now() - interval '1 hour', now() - interval '30 minutes', null),
            ('ord_badstatus0', 'cus_abcdefgh', 12.50, 'lost',
             now() - interval '1 hour', now() - interval '30 minutes', null),
            ('ord_stale00000', 'cus_abcdefgh', 12.50, 'paid',
             now() - interval '30 days', now() - interval '30 days', null),
            ('ord_backwards0', 'cus_abcdefgh', 12.50, 'paid',
             now() - interval '1 hour', now() - interval '2 hours', null),
            ('ord_cancel0000', 'cus_abcdefgh', 12.50, 'cancelled',
             now() - interval '1 hour', now() - interval '30 minutes', null),
            ('ord_shortcode0', 'cus_abcdefgh', 12.50, 'paid',
             now() - interval '1 hour', now() - interval '30 minutes', 'ab')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert bad rows");

        let rows = sqlx::query(&out.sql)
            .fetch_all(&pool)
            .await
            .expect("running generated check-table SQL");

        let mut found: Vec<(String, String)> = rows
            .iter()
            .map(|row| {
                let id: String = row.get("order_id");
                let rule: String = row.get("rule");
                (id, rule)
            })
            .collect();
        found.sort();

        let expected = vec![
            ("BADID".to_string(), "pattern:order_id".to_string()),
            (
                "ord_backwards0".to_string(),
                "cross_field:updated_at".to_string(),
            ),
            ("ord_badstatus0".to_string(), "enum:status".to_string()),
            (
                "ord_cancel0000".to_string(),
                "cross_field:discount_code".to_string(),
            ),
            ("ord_negative00".to_string(), "range:amount_usd".to_string()),
            (
                "ord_shortcode0".to_string(),
                "length:discount_code".to_string(),
            ),
            (
                "ord_stale00000".to_string(),
                "freshness:created_at".to_string(),
            ),
        ];
        assert_eq!(found, expected, "unexpected violation set: {found:?}");

        // The fully valid row must not appear.
        assert!(found.iter().all(|(id, _)| id != "ord_abcdefghij"));

        sqlx::query("drop table if exists rfc119_orders")
            .execute(&pool)
            .await
            .expect("cleanup");
    });
}
