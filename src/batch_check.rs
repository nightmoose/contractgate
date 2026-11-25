//! `check-table`: run a contract's rules as one SQL query against a
//! warehouse table (RFC-119).
//!
//! The query is a `UNION ALL` of one `SELECT <key columns>, rule, reason`
//! per rule `batch_rules` compiled, so the result is a flat violation
//! report: every row names the key of the offending record, which rule it
//! broke, and a plain-English reason — not just a failed-row count.

use crate::batch_rules::{self, RulePredicate, SkippedRule};
use crate::contract::Contract;
use crate::sql_dialect::{self, SqlDialect};

#[derive(Debug)]
pub struct CheckTableSql {
    pub sql: String,
    pub rule_count: usize,
    pub skipped: Vec<SkippedRule>,
}

/// `table_ref` is used verbatim (e.g. `orders`, `analytics.orders`, or a
/// dialect-specific quoted/three-part name) — the caller is responsible
/// for any schema qualification and casing their warehouse needs.
pub fn generate_check_sql(
    contract: &Contract,
    dialect: SqlDialect,
    table_ref: &str,
    key_columns: &[String],
) -> Result<CheckTableSql, String> {
    if key_columns.is_empty() {
        return Err("at least one --key column is required".to_string());
    }

    let compiled = batch_rules::compile(contract, dialect, table_ref);
    if compiled.predicates.is_empty() {
        return Err(
            "this contract has no rule that translates to batch SQL (see skipped list)".to_string(),
        );
    }

    let key_cols_quoted: Vec<String> = key_columns
        .iter()
        .map(|c| sql_dialect::quote_ident(dialect, c))
        .collect();
    let key_cols_csv = key_cols_quoted.join(", ");

    let branches: Vec<String> = compiled
        .predicates
        .iter()
        .map(|p| render_branch(&key_cols_csv, table_ref, p))
        .collect();

    let sql = format!(
        "with violations as (\n{body}\n)\nselect *\nfrom violations\norder by {key_cols_csv}, rule\n",
        body = branches.join("\n  union all\n"),
    );

    Ok(CheckTableSql {
        sql,
        rule_count: compiled.predicates.len(),
        skipped: compiled.skipped,
    })
}

fn render_branch(key_cols_csv: &str, table_ref: &str, p: &RulePredicate) -> String {
    let rule_lit = sql_dialect::quote_literal(&p.rule_id);
    let reason_lit = sql_dialect::quote_literal(&p.reason);
    format!(
        "  select {key_cols_csv}, {rule_lit} as rule, {reason_lit} as reason\n  from {table_ref}\n  where {predicate}",
        predicate = p.predicate_sql,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> Contract {
        serde_yaml::from_str(
            r#"
version: "1.0"
name: orders
ontology:
  entities:
    - name: order_id
      type: string
      required: true
    - name: amount_usd
      type: number
      required: true
      min: 0
"#,
        )
        .unwrap()
    }

    #[test]
    fn requires_at_least_one_key_column() {
        let err = generate_check_sql(&contract(), SqlDialect::Postgres, "orders", &[]).unwrap_err();
        assert!(err.contains("--key"));
    }

    #[test]
    fn unions_one_branch_per_rule() {
        let keys = vec!["order_id".to_string()];
        let out = generate_check_sql(&contract(), SqlDialect::Postgres, "orders", &keys).unwrap();
        // required:order_id, required:amount_usd, range:amount_usd
        assert_eq!(out.rule_count, 3);
        assert!(out.sql.contains("union all"));
        assert!(out.sql.contains("order by \"order_id\", rule"));
    }
}
