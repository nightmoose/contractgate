//! Fork filter engine for public catalog exports (RFC-034).
//!
//! A `ForkFilter` is stored as JSONB in `contracts.fork_filter` and applied
//! server-side when a customer calls `POST /contracts/:id/export`.
//!
//! ## Filter shape (JSONB)
//!
//! ```json
//! {
//!   "fields": ["state", "B01003_001E"],   // null = retain all fields
//!   "predicates": [
//!     { "field": "state", "op": "eq",  "value": "06"   },
//!     { "field": "B01003_001E", "op": "gte", "value": 10000 }
//!   ]
//! }
//! ```
//!
//! ## Predicate ops
//!
//! | Op  | Semantics                               |
//! |-----|-----------------------------------------|
//! | eq  | equal (strings: case-insensitive trim)  |
//! | neq | not equal                               |
//! | gt  | greater than (numeric or string order)  |
//! | gte | ≥                                       |
//! | lt  | less than                               |
//! | lte | ≤                                       |
//! | in  | value is in the provided JSON array     |
//!
//! ## Execution order
//!
//! 1. Evaluate all predicates — row is dropped on the first mismatch.
//! 2. Apply field subsetting — only listed fields are retained.
//!
//! Subsetting runs *after* predicate evaluation so predicates can reference
//! fields that are not included in the final output.

