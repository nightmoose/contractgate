//! ODCS import / export for DataContractGate.
//!
//! Export writes ODCS v3.1.0 (`apiVersion: v3.1.0`) so the document still
//! validates against the vendored v3.1.0 JSON Schema. Import accepts
//! `apiVersion: v3.1.0` and `v3.2.0`, plus any other `v3.*` string. A missing
//! `apiVersion` still imports. A property-level `enum` — a scalar list or
//! `{ value: ... }` objects — is applied before the v3.1 `invalidValues` /
//! `validValues` rule. `x-cg-enum` still wins over both.
//!
//! # Round-trip guarantee
//!
//! Export always writes `x-contractgate-*` extensions (D-003) as entries of
//! the top-level `customProperties[]` array, so the document validates
//! against the official ODCS v3.1.0 JSON Schema.  Import reads those
//! extensions (or the legacy bare root keys) as authoritative (Mode A —
//! lossless).  For foreign ODCS
//! documents without extensions, import reconstructs a best-effort Contract
//! from the ODCS schema section (Mode B — stripped, `requires_review = true`).
//!
//! # PII safety invariant
//!
//! `pii_salt` is NEVER written to ODCS output under any circumstances.
//! The `ContractIdentity` is accepted solely to provide the contract `id` and
//! `name`; callers must NOT pass the salt to any outbound serializer.

use crate::contract::{
    Contract, ContractIdentity, ContractVersion, EgressLeakageMode, FieldDefinition, FieldType,
    ImportSource, Ontology, QualityRule, QualityRuleType, UniqueScope, VersionState,
};
use serde_yaml::{Mapping, Value};

// ---------------------------------------------------------------------------
// ODCS status ↔ VersionState mapping
// ---------------------------------------------------------------------------

fn version_state_to_odcs_status(state: VersionState) -> &'static str {
    match state {
        VersionState::Draft => "proposed",
        VersionState::Stable => "active",
        VersionState::Deprecated => "retired",
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// ODCS spec release written on export. Stays v3.1.0 so exports validate
/// against the vendored v3.1.0 JSON Schema (that schema has no property `enum`).
pub const ODCS_API_VERSION: &str = "v3.1.0";
/// `x-contractgate-*` extension *format* version (not the contract version).
const EXT_FORMAT_VERSION: &str = "1.0";
/// Per-quality-entry custom property carrying the verbatim CG `QualityRule`.
const X_CG_QUALITY: &str = "x-cg-quality";
/// Per-quality-entry marker: the entry restates an ontology constraint
/// (e.g. `enum`) for ODCS-native engines and is not a CG quality rule.
const X_CG_DERIVED: &str = "x-cg-derived";
/// `engine` name for `type: custom` quality entries only DCG can execute.
const CG_ENGINE: &str = "contractgate";
/// Default ODCS date format for CG `type: date` (YYYY-MM-DD).
const CG_DATE_FORMAT: &str = "yyyy-MM-dd";

// ---------------------------------------------------------------------------
// FieldType ↔ ODCS logicalType mapping
// ---------------------------------------------------------------------------

/// ODCS logicalType for a CG field type.  `Any` has no ODCS equivalent, so
/// `logicalType` is omitted and `x-cg-type: any` carries it instead.
fn field_type_to_logical(ft: &FieldType) -> Option<&'static str> {
    match ft {
        FieldType::String => Some("string"),
        FieldType::Integer => Some("integer"),
        FieldType::Float => Some("number"),
        FieldType::Boolean => Some("boolean"),
        FieldType::Object => Some("object"),
        FieldType::Array => Some("array"),
        FieldType::Any => None,
        // RFC-044
        FieldType::Date => Some("date"),
        FieldType::Timestamp => Some("timestamp"),
    }
}

fn logical_to_field_type(s: &str) -> FieldType {
    match s {
        "integer" | "long" | "int" => FieldType::Integer,
        "double" | "float" | "decimal" | "number" => FieldType::Float,
        "boolean" | "bool" => FieldType::Boolean,
        "object" | "record" | "struct" => FieldType::Object,
        "array" | "list" => FieldType::Array,
        // RFC-044: date32 is the Arrow/Iceberg alias
        "date" | "date32" => FieldType::Date,
        "timestamp" | "datetime" | "timestamptz" => FieldType::Timestamp,
        "any" => FieldType::Any,
        _ => FieldType::String,
    }
}

// ---------------------------------------------------------------------------
// Property flattening (D-001)
// ---------------------------------------------------------------------------

/// Flatten a CG field tree to ODCS `schema[].properties[]`.
/// Nested objects are expanded to dot-notation names, e.g.
/// `user.address.street`.  Constraints are written twice: as ODCS-native
/// fields (`logicalTypeOptions`, `unique`, `quality[]`) for strict ODCS
/// consumers, and as `x-cg-*` `customProperties` for lossless re-import.
fn flatten_to_odcs_properties(
    prefix: &str,
    fields: &[FieldDefinition],
    quality_rules: &[QualityRule],
) -> Vec<Value> {
    let mut out = Vec::new();
    for f in fields {
        let name = if prefix.is_empty() {
            f.name.clone()
        } else {
            format!("{}.{}", prefix, f.name)
        };

        if let (FieldType::Object, Some(nested)) = (
            &f.field_type,
            f.properties.as_ref().filter(|n| !n.is_empty()),
        ) {
            // Recurse (an object with no fields is kept as a leaf below) — the
            // parent "object" node is omitted from ODCS; only leaves appear as
            // properties (D-001).
            out.extend(flatten_to_odcs_properties(&name, nested, quality_rules));
        } else {
            let prop_quality: Vec<&QualityRule> =
                quality_rules.iter().filter(|r| r.field == name).collect();
            out.push(build_odcs_property(&name, f, &prop_quality));
        }
    }
    out
}

