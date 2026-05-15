//! Core semantic validation engine — the patent-pending heart of ContractGate.
//!
//! The validator checks an incoming JSON event against a semantic `Contract` and
//! returns either `ValidationResult::Pass` or `ValidationResult::Fail` with a
//! detailed list of `Violation` structs.
//!
//! ### Two-stage shape
//!
//! Validation is split into a **compile-once** stage and a **validate-many**
//! stage so per-event work stays cheap:
//!
//!   1. [`CompiledContract::compile_with_salt`] runs at contract-load time.
//!      It parses every `pattern` regex, indexes the declared top-level field
//!      names (for compliance-mode O(1) lookup), and binds the per-contract
//!      `pii_salt` so transforms have keying material.  This is the only stage
//!      that can fail with a contract-error; once compiled, [`validate`]
//!      cannot panic.
//!   2. [`validate`] runs per inbound event.  It does no I/O, no allocation
//!      of regexes, and no parsing of contract YAML.  The compiled contract
//!      is shared by `Arc`, so callers can fan out parallel validations
//!      across rayon (see `ingest.rs`) without contention.
//!
//! ### Per-event pipeline order
//!
//! Within a single [`validate`] call, checks run in this fixed order so the
//! resulting `violations` vec is stable and operator-triagable:
//!
//!   1. **Ontology fields** — required/type/pattern/enum/range/length, walked
//!      recursively for nested object fields.
//!   2. **Metric definitions** — declared in `metrics:`, currently only the
//!      `min`/`max` envelope (formula evaluation lands later).
//!   3. **Compliance-mode undeclared fields** (RFC-004) — only runs when the
//!      resolved contract version opted in, and is intentionally last so the
//!      "fix your ontology errors first" violations come before the "stray
//!      field" violations in the response.
//!
//! ### What lives elsewhere
//!
//! - **PII transforms** (`hash`, `mask:format_preserving`, etc.) live in
//!   `transform.rs`.  Validation is read-only; transforms are applied to a
//!   *clone* of the event on the ingest side and are what gets persisted.
//!   Compile-time only: this module rejects contracts that put a transform
//!   on a non-string field (see [`validate_transform_types`]).
//! - **Version resolution + fallback** lives in `ingest.rs`.  The validator
//!   sees a single `CompiledContract` and has no notion of "is this stable
//!   or deprecated"; that decision is made before we get here.
//!
//! ### Design goals
//!
//!   - Zero heap allocations in the hot path wherever possible
//!   - All regex compiled once at contract load time (cached via `CompiledContract`)
//!   - Sub-15 ms p99 for typical event sizes on modest hardware
//!   - Clear, actionable violation messages for data engineers
//!   - `validate()` is total: any input shape produces a `ValidationResult`,
//!     never a panic or `Result::Err`.

use crate::contract::{
    Contract, FieldDefinition, FieldType, MetricDefinition, QualityRule, QualityRuleType,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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
    #[allow(dead_code)]
    UnknownField,
    /// RFC-004 compliance-mode violation: an inbound event contained a
    /// field name that is not declared in the contract's ontology.  Only
    /// raised when the resolved version has `compliance_mode = true`.
    UndeclaredField,
    /// RFC-030 egress leakage violation: an outbound payload contained a
    /// field not declared in the contract's ontology when
    /// `egress_leakage_mode = fail`.  The field is stripped from the
    /// response regardless of the RFC-029 disposition.
    LeakageViolation,
    /// Quality rule: field is null, missing when expected, or (for strings)
    /// an empty string.  Emitted by `rule_type: completeness` checks.
    CompletenessViolation,
    /// Quality rule: field value is older than `max_age_seconds` relative to
    /// the ingest wall-clock time.  Emitted by `rule_type: freshness` checks.
    FreshnessViolation,
    /// Quality rule: field value appears more than once in the same ingest
    /// batch.  Emitted by `rule_type: uniqueness` checks.
    UniquenessViolation,
}

// ---------------------------------------------------------------------------
// Pre-compiled contract (cached regexes + field index)
// ---------------------------------------------------------------------------

