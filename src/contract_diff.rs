//! Pure contract diff: the field-level changes between two contracts.
//!
//! Shared by `POST /contracts/diff` (`infer_diff`) and the `diff` CLI command.
//! Lives in the library so the CLI does not need the server stack.

use crate::contract::{Contract, FieldDefinition};
use serde_json::Value;
use std::collections::HashMap;

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