/// ODCS-native `logicalTypeOptions` for a field, limited to the keys the
/// ODCS v3.1.0 JSON Schema allows for its logicalType.
fn native_type_options(f: &FieldDefinition) -> Mapping {
    let mut o = Mapping::new();
    match f.field_type {
        FieldType::String => {
            if let Some(p) = &f.pattern {
                o.insert(v_str("pattern"), v_str(p));
            }
            if let Some(n) = f.min_length {
                o.insert(v_str("minLength"), Value::Number((n as u64).into()));
            }
            if let Some(n) = f.max_length {
                o.insert(v_str("maxLength"), Value::Number((n as u64).into()));
            }
            if let Some(fmt) = &f.format {
                o.insert(v_str("format"), v_str(fmt));
            }
        }
        FieldType::Integer | FieldType::Float => {
            if let Some(v) = f.min {
                o.insert(v_str("minimum"), num(v));
            }
            if let Some(v) = f.max {
                o.insert(v_str("maximum"), num(v));
            }
        }
        FieldType::Date => {
            let fmt = f.format.as_deref().unwrap_or(CG_DATE_FORMAT);
            o.insert(v_str("format"), v_str(fmt));
        }
        FieldType::Timestamp => {
            if let Some(fmt) = &f.format {
                o.insert(v_str("format"), v_str(fmt));
            }
        }
        _ => {}
    }
    o
}

/// Build one ODCS property entry for a leaf field.
/// `prop_quality` contains only the quality rules that target this property.
fn build_odcs_property(name: &str, f: &FieldDefinition, prop_quality: &[&QualityRule]) -> Value {
    let mut prop = Mapping::new();
    prop.insert(v_str("name"), v_str(name));
    if let Some(lt) = field_type_to_logical(&f.field_type) {
        prop.insert(v_str("logicalType"), v_str(lt));
    }
    let opts = native_type_options(f);
    if !opts.is_empty() {
        prop.insert(v_str("logicalTypeOptions"), Value::Mapping(opts));
    }
    prop.insert(v_str("required"), Value::Bool(f.required));
    if prop_quality
        .iter()
        .any(|r| r.rule_type == QualityRuleType::Uniqueness)
    {
        prop.insert(v_str("unique"), Value::Bool(true));
    }

    // classification: "pii" when a PII transform is declared
    if f.transform.is_some() {
        prop.insert(v_str("classification"), v_str("pii"));
    }

    // quality[]: schema-valid ODCS entries (library metrics or type: custom).
    let mut quality_entries: Vec<Value> = prop_quality
        .iter()
        .map(|r| quality_rule_to_odcs(r, f))
        .collect();
    // ODCS v3.1.0 has no property-level `enum`; restate it as a zero-tolerance
    // invalidValues rule unless a validity rule already carries validValues.
    let has_validity = prop_quality
        .iter()
        .any(|r| r.rule_type == QualityRuleType::Validity);
    if let (Some(vals), false) = (&f.allowed_values, has_validity) {
        let mut q = Mapping::new();
        q.insert(v_str("type"), v_str("library"));
        q.insert(v_str("metric"), v_str("invalidValues"));
        q.insert(v_str("dimension"), v_str("conformity"));
        q.insert(v_str("mustBe"), Value::Number(0.into()));
        let mut args = Mapping::new();
        args.insert(v_str("validValues"), json_seq(vals));
        q.insert(v_str("arguments"), Value::Mapping(args));
        q.insert(
            v_str("customProperties"),
            Value::Sequence(vec![custom_prop(X_CG_DERIVED, v_str("enum"))]),
        );
        quality_entries.push(Value::Mapping(q));
    }
    prop.insert(v_str("quality"), Value::Sequence(quality_entries));

    // customProperties: CG constraints (authoritative on re-import)
    let mut custom: Vec<Value> = Vec::new();
    if f.field_type == FieldType::Any {
        custom.push(custom_prop("x-cg-type", v_str("any")));
    }
    if let Some(pat) = &f.pattern {
        custom.push(custom_prop("x-cg-pattern", v_str(pat)));
    }
    if let Some(vals) = &f.allowed_values {
        custom.push(custom_prop("x-cg-enum", json_seq(vals)));
    }
    if let Some(v) = f.min {
        custom.push(custom_prop("x-cg-min", num(v)));
    }
    if let Some(v) = f.max {
        custom.push(custom_prop("x-cg-max", num(v)));
    }
    if let Some(v) = f.min_length {
        custom.push(custom_prop(
            "x-cg-min-length",
            Value::Number((v as u64).into()),
        ));
    }
    if let Some(v) = f.max_length {
        custom.push(custom_prop(
            "x-cg-max-length",
            Value::Number((v as u64).into()),
        ));
    }
    if let Some(fmt) = &f.format {
        custom.push(custom_prop("x-cg-format", v_str(fmt)));
    }
    if let Some(tr) = &f.transform {
        custom.push(custom_prop("x-cg-transform-kind", v_str(tr.kind.as_str())));
        if let Some(style) = &tr.style {
            custom.push(custom_prop("x-cg-transform-style", v_str(style.as_str())));
        }
    }
    if !custom.is_empty() {
        prop.insert(v_str("customProperties"), Value::Sequence(custom));
    }

    // An object leaf here has no nested fields (those are flattened by the
    // caller); arrays carry their element schema as ODCS `items`, with an
    // object element's fields nested natively under `items.properties`.
    if let (FieldType::Array, Some(item)) = (&f.field_type, &f.items) {
        prop.insert(v_str("items"), build_odcs_item(item));
    }

    Value::Mapping(prop)
}

/// Build an ODCS element schema (`items`) for an array field, recursively.
/// Element fields are nested natively (`properties[]` with leaf names), since
/// dot-notation paths can't address inside an array.
fn build_odcs_item(item: &FieldDefinition) -> Value {
    let name = if item.name.is_empty() {
        "item"
    } else {
        item.name.as_str()
    };
    let mut v = build_odcs_property(name, item, &[]);
    if let (FieldType::Object, Some(nested), Value::Mapping(m)) =
        (&item.field_type, &item.properties, &mut v)
    {
        let children: Vec<Value> = nested.iter().map(build_odcs_nested_property).collect();
        m.insert(v_str("properties"), Value::Sequence(children));
    }
    v
}

/// One natively nested ODCS property (inside an array element), recursive.
fn build_odcs_nested_property(f: &FieldDefinition) -> Value {
    let mut v = build_odcs_property(&f.name, f, &[]);
    if let (FieldType::Object, Some(nested), Value::Mapping(m)) =
        (&f.field_type, &f.properties, &mut v)
    {
        let children: Vec<Value> = nested.iter().map(build_odcs_nested_property).collect();
        m.insert(v_str("properties"), Value::Sequence(children));
    }
    v
}

// ---------------------------------------------------------------------------
// Quality rule ↔ ODCS quality[] mapping
// ---------------------------------------------------------------------------