/// A `Contract` with expensive operations (regex compilation) done once.
/// Re-use across many `validate()` calls for maximum throughput.
#[derive(Debug)]
pub struct CompiledContract {
    pub contract: Contract,
    /// field_name → compiled regex (for ontology fields with `pattern`)
    pub patterns: HashMap<String, Regex>,
    /// RFC-004: per-contract 32-byte salt for the hash + format-preserving
    /// mask transforms.  Loaded from `contracts.pii_salt` on the identity
    /// row; `compile()` with no salt defaults to an empty `Vec<u8>` which
    /// is valid for any contract that does not declare a `hash` or
    /// `format_preserving` transform (the transform engine only reads this
    /// when it actually needs keying material).
    pub pii_salt: Vec<u8>,
    /// RFC-004: the set of top-level field names declared in the ontology,
    /// cached here so the compliance-mode undeclared-field check is an
    /// O(1) HashSet lookup per incoming field rather than an O(n) scan.
    /// Empty when `contract.compliance_mode == false` (the validator
    /// short-circuits the check, so the set is never consulted).
    pub declared_top_level_fields: HashSet<String>,
}

impl CompiledContract {
    /// Compile all regex patterns in the contract with no PII salt.
    /// Backwards-compatible entry point used by unit tests and any call
    /// site that pre-dates RFC-004.  For production paths that serve
    /// ingest traffic, use `compile_with_salt` so hash + format-preserving
    /// transforms are keyed correctly.
    pub fn compile(contract: Contract) -> anyhow::Result<Self> {
        Self::compile_with_salt(contract, Vec::new())
    }

