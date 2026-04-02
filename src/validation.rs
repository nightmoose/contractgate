//! Core semantic validation engine — the patent-pending heart of ContractGate.
//!
//! The validator checks an incoming JSON event against a semantic `Contract` and
//! returns either `ValidationResult::Pass` or `ValidationResult::Fail` with a
//! detailed list of `Violation` structs.
//!
//! Design goals:
//!   - Zero heap allocations in the hot path wherever possible
//!   - All regex compiled once at contract load time (cached via `CompiledContract`)
//!   - Sub-15 ms p99 for typical event sizes on modest hardware
//!   - Clear, actionable violation messages for data engineers

use crate::contract::{Contract, FieldDefinition, FieldType, MetricDefinition};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// The outcome of validating a single event against a contract.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationResult {
    pub passed: bool,
    /// Empty when `passed == true`
    pub violations: Vec<Violation>,
    /// Wall-clock duration of the validation in microseconds
    pub validation_us: u64,
}

/// A single rule violation found during validation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Violation {
    /// Dot-separated path to the offending field (e.g. "user.address.zip")
    pub field: String,
    /// Human-readable explanation
    pub message: String,
    /// Machine-readable violation kind (for programmatic filtering)
    pub kind: ViolationKind,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    MissingRequiredField,
    TypeMismatch,
    PatternMismatch,
    EnumViolation,
    RangeViolation,
    LengthViolation,
    MetricRangeViolation,
    UnknownField,
}

// ---------------------------------------------------------------------------
// Pre-compiled contract (cached regexes + field index)
// ---------------------------------------------------------------------------

/// A `Contract` with expensive operations (regex compilation) done once.
/// Re-use across many `validate()` calls for maximum throughput.
pub struct CompiledContract {
    pub contract: Contract,
    /// field_name → compiled regex (for ontology fields with `pattern`)
    pub patterns: HashMap<String, Regex>,
}

impl CompiledContract {
    /// Compile all regex patterns in the contract.
    /// Returns an error if any pattern is invalid.
    pub fn compile(contract: Contract) -> anyhow::Result<Self> {
        let mut patterns = HashMap::new();
        compile_field_patterns(&contract.ontology.entities, "", &mut patterns)?;
        Ok(CompiledContract { contract, patterns })
    }
}