/// Insert the ODCS threshold operator for a CG pass-fraction `threshold`
/// (share of events that must pass).  Strict (None / 1.0) → `mustBe: 0`;
/// otherwise the tolerated failure share as `mustBeLessOrEqualTo` percent.
fn insert_threshold(m: &mut Mapping, threshold: Option<f64>) {
    match threshold {
        Some(t) if t < 1.0 => {
            let pct = ((1.0 - t) * 100.0 * 1e6).round() / 1e6;
            m.insert(v_str("mustBeLessOrEqualTo"), num(pct));
            m.insert(v_str("unit"), v_str("percent"));
        }
        _ => {
            m.insert(v_str("mustBe"), Value::Number(0.into()));
        }
    }
}

/// Serialize a CG `QualityRule` to a schema-valid ODCS `quality[]` entry.
///
/// | CG type | ODCS entry |
/// |---|---|
/// | completeness | `library` / `missingValues` |
/// | uniqueness | `library` / `duplicateValues` |
/// | validity | `library` / `invalidValues` (+ `validValues` / `pattern`) |
/// | freshness | `custom`, `engine: contractgate` |
///
/// The verbatim CG rule rides along as `customProperties[x-cg-quality]` and
/// wins on re-import.
fn quality_rule_to_odcs(rule: &QualityRule, f: &FieldDefinition) -> Value {
    let mut m = Mapping::new();
    match rule.rule_type {
        QualityRuleType::Completeness => {
            m.insert(v_str("type"), v_str("library"));
            m.insert(v_str("metric"), v_str("missingValues"));
            m.insert(v_str("dimension"), v_str("completeness"));
            insert_threshold(&mut m, rule.threshold);
        }
        QualityRuleType::Uniqueness => {
            m.insert(v_str("type"), v_str("library"));
            m.insert(v_str("metric"), v_str("duplicateValues"));
            m.insert(v_str("dimension"), v_str("uniqueness"));
            insert_threshold(&mut m, rule.threshold);
        }
        QualityRuleType::Validity => {
            m.insert(v_str("type"), v_str("library"));
            m.insert(v_str("metric"), v_str("invalidValues"));
            m.insert(v_str("dimension"), v_str("conformity"));
            insert_threshold(&mut m, rule.threshold);
            let mut args = Mapping::new();
            if let Some(vals) = &f.allowed_values {
                args.insert(v_str("validValues"), json_seq(vals));
            }
            if let Some(p) = &f.pattern {
                args.insert(v_str("pattern"), v_str(p));
            }
            if !args.is_empty() {
                m.insert(v_str("arguments"), Value::Mapping(args));
            }
        }
        QualityRuleType::Freshness => {
            m.insert(v_str("type"), v_str("custom"));
            m.insert(v_str("engine"), v_str(CG_ENGINE));
            m.insert(v_str("dimension"), v_str("timeliness"));
            // String form (the schema allows string or object; OpenMetadata
            // 2.0.x only deserializes strings).
            let imp = match rule.max_age_seconds {
                Some(n) => format!("freshness maxAgeSeconds={n}"),
                None => "freshness".to_string(),
            };
            m.insert(v_str("implementation"), v_str(imp));
        }
    }
    if let Some(desc) = &rule.description {
        m.insert(v_str("description"), v_str(desc));
    }
    if let Ok(raw) = serde_yaml::to_value(rule) {
        m.insert(
            v_str("customProperties"),
            Value::Sequence(vec![custom_prop(X_CG_QUALITY, raw)]),
        );
    }
    Value::Mapping(m)
}

/// CG pass-fraction threshold from an ODCS operator.  Only percent-based
/// upper bounds map; zero tolerance and count-based bounds become strict.
fn threshold_from_odcs(q: &Mapping) -> Option<f64> {
    let percent = q.get("unit").and_then(|v| v.as_str()) == Some("percent");
    let bound = q
        .get("mustBeLessOrEqualTo")
        .or_else(|| q.get("mustBeLessThan"))
        .and_then(|v| v.as_f64())?;
    (percent && bound > 0.0 && bound < 100.0).then(|| 1.0 - bound / 100.0)
}

/// Parse a `type: custom, engine: contractgate` freshness implementation:
/// `"freshness maxAgeSeconds=N"` (export form) or `{type: freshness,
/// maxAgeSeconds: N}`.  Returns `None` for anything that isn't freshness,
/// `Some(None)` for freshness without a max age.
fn parse_cg_freshness(imp: &Value) -> Option<Option<u64>> {
    match imp {
        Value::String(s) => {
            let mut parts = s.split_whitespace();
            if parts.next() != Some("freshness") {
                return None;
            }
            Some(parts.find_map(|p| {
                p.strip_prefix("maxAgeSeconds=")
                    .and_then(|n| n.parse().ok())
            }))
        }
        Value::Mapping(m) => {
            if m.get("type").and_then(|v| v.as_str()) != Some("freshness") {
                return None;
            }
            Some(m.get("maxAgeSeconds").and_then(|v| v.as_u64()))
        }
        _ => None,
    }
}

/// `customProperties[]` lookup by `property` name.
fn custom_prop_value<'a>(m: &'a Mapping, key: &str) -> Option<&'a Value> {
    m.get("customProperties")?
        .as_sequence()?
        .iter()
        .filter_map(|cp| cp.as_mapping())
        .find(|cp| cp.get("property").and_then(|v| v.as_str()) == Some(key))
        .and_then(|cp| cp.get("value"))
}

