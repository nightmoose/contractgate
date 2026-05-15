//! Contract inference — derive a draft `Contract` from JSON samples.
//!
//! `POST /contracts/infer`
//!
//! Accepts one or more raw JSON event objects and returns a YAML contract
//! that describes their shape.  The result is a *draft* — callers should
//! review and refine before promoting to stable.
//!
//! ## What gets inferred
//!
//! | Observation                        | Output                          |
//! |------------------------------------|---------------------------------|
//! | Field absent from some samples     | `required: false`               |
//! | Field null in any sample           | `required: false`               |
//! | All values are integers            | `type: integer` + min/max       |
//! | All values are decimals            | `type: float` + min/max         |
//! | All values are booleans            | `type: boolean`                 |
//! | All values are objects             | `type: object` + properties     |
//! | All values are arrays              | `type: array` + items           |
//! | All strings match UUID shape       | `pattern: ^[0-9a-f]{8}-…$`     |
//! | All strings match ISO-8601 date    | `pattern: ^\d{4}-\d{2}-\d{2}$` |
//! | All strings match ISO-8601 dt      | `pattern: ^\d{4}-…T\d{2}:…$`   |
//! | ≤8 distinct string values, ≥2 seen | `enum: [...]`                   |
//! | Mixed types across samples         | `type: any`                     |

use crate::contract::{Contract, EgressLeakageMode, FieldDefinition, FieldType, Ontology};
use crate::error::{AppError, AppResult};
use axum::Json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct InferRequest {
    /// Name for the generated contract (used in the YAML `name` field).
    pub name: String,
    /// Optional description embedded in the contract.
    #[serde(default)]
    pub description: Option<String>,
    /// One or more raw JSON event objects to infer from.
    /// All samples must be JSON objects (not arrays or scalars).
    pub samples: Vec<Value>,
}

