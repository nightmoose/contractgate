//! Three-way contract merge for incremental re-scaffold (RFC-024 §E).
//!
//! Merges at field granularity, not YAML text level.  Preserves human edits;
//! accepts scaffold-side updates when no human edit conflicts.
//!
//! Note: merge semantics are implemented here but the CLI uses plain re-scaffold
//! in the MVP (Phase 1).  This module will be wired up in Phase 2.
//!
//! Developer tooling — not part of the patent-core validation engine.

use crate::contract::{Contract, FieldDefinition, Ontology};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A conflict detected during three-way merge.
#[derive(Debug, Clone)]
pub struct MergeConflict {
    pub field_name: String,
    /// Description of what changed on each side.
    pub description: String,
}

/// Outcome of a three-way merge.
#[derive(Debug)]
pub struct MergeResult {
    /// The merged contract.  Conflicts are preserved as `ours` (human side).
    pub contract: Contract,
    /// Fields where both sides diverged from base simultaneously.
    pub conflicts: Vec<MergeConflict>,
    /// Fields that disappeared from the new scaffold (topic-side schema drift).
    pub drift_removed: Vec<String>,
    /// Fields that are new in the scaffold (additions since last run).
    pub drift_added: Vec<String>,
}

impl MergeResult {
    /// True when there are no conflicts and no removed-field drift.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.drift_removed.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Core algorithm
// ---------------------------------------------------------------------------

/// Perform a field-level three-way merge.
///
/// Arguments:
/// - `base`  — the contract emitted by the **previous** scaffold run (the
///   common ancestor).  `None` if no previous scaffold exists — treats every
///   difference between `ours` and `theirs` as a conflict.
/// - `ours`  — the current live contract (may contain human edits).
/// - `theirs`— the new scaffold output.
///
/// Returns a `MergeResult` with the merged contract and a list of conflicts.
pub fn three_way_merge(base: Option<&Contract>, ours: &Contract, theirs: &Contract) -> MergeResult {
    let base_map = base.map(|c| build_field_map(&c.ontology.entities));
    let ours_map = build_field_map(&ours.ontology.entities);
    let theirs_map = build_field_map(&theirs.ontology.entities);

    // All field names across all three versions.
    let mut all_names: Vec<String> = {
        let mut s: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if let Some(ref bm) = base_map {
            s.extend(bm.keys().map(|k| k.to_string()));
        }
        s.extend(ours_map.keys().map(|k| k.to_string()));
        s.extend(theirs_map.keys().map(|k| k.to_string()));
        s.into_iter().collect()
    };
    all_names.sort();

    let mut result_fields: Vec<FieldDefinition> = Vec::new();
    let mut conflicts: Vec<MergeConflict> = Vec::new();
    let mut drift_removed: Vec<String> = Vec::new();
    let mut drift_added: Vec<String> = Vec::new();

    for name in &all_names {
        let base_f = base_map.as_ref().and_then(|m| m.get(name.as_str()));
        let ours_f = ours_map.get(name.as_str());
        let theirs_f = theirs_map.get(name.as_str());

        match (base_f, ours_f, theirs_f) {
            // Field exists in all three — standard merge.
            (Some(b), Some(o), Some(t)) => {
                if field_equivalent(o, b) {
                    // No human edit — accept scaffold side.
                    result_fields.push((*t).clone());
                } else if field_equivalent(t, b) {
                    // No scaffold change — preserve human edit.
                    result_fields.push((*o).clone());
                } else {
                    // Both diverged from base — conflict.
                    conflicts.push(MergeConflict {
                        field_name: name.clone(),
                        description: format!(
                            "both human edits and scaffold changes detected; \
                             preserving human version"
                        ),
                    });
                    result_fields.push((*o).clone());
                }
            }

            // Field is new in scaffold (didn't exist in base or ours).
            (None, None, Some(t)) => {
                drift_added.push(name.clone());
                result_fields.push((*t).clone());
            }

            // Field is in both ours and theirs but not in base — new on both sides, keep ours.
            (None, Some(o), Some(_)) => {
                result_fields.push((*o).clone());
            }

            // Field was in ours but disappeared from scaffold — flag as drift, preserve ours.
            (_, Some(o), None) => {
                drift_removed.push(name.clone());
                result_fields.push((*o).clone());
            }

            // Field existed in base but was deleted in both ours and theirs.
            (Some(_), None, None) => {
                // Deleted on both sides — omit.
            }

            // Field was in base and theirs but human deleted it — honour deletion.
            (Some(_), None, Some(_)) => {
                // Human deleted it intentionally — don't resurrect.
            }

            // Impossible (None, None, None) — all_names only includes seen fields.
            (None, None, None) => {}
        }
    }

    // Re-use the top-level metadata from ours (human-edited values preserved).
    let merged = Contract {
        version: ours.version.clone(),
        name: ours.name.clone(),
        description: ours.description.clone(),
        compliance_mode: ours.compliance_mode,
        ontology: Ontology {
            entities: result_fields,
        },
        // Preserve human glossary / metrics entirely (not scaffold-managed).
        glossary: ours.glossary.clone(),
        metrics: ours.metrics.clone(),
        quality: ours.quality.clone(),
    };

    MergeResult {
        contract: merged,
        conflicts,
        drift_removed,
        drift_added,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_field_map<'a>(fields: &'a [FieldDefinition]) -> HashMap<&'a str, &'a FieldDefinition> {
    fields.iter().map(|f| (f.name.as_str(), f)).collect()
}

/// Two FieldDefinitions are "equivalent" for merge purposes when their
/// structurally significant fields match.  We ignore doc-only fields
/// (currently: none — all fields are structural).
fn field_equivalent(a: &FieldDefinition, b: &FieldDefinition) -> bool {
    a.name == b.name
        && a.field_type == b.field_type
        && a.required == b.required
        && a.pattern == b.pattern
        && a.allowed_values == b.allowed_values
        && ordered_f64_eq(a.min, b.min)
        && ordered_f64_eq(a.max, b.max)
        && a.min_length == b.min_length
        && a.max_length == b.max_length
}

fn ordered_f64_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.to_bits() == y.to_bits(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{FieldType, Ontology};

    fn make_field(name: &str, ft: FieldType) -> FieldDefinition {
        FieldDefinition {
            name: name.to_string(),
            field_type: ft,
            required: true,
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

    fn make_contract(fields: Vec<FieldDefinition>) -> Contract {
        Contract {
            version: "1.0".to_string(),
            name: "test".to_string(),
            description: None,
            compliance_mode: false,
            ontology: Ontology { entities: fields },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
        }
    }

    /// merge(base, ours=base, theirs) == theirs — no human edit → accept scaffold.
    #[test]
    fn no_human_edit_accepts_scaffold() {
        let base = make_contract(vec![make_field("x", FieldType::String)]);
        let ours = base.clone();
        let theirs = make_contract(vec![make_field("x", FieldType::Integer)]);
        let result = three_way_merge(Some(&base), &ours, &theirs);
        assert!(result.conflicts.is_empty());
        assert_eq!(
            result.contract.ontology.entities[0].field_type,
            FieldType::Integer
        );
    }

    /// merge(base, ours, theirs=base) == ours — no schema change → preserve human edit.
    #[test]
    fn no_schema_change_preserves_human_edit() {
        let base = make_contract(vec![make_field("x", FieldType::String)]);
        let mut ours = base.clone();
        ours.ontology.entities[0].required = false; // human edited required flag
        let theirs = base.clone();
        let result = three_way_merge(Some(&base), &ours, &theirs);
        assert!(result.conflicts.is_empty());
        assert!(!result.contract.ontology.entities[0].required);
    }

    /// New field in scaffold → drift_added.
    #[test]
    fn new_scaffold_field_flagged_as_added() {
        let base = make_contract(vec![make_field("x", FieldType::String)]);
        let ours = base.clone();
        let theirs = make_contract(vec![
            make_field("x", FieldType::String),
            make_field("y", FieldType::Integer),
        ]);
        let result = three_way_merge(Some(&base), &ours, &theirs);
        assert!(result.drift_added.contains(&"y".to_string()));
        assert_eq!(result.contract.ontology.entities.len(), 2);
    }

    /// Field disappears from scaffold → drift_removed, ours preserved.
    #[test]
    fn removed_scaffold_field_flagged_as_drift() {
        let base = make_contract(vec![
            make_field("x", FieldType::String),
            make_field("y", FieldType::Integer),
        ]);
        let ours = base.clone();
        let theirs = make_contract(vec![make_field("x", FieldType::String)]);
        let result = three_way_merge(Some(&base), &ours, &theirs);
        assert!(result.drift_removed.contains(&"y".to_string()));
        // Human side (ours) is preserved.
        assert_eq!(result.contract.ontology.entities.len(), 2);
    }

    /// Both sides changed → conflict, ours preserved.
    #[test]
    fn both_diverged_produces_conflict() {
        let base = make_contract(vec![make_field("x", FieldType::String)]);
        let mut ours = base.clone();
        ours.ontology.entities[0].required = false;
        let mut theirs = base.clone();
        theirs.ontology.entities[0].field_type = FieldType::Integer;
        let result = three_way_merge(Some(&base), &ours, &theirs);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].field_name, "x");
        // Conflict resolution: preserve ours.
        assert!(!result.contract.ontology.entities[0].required);
        assert_eq!(
            result.contract.ontology.entities[0].field_type,
            FieldType::String
        );
    }
}