/// Parse a property's `quality[]` into CG `QualityRule`s (Mode B).
///
/// Order of authority: `x-cg-quality` verbatim rule › legacy CG `type`
/// vocabulary (pre-strict exports) › ODCS library metrics / custom engine.
/// Entries marked `x-cg-derived` restate constraints and are skipped;
/// `rowCount` and other dataset-level metrics have no CG equivalent.
fn odcs_quality_to_rules(field_name: &str, quality_seq: &[Value]) -> Vec<QualityRule> {
    quality_seq
        .iter()
        .filter_map(|q| {
            let m = q.as_mapping()?;
            if custom_prop_value(m, X_CG_DERIVED).is_some() {
                return None;
            }
            if let Some(raw) = custom_prop_value(m, X_CG_QUALITY) {
                if let Ok(mut rule) = serde_yaml::from_value::<QualityRule>(raw.clone()) {
                    rule.field = field_name.to_string();
                    return Some(rule);
                }
            }
            let description = m
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            let type_str = m.get("type").and_then(|v| v.as_str()).unwrap_or("library");
            let attrs = m.get("attributes").and_then(|v| v.as_mapping());
            let (rule_type, threshold, max_age_seconds) = match type_str {
                // Legacy (pre-strict) DCG vocabulary.
                "completeness" | "validity" | "freshness" | "uniqueness" => {
                    let rt = match type_str {
                        "completeness" => QualityRuleType::Completeness,
                        "validity" => QualityRuleType::Validity,
                        "freshness" => QualityRuleType::Freshness,
                        _ => QualityRuleType::Uniqueness,
                    };
                    (
                        rt,
                        attrs
                            .and_then(|a| a.get("threshold"))
                            .and_then(|v| v.as_f64()),
                        attrs
                            .and_then(|a| a.get("maxAgeSeconds"))
                            .and_then(|v| v.as_u64()),
                    )
                }
                "library" => {
                    let rt = match m.get("metric").and_then(|v| v.as_str())? {
                        "missingValues" | "nullValues" => QualityRuleType::Completeness,
                        "duplicateValues" => QualityRuleType::Uniqueness,
                        "invalidValues" => QualityRuleType::Validity,
                        _ => return None, // rowCount etc.
                    };
                    (rt, threshold_from_odcs(m), None)
                }
                "custom" => {
                    if m.get("engine").and_then(|v| v.as_str()) != Some(CG_ENGINE) {
                        return None;
                    }
                    (
                        QualityRuleType::Freshness,
                        None,
                        parse_cg_freshness(m.get("implementation")?)?,
                    )
                }
                _ => return None,
            };
            let scope =
                matches!(rule_type, QualityRuleType::Uniqueness).then_some(UniqueScope::Batch);
            Some(QualityRule {
                field: field_name.to_string(),
                rule_type,
                description,
                max_age_seconds,
                scope,
                threshold,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Inputs needed to build an ODCS document.  Callers load these three rows
/// from the DB and pass references — no ownership required.
pub struct OdcsExportInput<'a> {
    pub identity: &'a ContractIdentity,
    pub version: &'a ContractVersion,
    pub contract: &'a Contract,
}

/// Serialize a CG contract version as an ODCS v3.1.0 YAML document that
/// validates against the official ODCS JSON Schema.
///
/// `x-contractgate-*` extensions (D-003) are written as entries of the
/// top-level `customProperties[]` array (the schema forbids unknown root
/// keys) so the document can be re-imported losslessly (Mode A).
///
/// # PII safety
/// `pii_salt` is NEVER included.  The `ContractIdentity` is used only for
/// `id`, `name`, and `description`.
pub fn export_odcs(input: OdcsExportInput<'_>) -> Result<String, String> {
    let OdcsExportInput {
        identity,
        version,
        contract,
    } = input;

    let mut doc = Mapping::new();

    // ── Mandatory ODCS fields ────────────────────────────────────────────────
    doc.insert(v_str("apiVersion"), v_str(ODCS_API_VERSION));
    doc.insert(v_str("kind"), v_str("DataContract"));
    doc.insert(v_str("id"), v_str(identity.id.to_string()));
    doc.insert(v_str("name"), v_str(&identity.name));
    doc.insert(v_str("version"), v_str(&version.version));
    doc.insert(
        v_str("status"),
        v_str(version_state_to_odcs_status(version.state)),
    );

    // ── Recommended ODCS fields ──────────────────────────────────────────────
    // D-004: name populates both dataProduct and schema[0].name
    doc.insert(v_str("dataProduct"), v_str(&identity.name));
    // ODCS `description` is an object; the plain text goes in `purpose`.
    if let Some(desc) = &identity.description {
        let mut d = Mapping::new();
        d.insert(v_str("purpose"), v_str(desc));
        doc.insert(v_str("description"), Value::Mapping(d));
    }

    // ── schema[] section ─────────────────────────────────────────────────────
    let mut schema_entry = Mapping::new();
    schema_entry.insert(v_str("name"), v_str(&identity.name)); // D-004
    schema_entry.insert(v_str("logicalType"), v_str("object"));
    if let Some(desc) = &contract.description {
        schema_entry.insert(v_str("description"), v_str(desc));
    }
    let properties = flatten_to_odcs_properties("", &contract.ontology.entities, &contract.quality);
    schema_entry.insert(v_str("properties"), Value::Sequence(properties));
    doc.insert(
        v_str("schema"),
        Value::Sequence(vec![Value::Mapping(schema_entry)]),
    );

    // ── x-contractgate-* extensions (root customProperties[]) ────────────────
    let mut ext: Vec<Value> = Vec::new();
    // Extension *format* version (internal ODCS extension strategy §6).
    ext.push(custom_prop(
        "x-contractgate-version",
        v_str(EXT_FORMAT_VERSION),
    ));
    // Verbatim CG contract as a YAML subtree (Mode A source of truth).
    let ontology_value = serde_yaml::to_value(contract)
        .map_err(|e| format!("failed to serialize contract ontology: {e}"))?;
    ext.push(custom_prop("x-contractgate-ontology", ontology_value));
    if !contract.glossary.is_empty() {
        let v = serde_yaml::to_value(&contract.glossary)
            .map_err(|e| format!("failed to serialize glossary: {e}"))?;
        ext.push(custom_prop("x-contractgate-glossary", v));
    }
    if !contract.metrics.is_empty() {
        let v = serde_yaml::to_value(&contract.metrics)
            .map_err(|e| format!("failed to serialize metrics: {e}"))?;
        ext.push(custom_prop("x-contractgate-metrics", v));
    }
    // Only written when true to keep docs tidy.
    if contract.compliance_mode {
        ext.push(custom_prop(
            "x-contractgate-compliance-mode",
            Value::Bool(true),
        ));
    }
    doc.insert(v_str("customProperties"), Value::Sequence(ext));

    serde_yaml::to_string(&Value::Mapping(doc))
        .map_err(|e| format!("failed to serialize ODCS document: {e}"))
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Result of a successful ODCS import.
pub struct ImportResult {
    /// The ODCS `version` field — used as the CG version string.
    pub version: String,
    /// CG contract YAML, ready to store as `yaml_content`.
    pub yaml_content: String,
    /// Fidelity level — `Odcs` (lossless) or `OdcsStripped` (best-effort).
    pub import_source: ImportSource,
}

/// Look up a top-level `x-contractgate-*` extension in either location:
/// the root `customProperties[]` array (current, schema-valid) or a bare
/// root key (exports before feat/odcs-strict).
pub fn root_extension<'a>(doc: &'a Mapping, key: &str) -> Option<&'a Value> {
    custom_prop_value(doc, key).or_else(|| doc.get(key))
}

/// ODCS `description`: a string (legacy) or the v3 object form
/// (`purpose` › `usage` › `limitations`).
fn description_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Mapping(m) => ["purpose", "usage", "limitations"]
            .iter()
            .find_map(|k| m.get(*k).and_then(|v| v.as_str()))
            .map(String::from),
        _ => None,
    }
}

/// Parse an ODCS v3.x YAML document and return a CG contract.
///
/// **Mode A — lossless:** an `x-contractgate-version` extension is present
/// (root `customProperties[]` or legacy root key).  The
/// `x-contractgate-ontology` subtree is the YAML content.
/// `import_source = Odcs`.
///
/// **Mode B — stripped / foreign:** a best-effort `Contract` is rebuilt from
/// `schema[0].properties[]`: ODCS-native fields (`logicalTypeOptions`,
/// `enum`, `unique`, `required`, library quality metrics) are read first and
/// `x-cg-*` customProperties win where both exist. Property `enum` accepts
/// the ODCS v3.2.0 `{value: ...}` item list and a scalar list. PII
/// transforms, glossary, metrics, and dataset-level rules (`rowCount`) are
/// not recoverable — the caller sets `requires_review = true` (D-002).
pub fn import_odcs(yaml: &str) -> Result<ImportResult, String> {
    let doc: Value = serde_yaml::from_str(yaml).map_err(|e| format!("invalid ODCS YAML: {e}"))?;

    let mapping = doc
        .as_mapping()
        .ok_or_else(|| "ODCS document must be a YAML mapping".to_string())?;

    // Extract the mandatory `version` field.
    let version = mapping
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "ODCS document missing required 'version' field".to_string())?
        .to_string();

    check_api_version(mapping)?;

    if root_extension(mapping, "x-contractgate-version").is_some() {
        import_mode_a(mapping, version)
    } else {
        import_mode_b(mapping, version)
    }
}

