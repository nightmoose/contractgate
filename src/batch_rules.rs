//! Compiles a `Contract`'s rules into SQL predicates (RFC-119).
//!
//! Shared by `dbt_export` (singular dbt tests) and `batch_check`
//! (`check-table`'s violation-report query). Every predicate is a boolean
//! SQL expression that is TRUE for a row that *violates* the rule, built
//! from bare (dialect-quoted) column names so it can be dropped into a
//! `WHERE` clause against the contract's table, however that table is
//! referenced (`orders`, `"ORDERS"`, or dbt's `{{ ref('orders') }}`).
//!
//! Only top-level scalar fields are covered. Nested objects, arrays, and
//! `[]`-wildcard per-item rules are a flat-table concept gap, not a bug —
//! see RFC-119 — and are reported in `CompiledRules::skipped` instead of
//! silently dropped.

use crate::contract::{Contract, FieldDefinition, FieldType, QualityRuleType};
use crate::sql_dialect::{self, SqlDialect};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleCategory {
    /// Maps to a dbt generic `not_null` test.
    Required,
    /// Maps to a dbt generic `accepted_values` test.
    Enum,
    /// Maps to a dbt generic `unique` test. In `check-table` this becomes a
    /// `GROUP BY ... HAVING count(*) > 1` predicate, since there is no dbt
    /// test framework to lean on.
    Unique,
    Pattern,
    Format,
    Range,
    Length,
    Freshness,
    CrossField,
}

#[derive(Debug, Clone)]
pub struct RulePredicate {
    /// Stable id, e.g. `pattern:order_id` — used for test/file naming.
    pub rule_id: String,
    pub category: RuleCategory,
    /// Field this predicate is about (dotted path for cross-field rules).
    pub field: String,
    /// Plain-English reason shown in `check-table` violation reports.
    pub reason: String,
    /// Boolean SQL expression, true for a violating row. References bare
    /// column names only, except `Unique`, which references `table_ref`.
    pub predicate_sql: String,
}