use serde_json::Value;
use std::cmp::Ordering;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct ForkFilter {
    /// Field names to retain in the output row.  `None` = retain all.
    pub fields: Option<Vec<String>>,
    /// Row-level filters — all must pass for a row to be included.
    #[serde(default)]
    pub predicates: Vec<Predicate>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct Predicate {
    /// Name of the field to test.
    pub field: String,
    /// Comparison operator.
    pub op: Op,
    /// RHS value to compare against.
    pub value: Value,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
}

// ---------------------------------------------------------------------------
// Filter application
// ---------------------------------------------------------------------------

impl ForkFilter {
    /// Apply this filter to a single JSON object row.
    ///
    /// Returns `None` when the row is dropped by a failing predicate.
    /// Returns `Some(filtered_row)` otherwise, with field subsetting applied.
    pub fn apply(&self, row: &Value) -> Option<Value> {
        // 1. Predicate pass — drop row on first failure.
        for pred in &self.predicates {
            if !pred.matches(row) {
                return None;
            }
        }

        // 2. Field subsetting.
        match &self.fields {
            None => Some(row.clone()),
            Some(keep) => {
                let obj = row.as_object()?;
                let mut out = serde_json::Map::with_capacity(keep.len());
                for field in keep {
                    if let Some(v) = obj.get(field.as_str()) {
                        out.insert(field.clone(), v.clone());
                    }
                    // Field absent in row → omit silently (not an error).
                }
                Some(Value::Object(out))
            }
        }
    }

    /// Apply the filter to a batch of rows, dropping non-matching ones.
    /// Preserves order.
    pub fn apply_batch(&self, rows: &[Value]) -> Vec<Value> {
        rows.iter().filter_map(|r| self.apply(r)).collect()
    }
}

// ---------------------------------------------------------------------------
// Predicate matching
// ---------------------------------------------------------------------------

impl Predicate {
    fn matches(&self, row: &Value) -> bool {
        let field_val = match row.get(&self.field) {
            Some(v) if !v.is_null() => v,
            // Absent or null field: predicate fails (can't compare against nothing).
            _ => return false,
        };

        match &self.op {
            Op::Eq => json_eq(field_val, &self.value),
            Op::Neq => !json_eq(field_val, &self.value),
            Op::Gt => json_cmp(field_val, &self.value) == Some(Ordering::Greater),
            Op::Gte => matches!(
                json_cmp(field_val, &self.value),
                Some(Ordering::Greater) | Some(Ordering::Equal)
            ),
            Op::Lt => json_cmp(field_val, &self.value) == Some(Ordering::Less),
            Op::Lte => matches!(
                json_cmp(field_val, &self.value),
                Some(Ordering::Less) | Some(Ordering::Equal)
            ),
            Op::In => {
                if let Some(arr) = self.value.as_array() {
                    arr.iter().any(|v| json_eq(field_val, v))
                } else {
                    false
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

/// Equality: strings are trimmed and compared case-insensitively.
/// Numbers use serde_json's PartialEq (JSON canonical form).
fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(sa), Value::String(sb)) => sa.trim().eq_ignore_ascii_case(sb.trim()),
        // Cross-type numeric comparison: allow "06" == "06" etc, but also
        // allow integer JSON to match a string predicate value like "06".
        // Keep it simple: fall back to serde_json PartialEq.
        _ => a == b,
    }
}

/// Ordering: numeric first, then lexicographic for strings.
/// Returns `None` when types are incomparable.
fn json_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    if let (Some(fa), Some(fb)) = (a.as_f64(), b.as_f64()) {
        return fa.partial_cmp(&fb);
    }
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return Some(sa.trim().cmp(sb.trim()));
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> Value {
        json!({
            "state":    "06",
            "county":   "037",
            "NAME":     "Los Angeles County, California",
            "B01003_001E": 10039107i64,
            "B19013_001E": 71358i64
        })
    }

    // ---- field subsetting --------------------------------------------------

    #[test]
    fn retains_all_when_fields_none() {
        let f = ForkFilter::default();
        let out = f.apply(&row()).unwrap();
        assert_eq!(out.as_object().unwrap().len(), 5);
    }

    #[test]
    fn subsets_fields() {
        let f = ForkFilter {
            fields: Some(vec!["state".into(), "B01003_001E".into()]),
            predicates: vec![],
        };
        let out = f.apply(&row()).unwrap();
        let obj = out.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("state"));
        assert!(obj.contains_key("B01003_001E"));
        assert!(!obj.contains_key("NAME"));
    }

    #[test]
    fn missing_subset_field_omitted_silently() {
        let f = ForkFilter {
            fields: Some(vec!["state".into(), "nonexistent".into()]),
            predicates: vec![],
        };
        let out = f.apply(&row()).unwrap();
        let obj = out.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("state"));
    }

    // ---- eq predicate ------------------------------------------------------

    #[test]
    fn eq_string_matches() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::Eq,
                value: json!("06"),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    #[test]
    fn eq_string_no_match_drops_row() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::Eq,
                value: json!("01"),
            }],
        };
        assert!(f.apply(&row()).is_none());
    }

    #[test]
    fn eq_is_case_insensitive_for_strings() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "NAME".into(),
                op: Op::Eq,
                value: json!("los angeles county, california"),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    // ---- numeric comparisons -----------------------------------------------

    #[test]
    fn gte_numeric() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "B01003_001E".into(),
                op: Op::Gte,
                value: json!(1_000_000i64),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    #[test]
    fn lt_numeric_drops_row() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "B01003_001E".into(),
                op: Op::Lt,
                value: json!(1_000i64),
            }],
        };
        assert!(f.apply(&row()).is_none());
    }

    #[test]
    fn lte_exact_boundary() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "B01003_001E".into(),
                op: Op::Lte,
                value: json!(10039107i64),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    // ---- in predicate ------------------------------------------------------

    #[test]
    fn in_matches_array_member() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::In,
                value: json!(["06", "36", "48"]),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    #[test]
    fn in_no_match_drops_row() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::In,
                value: json!(["01", "02"]),
            }],
        };
        assert!(f.apply(&row()).is_none());
    }

    // ---- neq ---------------------------------------------------------------

    #[test]
    fn neq_passes_when_not_equal() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::Neq,
                value: json!("01"),
            }],
        };
        assert!(f.apply(&row()).is_some());
    }

    // ---- absent field ------------------------------------------------------

    #[test]
    fn predicate_on_absent_field_drops_row() {
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "nonexistent".into(),
                op: Op::Eq,
                value: json!("x"),
            }],
        };
        assert!(f.apply(&row()).is_none());
    }

    // ---- apply_batch -------------------------------------------------------

    #[test]
    fn batch_filter() {
        let rows = vec![
            json!({"state": "06", "pop": 1000000i64}),
            json!({"state": "01", "pop": 500000i64}),
            json!({"state": "06", "pop": 200000i64}),
        ];
        let f = ForkFilter {
            fields: None,
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::Eq,
                value: json!("06"),
            }],
        };
        let out = f.apply_batch(&rows);
        assert_eq!(out.len(), 2);
    }

    // ---- predicate + subsetting together -----------------------------------

    #[test]
    fn predicate_can_use_field_not_in_subset() {
        // Filter on 'state' but don't include it in output.
        let f = ForkFilter {
            fields: Some(vec!["B01003_001E".into()]),
            predicates: vec![Predicate {
                field: "state".into(),
                op: Op::Eq,
                value: json!("06"),
            }],
        };
        let out = f.apply(&row()).unwrap();
        let obj = out.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("B01003_001E"));
        assert!(!obj.contains_key("state"));
    }
}
