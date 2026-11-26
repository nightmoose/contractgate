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

use crate::contract::Contract;
pub use crate::contract_diff::{diff_contracts, DiffChange};
use crate::error::{AppError, AppResult};
use axum::Json;

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
    let contract_a: Contract = crate::odcs::parse_contract_yaml(&req.contract_yaml_a)
        .map_err(|e| AppError::BadRequest(format!("invalid contract_yaml_a: {e}")))?;

    let contract_b: Contract = crate::odcs::parse_contract_yaml(&req.contract_yaml_b)
        .map_err(|e| AppError::BadRequest(format!("invalid contract_yaml_b: {e}")))?;

    let changes = diff_contracts(&contract_a, &contract_b);
    let summarizer = RuleBasedSummarizer;
    let summary = summarizer.summarize(&changes);

    Ok(Json(DiffResponse { summary, changes }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{EgressLeakageMode, FieldDefinition, FieldType, Ontology};

    fn make_contract(fields: Vec<FieldDefinition>) -> Contract {
        Contract {
            version: "1.0".to_string(),
            name: "test".to_string(),
            description: None,
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            ontology: Ontology { entities: fields },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            envelope: None,
            null_as_absent: false,
            on_fail: crate::contract::OnFailMode::Quarantine,
            on_fail_locked: false,
            quarantine_guard: crate::contract::QuarantineGuardConfig::default(),
            cross_field: Vec::new(),
        }
    }

    fn simple_field(name: &str, ft: FieldType, required: bool) -> FieldDefinition {
        FieldDefinition {
            distribution: None,
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
            format: None,
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