#[derive(Debug, Clone)]
pub struct SkippedRule {
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledRules {
    pub predicates: Vec<RulePredicate>,
    pub skipped: Vec<SkippedRule>,
}

/// Compile every rule in `contract` into SQL predicates for `dialect`.
///
/// `table_ref` is how the table is referenced in SQL — a plain quoted
/// identifier for `check-table`, or a dbt `{{ ref('model') }}` / `{{
/// source(...) }}` call for the generated dbt tests. It is only used by the
/// `Unique` predicate, which needs a self-join subquery.
pub fn compile(contract: &Contract, dialect: SqlDialect, table_ref: &str) -> CompiledRules {
    let mut out = CompiledRules::default();

    for field in &contract.ontology.entities {
        compile_field(field, dialect, &mut out);
    }

    for rule in &contract.quality {
        if rule.field.contains('[') {
            out.skipped.push(SkippedRule {
                field: rule.field.clone(),
                reason: "per-item (`[]`) quality rule; batch SQL operates on flat table columns"
                    .to_string(),
            });
            continue;
        }
        if rule.field.contains('.') {
            out.skipped.push(SkippedRule {
                field: rule.field.clone(),
                reason: "nested field path; batch SQL operates on flat table columns".to_string(),
            });
            continue;
        }
        let col = sql_dialect::quote_ident(dialect, &rule.field);
        match rule.rule_type {
            QualityRuleType::Completeness => {
                out.predicates.push(RulePredicate {
                    rule_id: format!("completeness:{}", rule.field),
                    category: RuleCategory::Required,
                    field: rule.field.clone(),
                    reason: format!("{} is required but missing or empty", rule.field),
                    predicate_sql: format!("({col} is null or {col} = '')"),
                });
            }
            QualityRuleType::Validity => {
                // Already covered by the ontology-level pattern/enum/range
                // predicates on this field — see RFC-118's own doc comment
                // on `QualityRuleType::Validity`. No separate predicate.
            }
            QualityRuleType::Freshness => match rule.max_age_seconds {
                Some(max_age) => {
                    let cutoff = sql_dialect::now_minus_seconds(dialect, max_age);
                    out.predicates.push(RulePredicate {
                        rule_id: format!("freshness:{}", rule.field),
                        category: RuleCategory::Freshness,
                        field: rule.field.clone(),
                        reason: format!(
                            "{} is older than the freshness window ({} seconds)",
                            rule.field, max_age
                        ),
                        predicate_sql: format!("({col} is not null and {col} < {cutoff})"),
                    });
                }
                None => out.skipped.push(SkippedRule {
                    field: rule.field.clone(),
                    reason: "freshness rule has no max_age_seconds".to_string(),
                }),
            },
            QualityRuleType::Uniqueness => {
                out.predicates.push(RulePredicate {
                    rule_id: format!("unique:{}", rule.field),
                    category: RuleCategory::Unique,
                    field: rule.field.clone(),
                    reason: format!("{} is duplicated across rows", rule.field),
                    predicate_sql: format!(
                        "({col} is not null and {col} in (select {col} from {table_ref} group by {col} having count(*) > 1))"
                    ),
                });
            }
        }
    }

    for rule in &contract.cross_field {
        if rule.wildcard_array().is_some() {
            out.skipped.push(SkippedRule {
                field: rule.field.clone(),
                reason:
                    "per-item (`[]`) cross-field rule; batch SQL operates on flat table columns"
                        .to_string(),
            });
            continue;
        }
        let Some(predicate_sql) = cross_field_predicate_sql(rule, dialect) else {
            out.skipped.push(SkippedRule {
                field: rule.field.clone(),
                reason: "malformed cross_field rule (see check_shape)".to_string(),
            });
            continue;
        };
        out.predicates.push(RulePredicate {
            rule_id: format!("cross_field:{}", rule.field),
            category: RuleCategory::CrossField,
            field: rule.field.clone(),
            reason: format!("cross-field rule violated: {}", rule.label()),
            predicate_sql,
        });
    }

    out
}

fn compile_field(field: &FieldDefinition, dialect: SqlDialect, out: &mut CompiledRules) {
    if matches!(field.field_type, FieldType::Object | FieldType::Array) {
        out.skipped.push(SkippedRule {
            field: field.name.clone(),
            reason: format!(
                "{} field; batch SQL operates on flat table columns",
                if field.field_type == FieldType::Object {
                    "object"
                } else {
                    "array"
                }
            ),
        });
        return;
    }

    let col = sql_dialect::quote_ident(dialect, &field.name);

    if field.required {
        out.predicates.push(RulePredicate {
            rule_id: format!("required:{}", field.name),
            category: RuleCategory::Required,
            field: field.name.clone(),
            reason: format!("{} is required but missing", field.name),
            predicate_sql: format!("{col} is null"),
        });
    }

    if let Some(pattern) = &field.pattern {
        let is_match = sql_dialect::regex_match(dialect, &col, pattern);
        out.predicates.push(RulePredicate {
            rule_id: format!("pattern:{}", field.name),
            category: RuleCategory::Pattern,
            field: field.name.clone(),
            reason: format!("{} does not match pattern {pattern}", field.name),
            predicate_sql: format!("({col} is not null and not ({is_match}))"),
        });
    }

    if let Some(fmt) = &field.format {
        match format_regex(fmt) {
            Some(pattern) => {
                let is_match = sql_dialect::regex_match(dialect, &col, pattern);
                out.predicates.push(RulePredicate {
                    rule_id: format!("format:{}", field.name),
                    category: RuleCategory::Format,
                    field: field.name.clone(),
                    reason: format!("{} is not a valid {fmt}", field.name),
                    predicate_sql: format!("({col} is not null and not ({is_match}))"),
                });
            }
            None => out.skipped.push(SkippedRule {
                field: field.name.clone(),
                reason: format!("format '{fmt}' has no batch SQL equivalent"),
            }),
        }
    }

    if let Some(values) = &field.allowed_values {
        match enum_literal_list(values) {
            Some(literals) if !literals.is_empty() => {
                let list = literals.join(", ");
                out.predicates.push(RulePredicate {
                    rule_id: format!("enum:{}", field.name),
                    category: RuleCategory::Enum,
                    field: field.name.clone(),
                    reason: format!("{} is not one of the allowed values", field.name),
                    predicate_sql: format!("({col} is not null and {col} not in ({list}))"),
                });
            }
            _ => out.skipped.push(SkippedRule {
                field: field.name.clone(),
                reason: "enum contains non-scalar values".to_string(),
            }),
        }
    }

    if field.min.is_some() || field.max.is_some() {
        let mut bounds = Vec::new();
        if let Some(min) = field.min {
            bounds.push(format!("{col} < {min}"));
        }
        if let Some(max) = field.max {
            bounds.push(format!("{col} > {max}"));
        }
        let reason = match (field.min, field.max) {
            (Some(min), Some(max)) => format!("{} must be between {min} and {max}", field.name),
            (Some(min), None) => format!("{} must be at least {min}", field.name),
            (None, Some(max)) => format!("{} must be at most {max}", field.name),
            (None, None) => unreachable!(),
        };
        out.predicates.push(RulePredicate {
            rule_id: format!("range:{}", field.name),
            category: RuleCategory::Range,
            field: field.name.clone(),
            reason,
            predicate_sql: format!("({col} is not null and ({}))", bounds.join(" or ")),
        });
    }

    if field.min_length.is_some() || field.max_length.is_some() {
        let len = sql_dialect::length(&col);
        let mut bounds = Vec::new();
        if let Some(min_len) = field.min_length {
            bounds.push(format!("{len} < {min_len}"));
        }
        if let Some(max_len) = field.max_length {
            bounds.push(format!("{len} > {max_len}"));
        }
        let reason = match (field.min_length, field.max_length) {
            (Some(min_len), Some(max_len)) => {
                format!(
                    "{} length must be between {min_len} and {max_len}",
                    field.name
                )
            }
            (Some(min_len), None) => format!("{} length must be at least {min_len}", field.name),
            (None, Some(max_len)) => format!("{} length must be at most {max_len}", field.name),
            (None, None) => unreachable!(),
        };
        out.predicates.push(RulePredicate {
            rule_id: format!("length:{}", field.name),
            category: RuleCategory::Length,
            field: field.name.clone(),
            reason,
            predicate_sql: format!("({col} is not null and ({}))", bounds.join(" or ")),
        });
    }
}

/// Regex approximation for ODCS string formats. Matches the common case;
/// the online gateway's `string_format_ok` (src/validation.rs) uses
/// non-regex logic for a few edge cases (e.g. a trailing dot domain), so a
/// row this predicate rejects is always a real problem, but the predicate
/// may be slightly looser than the live engine at the margins.
fn format_regex(fmt: &str) -> Option<&'static str> {
    match fmt.to_ascii_lowercase().as_str() {
        "email" => Some(r"^[^@\s]+@[^@\s]+\.[^@\s]+$"),
        "uuid" => {
            Some(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
        }
        "uri" | "url" => Some(r"^[a-zA-Z][a-zA-Z0-9+.-]*:.+$"),
        _ => None,
    }
}

/// SQL literal list for an `enum:` constraint. `None` when any value is an
/// array/object (no scalar SQL literal exists for those).
fn enum_literal_list(values: &[Value]) -> Option<Vec<String>> {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => Some(sql_dialect::quote_literal(s)),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }),
            Value::Null => Some("NULL".to_string()),
            Value::Array(_) | Value::Object(_) => None,
        })
        .collect()
}