#[derive(serde::Serialize)]
pub struct InferResponse {
    /// Draft contract YAML — paste into `POST /contracts` or
    /// `POST /contracts/:id/versions`.
    pub yaml_content: String,
    /// Number of top-level fields discovered across all samples.
    pub field_count: usize,
    /// Number of samples used for inference.
    pub sample_count: usize,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_handler(Json(req): Json<InferRequest>) -> AppResult<Json<InferResponse>> {
    if req.samples.is_empty() {
        return Err(AppError::BadRequest(
            "at least one sample is required".into(),
        ));
    }

    for (i, s) in req.samples.iter().enumerate() {
        if !s.is_object() {
            return Err(AppError::BadRequest(format!(
                "sample[{i}] is not a JSON object"
            )));
        }
    }

    let entities = infer_fields_from_objects(&req.samples);
    let field_count = entities.len();

    let contract = Contract {
        version: "1.0".to_string(),
        name: req.name.clone(),
        description: req.description.clone(),
        compliance_mode: false,
        egress_leakage_mode: EgressLeakageMode::Off,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| AppError::Internal(format!("yaml serialisation failed: {e}")))?;

    Ok(Json(InferResponse {
        yaml_content,
        field_count,
        sample_count: req.samples.len(),
    }))
}

// ---------------------------------------------------------------------------
// Core inference
// ---------------------------------------------------------------------------

/// Public re-export for reuse by format-specific inference modules.
#[inline]
pub fn infer_fields_from_objects_pub(samples: &[Value]) -> Vec<FieldDefinition> {
    infer_fields_from_objects(samples)
}

/// Collect all values for each key across an object array, then build a
/// `FieldDefinition` per key.  Key order follows first-appearance.
fn infer_fields_from_objects(samples: &[Value]) -> Vec<FieldDefinition> {
    let total = samples.len();

    // Preserve first-appearance key order.
    let mut key_order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // key → vec of references into samples
    let mut field_values: HashMap<String, Vec<&Value>> = HashMap::new();

    for sample in samples {
        if let Some(obj) = sample.as_object() {
            for (k, v) in obj.iter() {
                if seen.insert(k.clone()) {
                    key_order.push(k.clone());
                }
                field_values.entry(k.clone()).or_default().push(v);
            }
        }
    }

    key_order
        .into_iter()
        .map(|key| {
            let values = field_values.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            infer_field(&key, values, total)
        })
        .collect()
}

/// Infer a single `FieldDefinition` from the observed values for one field.
///
/// `values`       — every `Value` seen for this field (across all samples).
/// `total_samples` — total number of samples (to detect missing occurrences).
fn infer_field(name: &str, values: &[&Value], total_samples: usize) -> FieldDefinition {
    // A field is required only if it appears in every sample AND is never null.
    let required = values.len() == total_samples && values.iter().all(|v| !v.is_null());

    // Strip nulls before type inference — null is the "absent" sentinel.
    let non_null: Vec<&Value> = values.iter().copied().filter(|v| !v.is_null()).collect();

    let field_type = infer_type(&non_null);

    let mut def = FieldDefinition {
        name: name.to_string(),
        field_type: field_type.clone(),
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
    };

    match &field_type {
        FieldType::String => {
            let strings: Vec<&str> = non_null.iter().filter_map(|v| v.as_str()).collect();
            refine_string(&mut def, &strings, total_samples);
        }
        FieldType::Integer => {
            let nums: Vec<i64> = non_null.iter().filter_map(|v| v.as_i64()).collect();
            if !nums.is_empty() {
                def.min = nums.iter().copied().min().map(|n| n as f64);
                def.max = nums.iter().copied().max().map(|n| n as f64);
            }
        }
        FieldType::Float => {
            let nums: Vec<f64> = non_null.iter().filter_map(|v| v.as_f64()).collect();
            if !nums.is_empty() {
                let min = nums.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                def.min = Some(min);
                def.max = Some(max);
            }
        }
        FieldType::Object => {
            // Collect sub-objects and recurse.
            let sub_objects: Vec<Value> = non_null
                .iter()
                .filter(|v| v.is_object())
                .map(|v| (*v).clone())
                .collect();
            if !sub_objects.is_empty() {
                def.properties = Some(infer_fields_from_objects(&sub_objects));
            }
        }
        FieldType::Array => {
            // Collect all elements across all array values; infer a single
            // items definition from their union.
            let all_elements: Vec<Value> = non_null
                .iter()
                .filter_map(|v| v.as_array())
                .flat_map(|arr| arr.iter().cloned())
                .collect();
            if !all_elements.is_empty() {
                let elem_refs: Vec<&Value> = all_elements.iter().collect();
                let elem_type = infer_type(&elem_refs);
                let items_def = FieldDefinition {
                    name: "item".to_string(),
                    field_type: elem_type,
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
                };
                def.items = Some(Box::new(items_def));
            }
        }
        // Boolean / Any — no additional refinements.
        _ => {}
    }

    def
}

// ---------------------------------------------------------------------------
// Type detection
// ---------------------------------------------------------------------------

/// Derive the most specific `FieldType` consistent with *all* non-null values.
/// Falls back to `Any` when the observed values span multiple types.
fn infer_type(non_null: &[&Value]) -> FieldType {
    if non_null.is_empty() {
        // All occurrences were null — field exists but carries no type info.
        return FieldType::Any;
    }

    let all_bool = non_null.iter().all(|v| v.is_boolean());
    if all_bool {
        return FieldType::Boolean;
    }

    let all_number = non_null.iter().all(|v| v.is_number());
    if all_number {
        // Use Integer only if every number round-trips through i64 exactly.
        let all_int = non_null.iter().all(|v| {
            v.as_i64()
                .map(|i| {
                    // Guard against JSON numbers like 1.0 that serde_json
                    // parses as f64 but whose as_i64() also succeeds — ensure
                    // the f64 representation is identical to avoid promoting
                    // floats that happen to be whole numbers.
                    v.as_f64().map(|f| f == i as f64).unwrap_or(true)
                })
                .unwrap_or(false)
        });
        return if all_int {
            FieldType::Integer
        } else {
            FieldType::Float
        };
    }

    if non_null.iter().all(|v| v.is_string()) {
        return FieldType::String;
    }

    if non_null.iter().all(|v| v.is_object()) {
        return FieldType::Object;
    }

    if non_null.iter().all(|v| v.is_array()) {
        return FieldType::Array;
    }

    FieldType::Any
}

// ---------------------------------------------------------------------------
// String refinements
// ---------------------------------------------------------------------------

/// Attach pattern / enum / length constraints to a string field definition.
fn refine_string(def: &mut FieldDefinition, strings: &[&str], total_samples: usize) {
    if strings.is_empty() {
        return;
    }

    // Length bounds (always useful).
    let min_len = strings.iter().map(|s| s.len()).min().unwrap_or(0);
    let max_len = strings.iter().map(|s| s.len()).max().unwrap_or(0);
    // Only emit if informative (non-trivial bounds).
    if min_len > 0 {
        def.min_length = Some(min_len);
    }
    if max_len > 0 && max_len < usize::MAX {
        def.max_length = Some(max_len);
    }

    // Pattern detection — checked in priority order.  Only emitted when
    // *all* observed non-null values match; a single outlier disables it.
    if strings.iter().all(|s| looks_like_uuid(s)) {
        def.pattern =
            Some("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$".to_string());
        // UUID pattern supersedes enum — cardinality is always high.
        return;
    }

    if strings.iter().all(|s| looks_like_datetime(s)) {
        def.pattern = Some(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}".to_string());
        return;
    }

    if strings.iter().all(|s| looks_like_date(s)) {
        def.pattern = Some(r"^\d{4}-\d{2}-\d{2}$".to_string());
        return;
    }

    // Enum detection — low-cardinality string fields.
    // Threshold: at most 8 distinct values, and we need at least 2 non-null
    // observations to avoid spurious enums on single-sample inference.
    let distinct: HashSet<&&str> = strings.iter().collect();
    if distinct.len() <= 8 && strings.len() >= 2.min(total_samples) {
        let mut sorted: Vec<Value> = distinct
            .into_iter()
            .map(|s| Value::String(s.to_string()))
            .collect();
        sorted.sort_by_key(|v| v.to_string());
        def.allowed_values = Some(sorted);
        // Clear length bounds — they're redundant when an enum is set.
        def.min_length = None;
        def.max_length = None;
    }
}

// ---------------------------------------------------------------------------
// Pattern helpers (no regex dependency)
// ---------------------------------------------------------------------------

/// Returns `true` if the string looks like a lowercase UUID v4.
/// Accepts both lower and upper hex digits.
fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
        return false;
    }
    let hex_ranges = [0..8, 9..13, 14..18, 19..23, 24..36];
    hex_ranges
        .into_iter()
        .all(|r| b[r].iter().all(|c| c.is_ascii_hexdigit()))
}

