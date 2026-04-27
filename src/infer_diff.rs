//! Contract evolution diff summarizer.
//!
//! `POST /contracts/diff`
//!
//! Accepts two contract YAML strings (older `a`, newer `b`) and returns a
//! structured list of changes plus a plain-English summary sentence.
//!
//! ## Change kinds
//!
//! | Kind | Trigger |
//! |------|---------|
//! | `field_added` | field in b not in a |
//! | `field_removed` | field in a not in b |
//! | `type_changed` | same field, different `field_type` |
//! | `required_changed` | same field, `required` flipped |
//! | `enum_value_added` | value in b's `allowed_values` not in a |
//! | `enum_value_removed` | value in a's `allowed_values` not in b |
//! | `pattern_changed` | same field, different `pattern` |
//! | `constraint_changed` | min / max / min_length / max_length changed |
//!
//! ## Extension point
//!
//! `DiffSummarizer` is a trait so a future LLM backend can be injected
//! without touching the handler.  The default is `RuleBasedSummarizer`.

use crate::contract::{Contract, FieldDefinition};
use crate::error::{AppError, AppResult};
use axum::Json;
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct DiffRequest {
    /// Older / baseline contract YAML.
    pub contract_yaml_a: String,
    /// Newer / proposed contract YAML.
    pub contract_yaml_b: String,
}