    /// Compile with an explicit per-contract salt loaded from
    /// `contracts.pii_salt`.  This is the production entry point.
    pub fn compile_with_salt(contract: Contract, pii_salt: Vec<u8>) -> anyhow::Result<Self> {
        let mut patterns = HashMap::new();
        compile_field_patterns(&contract.ontology.entities, "", &mut patterns)?;
        validate_transform_types(&contract.ontology.entities, "")?;

        let declared_top_level_fields = if contract.compliance_mode {
            contract
                .ontology
                .entities
                .iter()
                .map(|f| f.name.clone())
                .collect()
        } else {
            HashSet::new()
        };

        Ok(CompiledContract {
            contract,
            patterns,
            pii_salt,
            declared_top_level_fields,
        })
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
            let re = Regex::new(pattern).map_err(|e| {
                anyhow::anyhow!("Invalid regex '{}' for field '{}': {}", pattern, path, e)
            })?;
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

/// RFC-004: reject contracts that declare a `transform` on any non-string
/// entity.  Applying `hash` / `mask` to a number or boolean is either a
/// type error at runtime OR silently coerces through JSON, both of which
/// produce surprises in prod — reject at contract-compile time instead.
/// Numeric PII (account numbers, etc.) should be typed as `string` in the
/// contract.
fn validate_transform_types(fields: &[FieldDefinition], prefix: &str) -> anyhow::Result<()> {
    for field in fields {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{}.{}", prefix, field.name)
        };

        if field.transform.is_some() && field.field_type != FieldType::String {
            return Err(anyhow::anyhow!(
                "Field '{}' declares a PII transform but has type '{:?}' — transforms are only supported on string fields. If this field holds PII, change its type to 'string'.",
                path,
                field.field_type
            ));
        }

        if field.field_type == FieldType::Object {
            if let Some(props) = &field.properties {
                validate_transform_types(props, &path)?;
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

    let _obj = match event.as_object() {
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

    // 3. Per-event quality rules (completeness, validity, freshness).
    //    Uniqueness is batch-level and handled separately in ingest.rs.
    for rule in &compiled.contract.quality {
        match rule.rule_type {
            QualityRuleType::Completeness => {
                check_completeness(rule, event, &mut violations);
            }
            QualityRuleType::Validity => {
                // Validity is already covered by the ontology field checks above.
                // No extra violation is emitted — this rule type is purely
                // declarative / scoring.
            }
            QualityRuleType::Freshness => {
                check_freshness(rule, event, &mut violations);
            }
            QualityRuleType::Uniqueness => {
                // Batch-level — skip in per-event path.
            }
        }
    }

    // 4. RFC-004: compliance-mode undeclared-field check.  Runs last so
    //    undeclared-field violations appear after the standard
    //    missing/mismatched-field violations in the response — which
    //    matches how operators typically triage ("fix my ontology
    //    errors before you worry about stray fields").  Only runs when
    //    the resolved version opted in.
    if compiled.contract.compliance_mode {
        if let Some(obj) = event.as_object() {
            for field_name in obj.keys() {
                if !compiled.declared_top_level_fields.contains(field_name) {
                    violations.push(Violation {
                        field: field_name.clone(),
                        message: format!(
                            "Field '{}' is not declared in the contract ontology. Compliance mode rejects undeclared fields.",
                            field_name
                        ),
                        kind: ViolationKind::UndeclaredField,
                    });
                }
            }
        }
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
///
/// Formula-style metrics (no `field`) are skipped at ingestion time —
/// they are informational / for downstream aggregation systems only.
/// Field-bound metrics (with `field` set) are checked against `min`/`max`.
fn validate_metric(metric: &MetricDefinition, event: &Value, violations: &mut Vec<Violation>) {
    // Formula-only metrics have no field — nothing to validate per-event.
    let field_path = match &metric.field {
        Some(f) => f,
        None => return,
    };

    // Only validate if bounds are actually set
    if metric.min.is_none() && metric.max.is_none() {
        return;
    }

    let value = resolve_path(event, field_path);

    let n = match value.and_then(numeric_value) {
        Some(n) => n,
        None => {
            violations.push(Violation {
                field: field_path.clone(),
                message: format!(
                    "Metric '{}' field '{}' is missing or not numeric",
                    metric.name, field_path
                ),
                kind: ViolationKind::MissingRequiredField,
            });
            return;
        }
    };

    if let Some(min) = metric.min {
        if n < min {
            violations.push(Violation {
                field: field_path.clone(),
                message: format!(
                    "Metric '{}' value {} is below minimum {} (field: '{}')",
                    metric.name, n, min, field_path
                ),
                kind: ViolationKind::MetricRangeViolation,
            });
        }
    }
    if let Some(max) = metric.max {
        if n > max {
            violations.push(Violation {
                field: field_path.clone(),
                message: format!(
                    "Metric '{}' value {} exceeds maximum {} (field: '{}')",
                    metric.name, n, max, field_path
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
// Quality rule helpers (per-event)
// ---------------------------------------------------------------------------

/// Walk a dot-notation path and return the value at that path, or None.
fn get_nested_value<'a>(event: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = event;
    for segment in path.split('.') {
        cur = cur.as_object()?.get(segment)?;
    }
    Some(cur)
}

/// `rule_type: completeness` — field must be present, non-null, non-empty string.
fn check_completeness(rule: &QualityRule, event: &Value, violations: &mut Vec<Violation>) {
    match get_nested_value(event, &rule.field) {
        None | Some(Value::Null) => {
            violations.push(Violation {
                field: rule.field.clone(),
                message: format!(
                    "Quality completeness: field '{}' is absent or null",
                    rule.field
                ),
                kind: ViolationKind::CompletenessViolation,
            });
        }
        Some(Value::String(s)) if s.is_empty() => {
            violations.push(Violation {
                field: rule.field.clone(),
                message: format!(
                    "Quality completeness: field '{}' is an empty string",
                    rule.field
                ),
                kind: ViolationKind::CompletenessViolation,
            });
        }
        _ => {} // present and non-empty — passes
    }
}

/// `rule_type: freshness` — field must be a Unix epoch within `max_age_seconds`.
///
/// Detection heuristic: if the value is > 1_700_000_000_000 it is treated as
/// milliseconds and divided by 1000 first.  This covers both second-precision
/// and millisecond-precision timestamps without requiring a schema annotation.
fn check_freshness(rule: &QualityRule, event: &Value, violations: &mut Vec<Violation>) {
    let Some(max_age) = rule.max_age_seconds else {
        // No max_age_seconds configured — nothing to check.
        return;
    };
    let Some(val) = get_nested_value(event, &rule.field) else {
        // Field absent — completeness rule handles this; skip here.
        return;
    };

    let ts_secs: Option<i64> = match val {
        Value::Number(n) => n.as_i64().map(|v| {
            // Heuristic: ms epoch if value is larger than ~year 2023 in seconds
            if v > 1_700_000_000_000 {
                v / 1000
            } else {
                v
            }
        }),
        _ => None,
    };

    let Some(ts) = ts_secs else {
        violations.push(Violation {
            field: rule.field.clone(),
            message: format!(
                "Quality freshness: field '{}' is not a numeric Unix timestamp",
                rule.field
            ),
            kind: ViolationKind::FreshnessViolation,
        });
        return;
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let age_secs = now_secs - ts;
    if age_secs < 0 || age_secs > max_age as i64 {
        violations.push(Violation {
            field: rule.field.clone(),
            message: format!(
                "Quality freshness: field '{}' timestamp is {}s old (max {}s)",
                rule.field, age_secs, max_age
            ),
            kind: ViolationKind::FreshnessViolation,
        });
    }
}

// ---------------------------------------------------------------------------
// Quality rule helpers (batch-level)
// ---------------------------------------------------------------------------

/// `rule_type: uniqueness` — detect duplicate values for a field across
/// a batch of events.  Returns a `Vec<(event_index, Violation)>` so the
/// caller can annotate individual event results.
///
/// Only the *second and subsequent* occurrences of a value are flagged;
/// the first occurrence is always clean.
pub fn check_uniqueness_batch(rules: &[QualityRule], events: &[Value]) -> Vec<(usize, Violation)> {
    let mut out = Vec::new();

    let unique_rules: Vec<&QualityRule> = rules
        .iter()
        .filter(|r| r.rule_type == QualityRuleType::Uniqueness)
        .collect();

    if unique_rules.is_empty() {
        return out;
    }

    for rule in unique_rules {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (idx, event) in events.iter().enumerate() {
            let Some(val) = get_nested_value(event, &rule.field) else {
                continue; // absent — completeness rule covers this
            };
            if val.is_null() {
                continue; // null values are not deduplicated
            }
            // Use JSON serialisation as the canonical key so arrays / objects
            // are compared by value, not identity.
            let key = val.to_string();
            if !seen.insert(key.clone()) {
                out.push((
                    idx,
                    Violation {
                        field: rule.field.clone(),
                        message: format!(
                            "Quality uniqueness: duplicate value {:?} for field '{}' in batch",
                            key, rule.field
                        ),
                        kind: ViolationKind::UniquenessViolation,
                    },
                ));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{EgressLeakageMode, FieldDefinition, FieldType, Ontology};
    use serde_json::json;

    fn make_simple_contract() -> Contract {
        Contract {
            version: "1.0".into(),
            name: "test".into(),
            description: None,
            compliance_mode: false,
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
                        transform: None,
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
                        transform: None,
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
                        transform: None,
                    },
                ],
            },
            egress_leakage_mode: EgressLeakageMode::Off,
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
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
        assert!(
            result.passed,
            "Expected pass but got violations: {:?}",
            result.violations
        );
    }

    #[test]
    fn missing_required_field_fails() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let event = json!({ "user_id": "alice_01", "event_type": "click" }); // missing timestamp
        let result = validate(&compiled, &event);
        assert!(!result.passed);
        assert!(result
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::MissingRequiredField));
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
        assert!(result
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::EnumViolation));
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
        assert!(result
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::PatternMismatch));
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
        assert!(result
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::RangeViolation));
    }

    #[test]
    fn non_object_event_fails() {
        let contract = make_simple_contract();
        let compiled = CompiledContract::compile(contract).unwrap();
        let result = validate(&compiled, &json!(["not", "an", "object"]));
        assert!(!result.passed);
    }
}