/// Returns `true` if the string looks like an ISO 8601 date (`YYYY-MM-DD`).
fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[0..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

/// Returns `true` if the string looks like an ISO 8601 datetime
/// (`YYYY-MM-DDTHH:MM:SS…`).
fn looks_like_datetime(s: &str) -> bool {
    s.len() >= 19
        && s.as_bytes().get(10) == Some(&b'T')
        && looks_like_date(&s[..10])
        && s.as_bytes()[11..13].iter().all(|c| c.is_ascii_digit())
        && s.as_bytes().get(13) == Some(&b':')
        && s.as_bytes()[14..16].iter().all(|c| c.is_ascii_digit())
        && s.as_bytes().get(16) == Some(&b':')
        && s.as_bytes()[17..19].iter().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run_infer(name: &str, samples: Vec<Value>) -> Contract {
        let entities = infer_fields_from_objects(&samples);
        Contract {
            version: "1.0".to_string(),
            name: name.to_string(),
            description: None,
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            ontology: Ontology { entities },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
        }
    }

    #[test]
    fn infers_basic_types() {
        let samples = vec![
            json!({"id": "abc", "count": 3, "score": 1.5, "active": true}),
            json!({"id": "def", "count": 7, "score": 2.0, "active": false}),
        ];
        let contract = run_infer("test", samples);
        let fields: HashMap<&str, &FieldDefinition> = contract
            .ontology
            .entities
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        assert_eq!(fields["id"].field_type, FieldType::String);
        assert_eq!(fields["count"].field_type, FieldType::Integer);
        assert_eq!(fields["score"].field_type, FieldType::Float);
        assert_eq!(fields["active"].field_type, FieldType::Boolean);
        assert!(fields["id"].required);
    }

    #[test]
    fn optional_when_missing_from_some_samples() {
        let samples = vec![
            json!({"user_id": "u1", "amount": 10}),
            json!({"user_id": "u2"}),
        ];
        let contract = run_infer("test", samples);
        let fields: HashMap<&str, &FieldDefinition> = contract
            .ontology
            .entities
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        assert!(fields["user_id"].required);
        assert!(!fields["amount"].required);
    }

    #[test]
    fn optional_when_null_in_some_samples() {
        let samples = vec![json!({"x": 1, "y": null}), json!({"x": 2, "y": 3})];
        let contract = run_infer("test", samples);
        let fields: HashMap<&str, &FieldDefinition> = contract
            .ontology
            .entities
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        assert!(fields["x"].required);
        assert!(!fields["y"].required);
    }

    #[test]
    fn detects_enum() {
        let samples = vec![
            json!({"status": "active"}),
            json!({"status": "inactive"}),
            json!({"status": "active"}),
        ];
        let contract = run_infer("test", samples);
        let status = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status.allowed_values.is_some());
        let vals = status.allowed_values.as_ref().unwrap();
        assert_eq!(vals.len(), 2); // "active", "inactive"
    }

    #[test]
    fn detects_uuid_pattern() {
        let samples = vec![
            json!({"id": "550e8400-e29b-41d4-a716-446655440000"}),
            json!({"id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8"}),
        ];
        let contract = run_infer("test", samples);
        let id_field = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "id")
            .unwrap();
        assert!(id_field.pattern.is_some());
        assert!(id_field.pattern.as_ref().unwrap().contains("9a-f"));
    }

    #[test]
    fn detects_integer_min_max() {
        let samples = vec![json!({"n": 5}), json!({"n": 15}), json!({"n": 10})];
        let contract = run_infer("test", samples);
        let n = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "n")
            .unwrap();
        assert_eq!(n.min, Some(5.0));
        assert_eq!(n.max, Some(15.0));
    }

    #[test]
    fn detects_nested_object() {
        let samples = vec![
            json!({"meta": {"env": "prod", "region": "us-east"}}),
            json!({"meta": {"env": "staging", "region": "eu-west"}}),
        ];
        let contract = run_infer("test", samples);
        let meta = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "meta")
            .unwrap();
        assert_eq!(meta.field_type, FieldType::Object);
        assert!(meta.properties.is_some());
        let props = meta.properties.as_ref().unwrap();
        assert!(props.iter().any(|p| p.name == "env"));
        assert!(props.iter().any(|p| p.name == "region"));
    }

    #[test]
    fn looks_like_uuid_accepts_valid() {
        assert!(looks_like_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(looks_like_uuid("6BA7B810-9DAD-11D1-80B4-00C04FD430C8"));
    }

    #[test]
    fn looks_like_uuid_rejects_invalid() {
        assert!(!looks_like_uuid("not-a-uuid"));
        assert!(!looks_like_uuid("550e8400-e29b-41d4-a716-44665544000")); // one short
    }

    #[test]
    fn looks_like_date_correct() {
        assert!(looks_like_date("2026-04-27"));
        assert!(!looks_like_date("2026/04/27"));
        assert!(!looks_like_date("26-04-27"));
    }

    #[test]
    fn looks_like_datetime_correct() {
        assert!(looks_like_datetime("2026-04-27T14:30:00Z"));
        assert!(looks_like_datetime("2026-04-27T14:30:00.000Z"));
        assert!(!looks_like_datetime("2026-04-27 14:30:00"));
    }
}