/// Recursively walk field definitions and compile regex patterns.
fn compile_field_patterns(
    fields: &[FieldDefinition],
    prefix: &str,
    out: &mut HashMap<String, Regex>,
) -> anyhow::Result<()> {
    for field in fields {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{}.{}", prefix, field.name)
        };

        if let Some(pattern) = &field.pattern {
            let re = Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("Invalid regex '{}' for field '{}': {}", pattern, path, e))?;
            out.insert(path.clone(), re);
        }

        // Recurse into nested object properties
        if field.field_type == FieldType::Object {
            if let Some(props) = &field.properties {
                compile_field_patterns(props, &path, out)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main validation entry point
// ---------------------------------------------------------------------------

/// Validate `event` against `compiled_contract`.
///
/// Returns a `ValidationResult` — always succeeds (never panics).
pub fn validate(compiled: &CompiledContract, event: &Value) -> ValidationResult {
    let t0 = std::time::Instant::now();
    let mut violations = Vec::new();

    let obj = match event.as_object() {
        Some(o) => o,
        None => {
            return ValidationResult {
                passed: false,
                violations: vec![Violation {
                    field: "<root>".into(),
                    message: "Event must be a JSON object".into(),
                    kind: ViolationKind::TypeMismatch,
                }],
                validation_us: 0,
            };
        }
    };

    // 1. Validate ontology fields
    validate_fields(
        &compiled.contract.ontology.entities,
        event,
        "",
        &compiled.patterns,
        &mut violations,
    );

    // 2. Validate metric definitions
    for metric in &compiled.contract.metrics {
        validate_metric(metric, event, &mut violations);
    }

    let validation_us = t0.elapsed().as_micros() as u64;
    let passed = violations.is_empty();

    ValidationResult {
        passed,
        violations,
        validation_us,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validate_fields(
    fields: &[FieldDefinition],
    data: &Value,
    prefix: &str,
    patterns: &HashMap<String, Regex>,
    violations: &mut Vec<Violation>,
) {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return, // parent type mismatch already reported
    };

    for field in fields {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{}.{}", prefix, field.name)
        };

        match obj.get(&field.name) {
            None => {
                if field.required {
                    violations.push(Violation {
                        field: path,
                        message: format!("Required field '{}' is missing", field.name),
                        kind: ViolationKind::MissingRequiredField,
                    });
                }
                // Optional and absent — nothing to validate
            }
            Some(value) => {
                validate_value(field, value, &path, patterns, violations);
            }
        }
    }
}

fn validate_value(
    field: &FieldDefinition,
    value: &Value,
    path: &str,
    patterns: &HashMap<String, Regex>,
    violations: &mut Vec<Violation>,
) {
    // --- Type check ---
    let type_ok = match &field.field_type {
        FieldType::String => value.is_string(),
        FieldType::Integer => value.is_i64() || value.is_u64(),
        FieldType::Float => value.is_f64() || value.is_i64() || value.is_u64(),
        FieldType::Boolean => value.is_boolean(),
        FieldType::Object => value.is_object(),
        FieldType::Array => value.is_array(),
        FieldType::Any => true,
    };

    if !type_ok {
        violations.push(Violation {
            field: path.to_string(),
            message: format!(
                "Field '{}' expected type {:?}, got {}",
                path,
                field.field_type,
                json_type_name(value)
            ),
            kind: ViolationKind::TypeMismatch,
        });
        return; // Further checks on the wrong type make no sense
    }

    // --- String-specific checks ---
    if let Some(s) = value.as_str() {
        // Length checks
        if let Some(min_len) = field.min_length {
            if s.len() < min_len {
                violations.push(Violation {
                    field: path.to_string(),
                    message: format!(
                        "Field '{}' length {} is below minimum {}",
                        path,
                        s.len(),
                        min_len
                    ),
                    kind: ViolationKind::LengthViolation,
                });
            }
        }
        if let Some(max_len) = field.max_length {
            if s.len() > max_len {
                violations.push(Violation {
                    field: path.to_string(),
                    message: format!(
                        "Field '{}' length {} exceeds maximum {}",
                        path,
                        s.len(),
                        max_len
                    ),
                    kind: ViolationKind::LengthViolation,
                });
            }
        }

        // Pattern check (uses pre-compiled regex)
        if let Some(re) = patterns.get(path) {
            if !re.is_match(s) {
                violations.push(Violation {
                    field: path.to_string(),
                    message: format!(
                        "Field '{}' value {:?} does not match required pattern",
                        path, s
                    ),
                    kind: ViolationKind::PatternMismatch,
                });
            }
        }
    }

    // --- Numeric range checks ---
    if let Some(n) = numeric_value(value) {
        if let Some(min) = field.min {
            if n < min {
                violations.push(Violation {
                    field: path.to_string(),
                    message: format!("Field '{}' value {} is below minimum {}", path, n, min),
                    kind: ViolationKind::RangeViolation,
                });
            }
        }
        if let Some(max) = field.max {
            if n > max {
                violations.push(Violation {
                    field: path.to_string(),
                    message: format!("Field '{}' value {} exceeds maximum {}", path, n, max),
                    kind: ViolationKind::RangeViolation,
                });
            }
        }
    }

    // --- Enum check ---
    if let Some(allowed) = &field.allowed_values {
        if !allowed.contains(value) {
            let allowed_str: Vec<String> = allowed.iter().map(|v| v.to_string()).collect();
            violations.push(Violation {
                field: path.to_string(),
                message: format!(
                    "Field '{}' value {} not in allowed set: [{}]",
                    path,
                    value,
                    allowed_str.join(", ")
                ),
                kind: ViolationKind::EnumViolation,
            });
        }
    }

    // --- Recurse into nested objects ---
    if field.field_type == FieldType::Object {
        if let Some(props) = &field.properties {
            validate_fields(props, value, path, patterns, violations);
        }
    }

    // --- Recurse into array items ---
    if field.field_type == FieldType::Array {
        if let (Some(arr), Some(item_def)) = (value.as_array(), &field.items) {
            for (idx, item) in arr.iter().enumerate() {
                let item_path = format!("{}[{}]", path, idx);
                validate_value(item_def, item, &item_path, patterns, violations);
            }
        }
    }
}

/// Validate a metric definition against the event.
fn validate_metric(
    metric: &MetricDefinition,
    event: &Value,
    violations: &mut Vec<Violation>,
) {
    let value = resolve_path(event, &metric.field);

    let n = match value.and_then(numeric_value) {
        Some(n) => n,
        None => {
            // Missing metric field is a violation if it has bounds
            if metric.min.is_some() || metric.max.is_some() {
                violations.push(Violation {
                    field: metric.field.clone(),
                    message: format!(
                        "Metric '{}' field '{}' is missing or not numeric",
                        metric.name, metric.field
                    ),
                    kind: ViolationKind::MissingRequiredField,
                });
            }
            return;
        }
    };

    if let Some(min) = metric.min {
        if n < min {
            violations.push(Violation {
                field: metric.field.clone(),
                message: format!(
                    "Metric '{}' value {} is below minimum {} (field: '{}')",
                    metric.name, n, min, metric.field
                ),
                kind: ViolationKind::MetricRangeViolation,
            });
        }
    }
    if let Some(max) = metric.max {
        if n > max {
            violations.push(Violation {
                field: metric.field.clone(),
                message: format!(
                    "Metric '{}' value {} exceeds maximum {} (field: '{}')",
                    metric.name, n, max, metric.field
                ),
                kind: ViolationKind::MetricRangeViolation,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Resolve a dot-separated path in a JSON value (e.g. "user.address.zip").
fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    Some(current)
}

/// Extract a numeric f64 from any JSON number (int, uint, or float).
fn numeric_value(value: &Value) -> Option<f64> {
    if let Some(n) = value.as_f64() {
        return Some(n);
    }
    if let Some(n) = value.as_i64() {
        return Some(n as f64);
    }
    if let Some(n) = value.as_u64() {
        return Some(n as f64);
    }
    None
}

/// Human-readable JSON type name for error messages.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{FieldDefinition, FieldType, Ontology};
    use serde_json::json;

    fn make_simple_contract() -> Contract {
        Contract {
            version: "1.0".into(),
            name: "test".into(),
            description: None,
            ontology: Ontology {
                entities: vec![
                    FieldDefinition {
                        name: "user_id".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: Some(r"^[a-z0-9_]{3,50}$".into()),
                        allowed_values: None,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                    FieldDefinition {
                        name: "event_type".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: None,
                        allowed_values: Some(vec![
                            json!("click"),
                            json!("view"),
                            json!("purchase"),
                        ]),
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                    FieldDefinition {
                        name: "timestamp".into(),
                        field_type: FieldType::Integer,
                        required: true,
                        pattern: None,
                        allowed_values: None,
                        min: Some(0.0),
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                    },
                ],
            },
            glossary: vec![],
            metrics: vec![],
        }
    }

    #[test]
    fn valid_event_passes() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": 1712000000
        });
        let result = validate(&compiled, &event);
        assert!(result.passed, "Expected pass but got violations: {:?}", result.violations);
    }

    #[test]
    fn missing_required_field_fails() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({ "user_id": "alice_01", "event_type": "click" }); // missing timestamp
        let result = validate(&compiled, &event);
        assert!(!result.passed);
        assert!(result.violations.iter().any(|v| v.kind == ViolationKind::MissingRequiredField));
    }

    #[test]
    fn enum_violation_detected() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "delete", // not in allowed set
            "timestamp": 1712000000
        });
        let result = validate(&compiled, &event);
        assert!(!result.passed);
        assert!(result.violations.iter().any(|v| v.kind == ViolationKind::EnumViolation));
    }

    #[test]
    fn pattern_violation_detected() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({
            "user_id": "Alice 01!!",  // uppercase + spaces not allowed
            "event_type": "click",
            "timestamp": 1712000000
        });
        let result = validate(&compiled, &event);
        assert!(!result.passed);
        assert!(result.violations.iter().any(|v| v.kind == ViolationKind::PatternMismatch));
    }

    #[test]
    fn range_violation_detected() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": -1  // below min=0
        });
        let result = validate(&compiled, &event);
        assert!(!result.passed);
        assert!(result.violations.iter().any(|v| v.kind == ViolationKind::RangeViolation));
    }

    #[test]
    fn non_object_event_fails() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let result = validate(&compiled, &json!(["not", "an", "object"]));
        assert!(!result.passed);
    }
}