fn sql_compare_op(op: crate::contract::CompareOp) -> &'static str {
    use crate::contract::CompareOp;
    match op {
        CompareOp::Lt => "<",
        CompareOp::Lte => "<=",
        CompareOp::Gt => ">",
        CompareOp::Gte => ">=",
        CompareOp::Eq => "=",
        CompareOp::Ne => "<>",
    }
}

fn json_scalar_to_sql(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(sql_dialect::quote_literal(s)),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b {
            "TRUE".to_string()
        } else {
            "FALSE".to_string()
        }),
        Value::Null => None,
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn cross_field_predicate_sql(
    rule: &crate::contract::CrossFieldRule,
    dialect: SqlDialect,
) -> Option<String> {
    let field_col = sql_dialect::quote_ident(dialect, &rule.field);
    if let (Some(op), Some(other)) = (&rule.op, &rule.other) {
        let other_col = sql_dialect::quote_ident(dialect, other);
        let sql_op = sql_compare_op(*op);
        return Some(format!(
            "({field_col} is not null and {other_col} is not null and not ({field_col} {sql_op} {other_col}))"
        ));
    }
    if let Some(cond) = &rule.required_if {
        let cond_col = sql_dialect::quote_ident(dialect, &cond.field);
        return match &cond.equals {
            Value::Null => Some(format!("({cond_col} is null and {field_col} is null)")),
            other => {
                let literal = json_scalar_to_sql(other)?;
                Some(format!("({cond_col} = {literal} and {field_col} is null)"))
            }
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Contract, FieldDefinition, Ontology};

    fn minimal_contract(entities: Vec<FieldDefinition>) -> Contract {
        serde_yaml::from_str(
            r#"
version: "1.0"
name: test
ontology:
  entities: []
"#,
        )
        .map(|mut c: Contract| {
            c.ontology = Ontology { entities };
            c
        })
        .unwrap()
    }

    fn field(name: &str, field_type: FieldType) -> FieldDefinition {
        FieldDefinition {
            name: name.to_string(),
            field_type,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
            format: None,
        }
    }

    #[test]
    fn required_field_produces_is_null_predicate() {
        let mut f = field("order_id", FieldType::String);
        f.required = true;
        let contract = minimal_contract(vec![f]);
        let compiled = batch_rules_compile_for_test(&contract, SqlDialect::Postgres);
        let p = compiled
            .predicates
            .iter()
            .find(|p| p.rule_id == "required:order_id")
            .unwrap();
        assert_eq!(p.predicate_sql, "\"order_id\" is null");
    }

    #[test]
    fn object_and_array_fields_are_skipped_not_dropped_silently() {
        let obj = field("meta", FieldType::Object);
        let arr = field("lines", FieldType::Array);
        let contract = minimal_contract(vec![obj, arr]);
        let compiled = batch_rules_compile_for_test(&contract, SqlDialect::Postgres);
        assert!(compiled.predicates.is_empty());
        assert_eq!(compiled.skipped.len(), 2);
    }

    #[test]
    fn enum_predicate_quotes_string_literals() {
        let mut f = field("status", FieldType::String);
        f.allowed_values = Some(vec![
            Value::String("paid".into()),
            Value::String("void".into()),
        ]);
        let contract = minimal_contract(vec![f]);
        let compiled = batch_rules_compile_for_test(&contract, SqlDialect::Postgres);
        let p = compiled
            .predicates
            .iter()
            .find(|p| p.rule_id == "enum:status")
            .unwrap();
        assert_eq!(
            p.predicate_sql,
            "(\"status\" is not null and \"status\" not in ('paid', 'void'))"
        );
    }

    #[test]
    fn range_predicate_uses_both_bounds() {
        let mut f = field("amount", FieldType::Float);
        f.min = Some(0.0);
        let contract = minimal_contract(vec![f]);
        let compiled = batch_rules_compile_for_test(&contract, SqlDialect::DuckDb);
        let p = compiled
            .predicates
            .iter()
            .find(|p| p.rule_id == "range:amount")
            .unwrap();
        assert_eq!(
            p.predicate_sql,
            "(\"amount\" is not null and (\"amount\" < 0))"
        );
    }

    #[test]
    fn databricks_pattern_uses_rlike() {
        let mut f = field("sku", FieldType::String);
        f.pattern = Some("^[A-Z]{3}$".to_string());
        let contract = minimal_contract(vec![f]);
        let compiled = batch_rules_compile_for_test(&contract, SqlDialect::Databricks);
        let p = compiled
            .predicates
            .iter()
            .find(|p| p.rule_id == "pattern:sku")
            .unwrap();
        assert!(p.predicate_sql.contains("rlike"));
        assert!(p.predicate_sql.contains('`'));
    }

    fn batch_rules_compile_for_test(contract: &Contract, dialect: SqlDialect) -> CompiledRules {
        compile(contract, dialect, "orders")
    }
}