/// Accept `v3.1.0` and `v3.2.0`, and any other `v3.*` string, so a document
/// that imported before property-level enums landed still imports. A missing
/// `apiVersion` is allowed. A non-string value is rejected. Non-3.x versions
/// are rejected.
fn check_api_version(doc: &Mapping) -> Result<(), String> {
    let Some(api) = doc.get("apiVersion") else {
        return Ok(());
    };
    let Some(version) = api.as_str() else {
        return Err("ODCS apiVersion must be a string".to_string());
    };
    if version.starts_with("v3.") {
        return Ok(());
    }
    Err(format!(
        "unsupported ODCS apiVersion {version:?}; supported: v3.1.0, v3.2.0"
    ))
}

// ── Mode A (lossless) ────────────────────────────────────────────────────────

fn import_mode_a(doc: &Mapping, version: String) -> Result<ImportResult, String> {
    let ontology = root_extension(doc, "x-contractgate-ontology")
        .ok_or_else(|| {
            "ODCS document has x-contractgate-version but is missing \
             x-contractgate-ontology"
                .to_string()
        })?
        .clone();
    // Tools whose custom-property values must be strings (OpenMetadata
    // 2.0.x) carry the subtree as serialized YAML/JSON text.
    let ontology = match ontology {
        Value::String(text) => serde_yaml::from_str(&text)
            .map_err(|e| format!("x-contractgate-ontology string is not YAML: {e}"))?,
        other => other,
    };

    // Re-serialize the ontology subtree to a YAML string.  Key ordering may
    // differ from the original, but the Contract is semantically identical.
    let yaml_content = serde_yaml::to_string(&ontology)
        .map_err(|e| format!("failed to serialize x-contractgate-ontology: {e}"))?;
    // The ODCS document's top-level `version` is the version row (export
    // writes it from the row); make the embedded contract agree with it.
    let yaml_content = crate::version_pin::set_yaml_version(&yaml_content, &version);

    let _: Contract = serde_yaml::from_str(&yaml_content)
        .map_err(|e| format!("x-contractgate-ontology is not a valid Contract: {e}"))?;

    Ok(ImportResult {
        version,
        yaml_content,
        import_source: ImportSource::Odcs,
    })
}

// ── Mode B (stripped / foreign ODCS) ─────────────────────────────────────────

fn import_mode_b(doc: &Mapping, version: String) -> Result<ImportResult, String> {
    let schema_seq = doc
        .get("schema")
        .and_then(|v| v.as_sequence())
        .ok_or_else(|| "ODCS document missing 'schema' array".to_string())?;

    let schema0 = schema_seq
        .first()
        .and_then(|v| v.as_mapping())
        .ok_or_else(|| "ODCS 'schema' array is empty or not a mapping".to_string())?;

    // Contract name: dataProduct › schema[0].name › root name.
    let name = doc
        .get("dataProduct")
        .and_then(|v| v.as_str())
        .or_else(|| schema0.get("name").and_then(|v| v.as_str()))
        .or_else(|| doc.get("name").and_then(|v| v.as_str()))
        .unwrap_or("imported_contract")
        .to_string();

    let description = description_text(doc.get("description"))
        .or_else(|| description_text(schema0.get("description")));

    let (entities, quality) =
        if let Some(props) = schema0.get("properties").and_then(|v| v.as_sequence()) {
            let fields = unflatten_odcs_properties(props)?;
            let rules = extract_quality_rules_from_props(props);
            (fields, rules)
        } else {
            (vec![], vec![])
        };

    let compliance_mode = root_extension(doc, "x-contractgate-compliance-mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let contract = Contract {
        version: version.clone(),
        name,
        description,
        compliance_mode,
        egress_leakage_mode: EgressLeakageMode::Off,
        envelope: None,
        null_as_absent: false,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality,
        on_fail: crate::contract::OnFailMode::Quarantine,
        on_fail_locked: false,
        quarantine_guard: crate::contract::QuarantineGuardConfig::default(),
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| format!("failed to serialize reconstructed Contract: {e}"))?;

    Ok(ImportResult {
        version,
        yaml_content,
        import_source: ImportSource::OdcsStripped,
    })
}