#[derive(serde::Serialize)]
pub struct DiffResponse {
    /// Human-readable summary sentence.
    pub summary: String,
    /// Structured list of individual changes.
    pub changes: Vec<DiffChange>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DiffChange {
    /// One of the `kind` values documented above.
    pub kind: String,
    /// Dot-path of the affected field (e.g. `"meta.env"`).
    pub field: String,
    /// Human-readable detail (e.g. `"integer → float"`).
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Summarizer trait
// ---------------------------------------------------------------------------

/// Produce a plain-English summary sentence from a list of `DiffChange`s.
pub trait DiffSummarizer: Send + Sync {
    fn summarize(&self, changes: &[DiffChange]) -> String;
}

/// Rule-based summarizer — shipped by default.
pub struct RuleBasedSummarizer;

impl DiffSummarizer for RuleBasedSummarizer {
    fn summarize(&self, changes: &[DiffChange]) -> String {
        if changes.is_empty() {
            return "No changes detected between the two contract versions.".to_string();
        }

        // Count by kind category.
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut type_changed = 0usize;
        let mut required_changed = 0usize;
        let mut other = 0usize;

        for c in changes {
            match c.kind.as_str() {
                "field_added" => added += 1,
                "field_removed" => removed += 1,
                "type_changed" => type_changed += 1,
                "required_changed" => required_changed += 1,
                _ => other += 1,
            }
        }

        let total = changes.len();
        let mut parts: Vec<String> = Vec::new();
        if added > 0 {
            parts.push(format!(
                "{added} field{} added",
                if added == 1 { "" } else { "s" }
            ));
        }
        if removed > 0 {
            parts.push(format!(
                "{removed} field{} removed",
                if removed == 1 { "" } else { "s" }
            ));
        }
        if type_changed > 0 {
            parts.push(format!(
                "{type_changed} type change{}",
                if type_changed == 1 { "" } else { "s" }
            ));
        }
        if required_changed > 0 {
            parts.push(format!(
                "{required_changed} required-flag change{}",
                if required_changed == 1 { "" } else { "s" }
            ));
        }
        if other > 0 {
            parts.push(format!(
                "{other} constraint/enum change{}",
                if other == 1 { "" } else { "s" }
            ));
        }

        format!(
            "{total} change{}: {}.",
            if total == 1 { "" } else { "s" },
            parts.join(", ")
        )
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn diff_handler(Json(req): Json<DiffRequest>) -> AppResult<Json<DiffResponse>> {
    let contract_a: Contract = serde_yaml::from_str(&req.contract_yaml_a)
        .map_err(|e| AppError::BadRequest(format!("invalid contract_yaml_a: {e}")))?;

    let contract_b: Contract = serde_yaml::from_str(&req.contract_yaml_b)
        .map_err(|e| AppError::BadRequest(format!("invalid contract_yaml_b: {e}")))?;

    let changes = diff_contracts(&contract_a, &contract_b);
    let summarizer = RuleBasedSummarizer;
    let summary = summarizer.summarize(&changes);

    Ok(Json(DiffResponse { summary, changes }))
}

// ---------------------------------------------------------------------------
// Core diff logic
// ---------------------------------------------------------------------------

/// Compare two contracts and produce a flat list of `DiffChange`s.
pub fn diff_contracts(a: &Contract, b: &Contract) -> Vec<DiffChange> {
    let mut changes = Vec::new();
    diff_field_lists(&a.ontology.entities, &b.ontology.entities, "", &mut changes);
    changes
}

/// Recursively diff two lists of `FieldDefinition`s, building dot-path keys.
fn diff_field_lists(
    a_fields: &[FieldDefinition],
    b_fields: &[FieldDefinition],
    prefix: &str,
    changes: &mut Vec<DiffChange>,
) {
    let a_map: HashMap<&str, &FieldDefinition> =
        a_fields.iter().map(|f| (f.name.as_str(), f)).collect();
    let b_map: HashMap<&str, &FieldDefinition> =
        b_fields.iter().map(|f| (f.name.as_str(), f)).collect();

    // Fields present in a but absent in b → removed.
    for name in a_map.keys() {
        if !b_map.contains_key(name) {
            let f = a_map[name];
            let path = field_path(prefix, name);
            changes.push(DiffChange {
                kind: "field_removed".to_string(),
                field: path,
                detail: format!("{:?}, {}", f.field_type, required_label(f.required)),
            });
        }
    }

    // Fields present in b but absent in a → added.
    for name in b_map.keys() {
        if !a_map.contains_key(name) {
            let f = b_map[name];
            let path = field_path(prefix, name);
            changes.push(DiffChange {
                kind: "field_added".to_string(),
                field: path,
                detail: format!("{:?}, {}", f.field_type, required_label(f.required)),
            });
        }
    }

    // Fields present in both → compare.
    for name in a_map.keys() {
        if let Some(b_field) = b_map.get(name) {
            let a_field = a_map[name];
            let path = field_path(prefix, name);
            diff_single_field(a_field, b_field, &path, changes);
        }
    }
}

fn diff_single_field(
    a: &FieldDefinition,
    b: &FieldDefinition,
    path: &str,
    changes: &mut Vec<DiffChange>,
) {
    // Type change.
    if a.field_type != b.field_type {
        changes.push(DiffChange {
            kind: "type_changed".to_string(),
            field: path.to_string(),
            detail: format!("{:?} → {:?}", a.field_type, b.field_type),
        });
    }

    // Required flag change.
    if a.required != b.required {
        changes.push(DiffChange {
            kind: "required_changed".to_string(),
            field: path.to_string(),
            detail: format!(
                "{} → {}",
                required_label(a.required),
                required_label(b.required)
            ),
        });
    }

    // Pattern change.
    if a.pattern != b.pattern {
        changes.push(DiffChange {
            kind: "pattern_changed".to_string(),
            field: path.to_string(),
            detail: format!(
                "{} → {}",
                a.pattern.as_deref().unwrap_or("none"),
                b.pattern.as_deref().unwrap_or("none")
            ),
        });
    }

    // Numeric constraint changes.
    diff_opt_f64(a.min, b.min, "min", path, changes);
    diff_opt_f64(a.max, b.max, "max", path, changes);
    diff_opt_usize(a.min_length, b.min_length, "min_length", path, changes);
    diff_opt_usize(a.max_length, b.max_length, "max_length", path, changes);

    // Enum value changes.
    diff_enum_values(&a.allowed_values, &b.allowed_values, path, changes);

    // Recurse into nested properties.
    if let (Some(a_props), Some(b_props)) = (&a.properties, &b.properties) {
        diff_field_lists(a_props, b_props, path, changes);
    } else if a.properties.is_some() || b.properties.is_some() {
        // One side has properties, the other doesn't — already covered by
        // the type_changed detection above.
    }
}

fn diff_enum_values(
    a_vals: &Option<Vec<Value>>,
    b_vals: &Option<Vec<Value>>,
    path: &str,
    changes: &mut Vec<DiffChange>,
) {
    let a_set: std::collections::HashSet<String> = enum_set(a_vals);
    let b_set: std::collections::HashSet<String> = enum_set(b_vals);

    for v in b_set.difference(&a_set) {
        changes.push(DiffChange {
            kind: "enum_value_added".to_string(),
            field: path.to_string(),
            detail: v.clone(),
        });
    }
    for v in a_set.difference(&b_set) {
        changes.push(DiffChange {
            kind: "enum_value_removed".to_string(),
            field: path.to_string(),
            detail: v.clone(),
        });
    }
}

fn enum_set(vals: &Option<Vec<Value>>) -> std::collections::HashSet<String> {
    vals.as_ref()
        .map(|v| v.iter().map(|x| x.to_string()).collect())
        .unwrap_or_default()
}

fn diff_opt_f64(
    a: Option<f64>,
    b: Option<f64>,
    label: &str,
    path: &str,
    changes: &mut Vec<DiffChange>,
) {
    if a != b {
        changes.push(DiffChange {
            kind: "constraint_changed".to_string(),
            field: path.to_string(),
            detail: format!(
                "{label}: {} → {}",
                a.map(|v| v.to_string()).unwrap_or("none".to_string()),
                b.map(|v| v.to_string()).unwrap_or("none".to_string())
            ),
        });
    }
}

fn diff_opt_usize(
    a: Option<usize>,
    b: Option<usize>,
    label: &str,
    path: &str,
    changes: &mut Vec<DiffChange>,
) {
    if a != b {
        changes.push(DiffChange {
            kind: "constraint_changed".to_string(),
            field: path.to_string(),
            detail: format!(
                "{label}: {} → {}",
                a.map(|v| v.to_string()).unwrap_or("none".to_string()),
                b.map(|v| v.to_string()).unwrap_or("none".to_string())
            ),
        });
    }
}

fn field_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn required_label(required: bool) -> &'static str {
    if required {
        "required"
    } else {
        "optional"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{FieldType, Ontology};

    fn make_contract(fields: Vec<FieldDefinition>) -> Contract {
        Contract {
            version: "1.0".to_string(),
            name: "test".to_string(),
            description: None,
            compliance_mode: false,
            ontology: Ontology { entities: fields },
            glossary: vec![],
            metrics: vec![],
        }
    }

    fn simple_field(name: &str, ft: FieldType, required: bool) -> FieldDefinition {
        FieldDefinition {
            name: name.to_string(),
            field_type: ft,
            required,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
        }
    }

    #[test]
    fn detects_field_added() {
        let a = make_contract(vec![simple_field("user_id", FieldType::String, true)]);
        let b = make_contract(vec![
            simple_field("user_id", FieldType::String, true),
            simple_field("session_id", FieldType::String, false),
        ]);
        let changes = diff_contracts(&a, &b);
        assert!(changes
            .iter()
            .any(|c| c.kind == "field_added" && c.field == "session_id"));
    }

    #[test]
    fn detects_field_removed() {
        let a = make_contract(vec![
            simple_field("user_id", FieldType::String, true),
            simple_field("legacy_id", FieldType::String, false),
        ]);
        let b = make_contract(vec![simple_field("user_id", FieldType::String, true)]);
        let changes = diff_contracts(&a, &b);
        assert!(changes
            .iter()
            .any(|c| c.kind == "field_removed" && c.field == "legacy_id"));
    }

    #[test]
    fn detects_type_changed() {
        let a = make_contract(vec![simple_field("amount", FieldType::Integer, true)]);
        let b = make_contract(vec![simple_field("amount", FieldType::Float, true)]);
        let changes = diff_contracts(&a, &b);
        assert!(changes
            .iter()
            .any(|c| c.kind == "type_changed" && c.field == "amount"));
        let change = changes.iter().find(|c| c.kind == "type_changed").unwrap();
        assert_eq!(change.detail, "Integer → Float");
    }

    #[test]
    fn detects_required_changed() {
        let a = make_contract(vec![simple_field("amount", FieldType::Float, true)]);
        let b = make_contract(vec![simple_field("amount", FieldType::Float, false)]);
        let changes = diff_contracts(&a, &b);
        assert!(changes
            .iter()
            .any(|c| c.kind == "required_changed" && c.field == "amount"));
    }

    #[test]
    fn detects_enum_value_added_and_removed() {
        use serde_json::json;
        let mut fa = simple_field("status", FieldType::String, true);
        fa.allowed_values = Some(vec![json!("active"), json!("inactive")]);

        let mut fb = simple_field("status", FieldType::String, true);
        fb.allowed_values = Some(vec![json!("active"), json!("pending")]);

        let a = make_contract(vec![fa]);
        let b = make_contract(vec![fb]);
        let changes = diff_contracts(&a, &b);
        assert!(changes
            .iter()
            .any(|c| c.kind == "enum_value_added" && c.detail.contains("pending")));
        assert!(changes
            .iter()
            .any(|c| c.kind == "enum_value_removed" && c.detail.contains("inactive")));
    }

    #[test]
    fn detects_constraint_changed() {
        let mut fa = simple_field("amount", FieldType::Float, true);
        fa.min = Some(0.0);
        fa.max = Some(1000.0);

        let mut fb = simple_field("amount", FieldType::Float, true);
        fb.min = Some(0.0);
        fb.max = Some(9999.0);

        let a = make_contract(vec![fa]);
        let b = make_contract(vec![fb]);
        let changes = diff_contracts(&a, &b);
        assert!(changes.iter().any(|c| {
            c.kind == "constraint_changed" && c.field == "amount" && c.detail.contains("max")
        }));
    }

    #[test]
    fn no_changes_empty_diff() {
        let c = make_contract(vec![simple_field("id", FieldType::String, true)]);
        let changes = diff_contracts(&c, &c);
        assert!(changes.is_empty());
    }

    #[test]
    fn summarizer_correct_totals() {
        let changes = vec![
            DiffChange {
                kind: "field_added".to_string(),
                field: "x".to_string(),
                detail: "".to_string(),
            },
            DiffChange {
                kind: "field_removed".to_string(),
                field: "y".to_string(),
                detail: "".to_string(),
            },
            DiffChange {
                kind: "type_changed".to_string(),
                field: "z".to_string(),
                detail: "".to_string(),
            },
        ];
        let s = RuleBasedSummarizer.summarize(&changes);
        assert!(s.starts_with("3 changes"));
        assert!(s.contains("1 field added"));
        assert!(s.contains("1 field removed"));
        assert!(s.contains("1 type change"));
    }

    #[test]
    fn summarizer_no_changes() {
        let s = RuleBasedSummarizer.summarize(&[]);
        assert!(s.contains("No changes"));
    }
}