/// Collect quality rules across `schema[0].properties[]`.  The dot-notation
/// property name is stamped onto each rule; `unique: true` adds a batch
/// uniqueness rule when `quality[]` didn't already declare one.
fn extract_quality_rules_from_props(props: &[Value]) -> Vec<QualityRule> {
    let mut all_rules: Vec<QualityRule> = Vec::new();
    for p in props {
        let Some(m) = p.as_mapping() else { continue };
        let Some(name) = m.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut rules = m
            .get("quality")
            .and_then(|v| v.as_sequence())
            .map(|q| odcs_quality_to_rules(name, q))
            .unwrap_or_default();
        let unique = m.get("unique").and_then(|v| v.as_bool()) == Some(true);
        if unique
            && !rules
                .iter()
                .any(|r| r.rule_type == QualityRuleType::Uniqueness)
        {
            rules.push(QualityRule {
                field: name.to_string(),
                rule_type: QualityRuleType::Uniqueness,
                description: None,
                max_age_seconds: None,
                scope: Some(UniqueScope::Batch),
                threshold: None,
            });
        }
        all_rules.extend(rules);
    }
    all_rules
}

/// Reverse D-001 flattening: convert ODCS dot-notation property list back to a
/// nested CG field tree.
fn unflatten_odcs_properties(props: &[Value]) -> Result<Vec<FieldDefinition>, String> {
    let mut pairs: Vec<(String, FieldDefinition)> = Vec::new();
    for p in props {
        let m = p
            .as_mapping()
            .ok_or_else(|| "ODCS property entry is not a mapping".to_string())?;
        let name = m
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "ODCS property missing 'name'".to_string())?
            .to_string();
        let field_def = odcs_prop_to_field(m, &name)?;
        pairs.push((name, field_def));
    }

    build_field_tree(pairs)
}

/// Group flat (dot-notation name, FieldDefinition) pairs into a nested tree.
fn build_field_tree(pairs: Vec<(String, FieldDefinition)>) -> Result<Vec<FieldDefinition>, String> {
    let mut top_level: Vec<FieldDefinition> = Vec::new();
    let mut nested: std::collections::BTreeMap<String, Vec<(String, FieldDefinition)>> =
        std::collections::BTreeMap::new();

    for (name, field) in pairs {
        if let Some(dot_pos) = name.find('.') {
            let parent = name[..dot_pos].to_string();
            let rest = name[dot_pos + 1..].to_string();
            let mut child = field;
            child.name = rest.clone();
            nested.entry(parent).or_default().push((rest, child));
        } else {
            top_level.push(field);
        }
    }

    for (parent_name, children) in nested {
        let child_fields = build_field_tree(children)?;
        if let Some(existing) = top_level
            .iter_mut()
            .find(|f| f.name == parent_name && f.field_type == FieldType::Object)
        {
            existing
                .properties
                .get_or_insert_with(Vec::new)
                .extend(child_fields);
            continue;
        }
        top_level.push(FieldDefinition {
            name: parent_name,
            field_type: FieldType::Object,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: Some(child_fields),
            items: None,
            transform: None,
            format: None,
        });
    }

    Ok(top_level)
}

fn yaml_to_json_values(seq: &[Value]) -> Vec<serde_json::Value> {
    seq.iter()
        .map(|v| match v {
            // ODCS v3.2 enum form: `- value: x`
            Value::Mapping(m) if m.contains_key("value") => {
                m.get("value").cloned().unwrap_or(Value::Null)
            }
            other => other.clone(),
        })
        .filter_map(|v| serde_json::to_value(v).ok())
        .collect()
}

/// One ODCS v3.2 enum entry: a scalar, or an `EnumValue` object
/// (`value` plus optional `label` / `description`). Metadata is not part of
/// the allowed value. Nested objects and arrays are not allowed values.
fn odcs_enum_item(v: &Value) -> Option<serde_json::Value> {
    let raw = match v {
        Value::Mapping(m) => m.get("value")?.clone(),
        Value::Sequence(_) => return None,
        other => other.clone(),
    };
    let json = match raw {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::String(s) => serde_json::Value::String(s),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(u.into())
            } else {
                serde_json::to_value(n).ok()?
            }
        }
        other => {
            let json = serde_json::to_value(other).ok()?;
            if json.is_object() || json.is_array() {
                return None;
            }
            json
        }
    };
    Some(json)
}

/// Property-level `enum` from ODCS v3.2.0 (and the same list when a producer
/// still puts it on `logicalTypeOptions` or a custom property named `enum`).
///
/// Official items are `{ value: ... }`. Scalar lists are accepted as well:
/// string, number, and boolean entries, which `datacontract` import/export
/// still writes. An empty list is treated as absent so a later
/// `invalidValues` / `validValues` rule can still fill the constraint.
fn parse_odcs_enum(v: Option<&Value>) -> Option<Vec<serde_json::Value>> {
    let seq = v.and_then(|v| v.as_sequence())?;
    let values: Vec<serde_json::Value> = seq.iter().filter_map(odcs_enum_item).collect();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Parse a single ODCS property mapping into a CG `FieldDefinition`.
/// ODCS-native fields are read first; `x-cg-*` customProperties override.
fn odcs_prop_to_field(m: &Mapping, full_name: &str) -> Result<FieldDefinition, String> {
    let leaf_name = full_name
        .rsplit('.')
        .next()
        .unwrap_or(full_name)
        .to_string();

    let logical = custom_prop_value(m, "x-cg-type")
        .and_then(|v| v.as_str())
        .or_else(|| m.get("logicalType").and_then(|v| v.as_str()))
        .unwrap_or("string");
    let field_type = logical_to_field_type(logical);

    let required = m.get("required").and_then(|v| v.as_bool()).unwrap_or(false);

    // ── ODCS-native constraints ──────────────────────────────────────────────
    let opts = m.get("logicalTypeOptions").and_then(|v| v.as_mapping());
    let opt = |k: &str| opts.and_then(|o| o.get(k));
    let mut pattern = opt("pattern").and_then(|v| v.as_str()).map(String::from);
    let mut min_length = opt("minLength")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let mut max_length = opt("maxLength")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    // Date bounds are strings in ODCS; only numeric bounds map to CG min/max.
    let mut min = opt("minimum").and_then(|v| v.as_f64());
    let mut max = opt("maximum").and_then(|v| v.as_f64());
    let mut format = opt("format").and_then(|v| v.as_str()).map(String::from);
    if field_type == FieldType::Date && format.as_deref() == Some(CG_DATE_FORMAT) {
        format = None; // CG's built-in date format
    }
    // Precedence matches datacontract-cli v1.2.0: property `enum`, then
    // `logicalTypeOptions.enum`, then a custom property named `enum`, then
    // the ODCS v3.1 `invalidValues` / `validValues` rule. `x-cg-enum` still
    // wins below, so a DataContractGate export re-imports losslessly.
    let mut allowed_values = parse_odcs_enum(m.get("enum"))
        .or_else(|| parse_odcs_enum(opt("enum")))
        .or_else(|| parse_odcs_enum(custom_prop_value(m, "enum")));
    // invalidValues library rules carry validValues / pattern (ODCS v3.1 has
    // no property-level enum).
    if let Some(qs) = m.get("quality").and_then(|v| v.as_sequence()) {
        for q in qs.iter().filter_map(|q| q.as_mapping()) {
            if q.get("metric").and_then(|v| v.as_str()) != Some("invalidValues") {
                continue;
            }
            let Some(args) = q.get("arguments").and_then(|v| v.as_mapping()) else {
                continue;
            };
            if allowed_values.is_none() {
                allowed_values = args
                    .get("validValues")
                    .and_then(|v| v.as_sequence())
                    .map(|s| yaml_to_json_values(s));
            }
            if pattern.is_none() {
                pattern = args
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }
    }

    // ── x-cg-* customProperties win ──────────────────────────────────────────
    if let Some(customs) = m.get("customProperties").and_then(|v| v.as_sequence()) {
        for cp in customs {
            let Some(cp_map) = cp.as_mapping() else {
                continue;
            };
            let Some(prop_key) = cp_map.get("property").and_then(|v| v.as_str()) else {
                continue;
            };
            let val = cp_map.get("value");
            match prop_key {
                "x-cg-pattern" => {
                    pattern = val.and_then(|v| v.as_str()).map(String::from);
                }
                "x-cg-enum" => {
                    if let Some(seq) = val.and_then(|v| v.as_sequence()) {
                        allowed_values = Some(yaml_to_json_values(seq));
                    }
                }
                "x-cg-min" => min = val.and_then(|v| v.as_f64()),
                "x-cg-max" => max = val.and_then(|v| v.as_f64()),
                "x-cg-min-length" => {
                    min_length = val.and_then(|v| v.as_u64()).map(|n| n as usize);
                }
                "x-cg-max-length" => {
                    max_length = val.and_then(|v| v.as_u64()).map(|n| n as usize);
                }
                "x-cg-format" => format = val.and_then(|v| v.as_str()).map(String::from),
                // Transform kind/style not reconstructed in Mode B —
                // transform secrets (salt) are absent; review required (D-002).
                _ => {}
            }
        }
    }

    // ODCS v3 nests object fields natively under `properties[]` and array
    // element schemas under `items`; convert both recursively.
    let properties = match m.get("properties").and_then(|v| v.as_sequence()) {
        Some(seq) if field_type == FieldType::Object => Some(unflatten_odcs_properties(seq)?),
        _ => None,
    };
    let items = match m.get("items").and_then(|v| v.as_mapping()) {
        Some(im) if field_type == FieldType::Array => {
            let item_name = im.get("name").and_then(|v| v.as_str()).unwrap_or("item");
            Some(Box::new(odcs_prop_to_field(im, item_name)?))
        }
        _ => None,
    };

    Ok(FieldDefinition {
        name: leaf_name,
        field_type,
        required,
        pattern,
        allowed_values,
        min,
        max,
        min_length,
        max_length,
        properties,
        items,
        transform: None,
        format,
    })
}

// ---------------------------------------------------------------------------
// Import request / response API types (wired in main.rs)
// ---------------------------------------------------------------------------

/// Request body for `POST /contracts/import`.
#[derive(Debug, serde::Deserialize)]
pub struct ImportOdcsRequest {
    /// Raw ODCS YAML document.
    pub odcs_yaml: String,
    /// Optional contract identity name override.  When absent, the importer
    /// reads `dataProduct` (or `schema[0].name`) from the ODCS document.
    #[serde(default)]
    pub name_override: Option<String>,
    /// Import as a new version on this existing contract (must be owned by
    /// the caller). Alias: `contract_id`.
    #[serde(default)]
    pub target_contract_id: Option<uuid::Uuid>,
    /// Alias of `target_contract_id`.
    #[serde(default)]
    pub contract_id: Option<uuid::Uuid>,
    /// Org id for multi-tenant scoping (passed through from the auth layer).
    #[serde(default)]
    #[allow(dead_code)]
    pub org_id: Option<uuid::Uuid>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn v_str(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

fn custom_prop(key: &str, value: Value) -> Value {
    let mut m = Mapping::new();
    m.insert(v_str("property"), v_str(key));
    m.insert(v_str("value"), value);
    Value::Mapping(m)
}

fn num(v: f64) -> Value {
    serde_yaml::to_value(v).unwrap_or(Value::Null)
}

fn json_seq(vals: &[serde_json::Value]) -> Value {
    Value::Sequence(
        vals.iter()
            .map(|v| serde_yaml::to_value(v).unwrap_or(Value::Null))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HEADER: &str = "\
kind: DataContract
id: fx
version: \"1.0.0\"
status: active
schema:
  - name: fx
    properties:
";

    fn imported(yaml: &str) -> Contract {
        let result = import_odcs(yaml).unwrap_or_else(|err| panic!("import: {err}"));
        serde_yaml::from_str(&result.yaml_content).unwrap_or_else(|err| panic!("contract: {err}"))
    }

    fn field_named(yaml_body: &str) -> FieldDefinition {
        let contract = imported(&format!("apiVersion: v3.2.0\n{HEADER}{yaml_body}"));
        contract
            .ontology
            .entities
            .into_iter()
            .next()
            .expect("one field")
    }

    fn allowed(body: &str) -> Option<Vec<serde_json::Value>> {
        field_named(body).allowed_values
    }

    #[test]
    fn v31_and_v32_api_versions_import() {
        for api in ["v3.1.0", "v3.2.0", "v3.0.2"] {
            let yaml = format!(
                "apiVersion: {api}\n{HEADER}      - name: id\n        logicalType: string\n"
            );
            let contract = imported(&yaml);
            assert_eq!(contract.ontology.entities[0].name, "id", "{api}");
        }
    }

    #[test]
    fn missing_api_version_still_imports() {
        let yaml = format!("{HEADER}      - name: id\n        logicalType: string\n");
        let contract = imported(&yaml);
        assert_eq!(contract.ontology.entities[0].name, "id");
    }

    #[test]
    fn non_string_and_non_v3_api_versions_are_rejected() {
        let bad_type =
            format!("apiVersion: true\n{HEADER}      - name: id\n        logicalType: string\n");
        let type_err = match import_odcs(&bad_type) {
            Err(err) => err,
            Ok(_) => panic!("bool apiVersion must be rejected"),
        };
        assert!(type_err.contains("must be a string"), "{type_err}");

        let bad_major = format!(
            "apiVersion: \"2.2.2\"\n{HEADER}      - name: id\n        logicalType: string\n"
        );
        let major_err = match import_odcs(&bad_major) {
            Err(err) => err,
            Ok(_) => panic!("v2.2.2 must be rejected"),
        };
        assert!(
            major_err.contains("unsupported ODCS apiVersion"),
            "{major_err}"
        );
        assert!(major_err.contains("v3.1.0"), "{major_err}");
        assert!(major_err.contains("v3.2.0"), "{major_err}");
    }

    #[test]
    fn scalar_string_enum_and_value_objects_map_to_field_enum() {
        assert_eq!(
            allowed("      - name: currency\n        logicalType: string\n        enum: [\"USD\", \"EUR\", \"GBP\"]\n"),
            Some(vec![json!("USD"), json!("EUR"), json!("GBP")])
        );
        assert_eq!(
            allowed(
                "      - name: currency\n        logicalType: string\n        enum:\n          - value: USD\n            label: US Dollar\n          - value: EUR\n            description: Euro\n"
            ),
            Some(vec![json!("USD"), json!("EUR")])
        );
    }

    #[test]
    fn v31_property_enum_is_accepted() {
        let yaml = format!(
            "apiVersion: v3.1.0\n{HEADER}      - name: currency\n        logicalType: string\n        enum:\n          - USD\n          - EUR\n"
        );
        let contract = imported(&yaml);
        assert_eq!(
            contract.ontology.entities[0].allowed_values,
            Some(vec![json!("USD"), json!("EUR")])
        );
    }

    #[test]
    fn empty_enum_array_is_absent_and_falls_through_to_valid_values() {
        assert_eq!(
            allowed("      - name: status\n        logicalType: string\n        enum: []\n"),
            None
        );
        assert_eq!(
            allowed(
                "      - name: status\n        logicalType: string\n        enum: []\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [open, closed]\n"
            ),
            Some(vec![json!("open"), json!("closed")])
        );
        assert_eq!(
            allowed(
                "      - name: status\n        logicalType: string\n        enum:\n          - label: ignored\n          - value:\n              nested: true\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [kept]\n"
            ),
            Some(vec![json!("kept")])
        );
    }

    #[test]
    fn mixed_and_duplicate_enum_values_keep_order() {
        assert_eq!(
            allowed(
                "      - name: status\n        logicalType: string\n        enum:\n          - pending\n          - value: pending\n            label: Pending\n          - value: shipped\n          - value:\n              nested: drop\n          - cancelled\n          - cancelled\n"
            ),
            Some(vec![
                json!("pending"),
                json!("pending"),
                json!("shipped"),
                json!("cancelled"),
                json!("cancelled"),
            ])
        );
    }

    #[test]
    fn integer_and_boolean_enum_values_keep_their_types() {
        assert_eq!(
            allowed(
                "      - name: priority\n        logicalType: integer\n        enum:\n          - value: 1\n          - value: 2\n"
            ),
            Some(vec![json!(1), json!(2)])
        );
        assert_eq!(
            allowed(
                "      - name: active\n        logicalType: boolean\n        enum: [true, false]\n"
            ),
            Some(vec![json!(true), json!(false)])
        );
    }

    #[test]
    fn property_enum_beats_quality_valid_values_and_option_fallbacks() {
        assert_eq!(
            allowed(
                "      - name: currency\n        logicalType: string\n        enum: [\"USD\"]\n        logicalTypeOptions:\n          enum: [\"EUR\"]\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [\"GBP\"]\n        customProperties:\n          - property: enum\n            value: [\"JPY\"]\n"
            ),
            Some(vec![json!("USD")])
        );
        assert_eq!(
            allowed(
                "      - name: channel\n        logicalType: string\n        logicalTypeOptions:\n          enum: [web, mobile]\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [store]\n"
            ),
            Some(vec![json!("web"), json!("mobile")])
        );
        assert_eq!(
            allowed(
                "      - name: region\n        logicalType: string\n        customProperties:\n          - property: enum\n            value: [us, eu]\n"
            ),
            Some(vec![json!("us"), json!("eu")])
        );
    }

    #[test]
    fn x_cg_enum_beats_property_enum_and_valid_values() {
        assert_eq!(
            allowed(
                "      - name: winner\n        logicalType: string\n        enum: [property]\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [quality]\n        customProperties:\n          - property: x-cg-enum\n            value: [extension]\n"
            ),
            Some(vec![json!("extension")])
        );
    }

    #[test]
    fn property_enum_fills_when_quality_rule_has_pattern_only() {
        let field = field_named(
            "      - name: code\n        logicalType: string\n        enum: [\"A\", \"B\"]\n        quality:\n          - metric: invalidValues\n            arguments:\n              pattern: \"^[A-Z]$\"\n",
        );
        assert_eq!(field.allowed_values, Some(vec![json!("A"), json!("B")]));
        assert_eq!(field.pattern.as_deref(), Some("^[A-Z]$"));
    }

    #[test]
    fn quality_valid_values_apply_when_no_property_enum_is_present() {
        let yaml = format!(
            "apiVersion: v3.1.0\n{HEADER}      - name: status\n        logicalType: string\n        quality:\n          - metric: invalidValues\n            arguments:\n              validValues: [open, shipped]\n"
        );
        let contract = imported(&yaml);
        assert_eq!(
            contract.ontology.entities[0].allowed_values,
            Some(vec![json!("open"), json!("shipped")])
        );
    }

    #[test]
    fn nested_property_enum_is_imported() {
        let yaml = format!(
            "apiVersion: v3.2.0\n{HEADER}      - name: price\n        logicalType: object\n        properties:\n          - name: currency\n            logicalType: string\n            enum:\n              - value: USD\n              - value: EUR\n"
        );
        let contract = imported(&yaml);
        let nested = contract.ontology.entities[0]
            .properties
            .as_ref()
            .expect("nested");
        assert_eq!(
            nested[0].allowed_values,
            Some(vec![json!("USD"), json!("EUR")])
        );
    }
}
