//! ODCS v3.1.0 import / export for ContractGate.
//!
//! # Round-trip guarantee
//!
//! Export always writes `x-contractgate-*` extensions (D-003).  Import reads
//! those extensions as authoritative (Mode A — lossless).  For foreign ODCS
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
// FieldType ↔ ODCS logicalType mapping
// ---------------------------------------------------------------------------

fn field_type_to_logical(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::String => "string",
        FieldType::Integer => "integer",
        FieldType::Float => "double",
        FieldType::Boolean => "boolean",
        FieldType::Object => "object",
        FieldType::Array => "array",
        FieldType::Any => "any",
    }
}

fn logical_to_field_type(s: &str) -> FieldType {
    match s {
        "integer" | "long" | "int" => FieldType::Integer,
        "double" | "float" | "decimal" | "number" => FieldType::Float,
        "boolean" | "bool" => FieldType::Boolean,
        "object" | "record" | "struct" => FieldType::Object,
        "array" | "list" => FieldType::Array,
        _ => FieldType::String,
    }
}

// ---------------------------------------------------------------------------
// Property flattening (D-001)
// ---------------------------------------------------------------------------

/// Flatten a CG field tree to ODCS `schema[].properties[]`.
/// Nested objects are expanded to dot-notation names, e.g.
/// `user.address.street`.  The extension data (`customProperties`) carries
/// all scalar constraints so an import can reconstruct the original.
/// Quality rules are matched by dot-notation field name and written into
/// each property's `quality[]` array for ODCS-native consumers.
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

        if let (FieldType::Object, Some(nested)) = (&f.field_type, &f.properties) {
            // Recurse — the parent "object" node is omitted from ODCS; only
            // leaves appear as properties (D-001).
            out.extend(flatten_to_odcs_properties(&name, nested, quality_rules));
        } else {
            // Collect only the quality rules that target this exact field path.
            let prop_quality: Vec<&QualityRule> =
                quality_rules.iter().filter(|r| r.field == name).collect();
            out.push(build_odcs_property(&name, f, &prop_quality));
        }
    }
    out
}

/// Build one ODCS property entry for a leaf field.
/// `prop_quality` contains only the quality rules that target this property.
fn build_odcs_property(name: &str, f: &FieldDefinition, prop_quality: &[&QualityRule]) -> Value {
    let mut prop = Mapping::new();
    prop.insert(v_str("name"), v_str(name));
    prop.insert(
        v_str("logicalType"),
        v_str(field_type_to_logical(&f.field_type)),
    );
    prop.insert(v_str("required"), Value::Bool(f.required));

    // classification: "pii" when a PII transform is declared
    if f.transform.is_some() {
        prop.insert(v_str("classification"), v_str("pii"));
    }

    // quality[]: ODCS-native quality entries for this property.
    let quality_entries: Vec<Value> = prop_quality
        .iter()
        .map(|r| quality_rule_to_odcs(r))
        .collect();
    prop.insert(v_str("quality"), Value::Sequence(quality_entries));

    // customProperties: scalar constraints (round-trip safe)
    let mut custom: Vec<Value> = Vec::new();
    if let Some(pat) = &f.pattern {
        custom.push(custom_prop("x-cg-pattern", v_str(pat)));
    }
    if let Some(vals) = &f.allowed_values {
        let arr = Value::Sequence(
            vals.iter()
                .map(|v| serde_yaml::to_value(v).unwrap_or(Value::Null))
                .collect(),
        );
        custom.push(custom_prop("x-cg-enum", arr));
    }
    if let Some(v) = f.min {
        custom.push(custom_prop(
            "x-cg-min",
            serde_yaml::to_value(v).unwrap_or(Value::Null),
        ));
    }
    if let Some(v) = f.max {
        custom.push(custom_prop(
            "x-cg-max",
            serde_yaml::to_value(v).unwrap_or(Value::Null),
        ));
    }
    if let Some(v) = f.min_length {
        custom.push(custom_prop(
            "x-cg-min-length",
            serde_yaml::to_value(v as u64).unwrap_or(Value::Null),
        ));
    }
    if let Some(v) = f.max_length {
        custom.push(custom_prop(
            "x-cg-max-length",
            serde_yaml::to_value(v as u64).unwrap_or(Value::Null),
        ));
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

    Value::Mapping(prop)
}

// ---------------------------------------------------------------------------
// Quality rule ↔ ODCS quality[] mapping
// ---------------------------------------------------------------------------

/// Serialize a CG `QualityRule` to an ODCS `quality[]` entry.
///
/// ODCS quality entries use:
/// - `type`: one of `completeness | validity | freshness | uniqueness`
/// - `description`: optional human label
/// - `attributes`: type-specific parameters (e.g. `maxAgeSeconds`)
fn quality_rule_to_odcs(rule: &QualityRule) -> Value {
    let mut m = Mapping::new();
    m.insert(
        v_str("type"),
        v_str(match rule.rule_type {
            QualityRuleType::Completeness => "completeness",
            QualityRuleType::Validity => "validity",
            QualityRuleType::Freshness => "freshness",
            QualityRuleType::Uniqueness => "uniqueness",
        }),
    );
    if let Some(desc) = &rule.description {
        m.insert(v_str("description"), v_str(desc));
    }

    let mut attrs = Mapping::new();
    if let Some(max_age) = rule.max_age_seconds {
        attrs.insert(
            v_str("maxAgeSeconds"),
            serde_yaml::to_value(max_age).unwrap_or(Value::Null),
        );
    }
    if let Some(threshold) = rule.threshold {
        attrs.insert(
            v_str("threshold"),
            serde_yaml::to_value(threshold).unwrap_or(Value::Null),
        );
    }
    if !attrs.is_empty() {
        m.insert(v_str("attributes"), Value::Mapping(attrs));
    }

    Value::Mapping(m)
}

/// Parse the ODCS `quality[]` array on a single property back into CG
/// `QualityRule` structs.  Used by Mode B import.
///
/// `field_name` is the dot-notation property path (e.g. `"user.email"`) and
/// becomes `QualityRule::field`.
fn odcs_quality_to_rules(field_name: &str, quality_seq: &[Value]) -> Vec<QualityRule> {
    quality_seq
        .iter()
        .filter_map(|q| {
            let m = q.as_mapping()?;
            let type_str = m.get("type").and_then(|v| v.as_str())?;
            let rule_type = match type_str {
                "completeness" => QualityRuleType::Completeness,
                "validity" => QualityRuleType::Validity,
                "freshness" => QualityRuleType::Freshness,
                "uniqueness" => QualityRuleType::Uniqueness,
                _ => return None, // unrecognised type — skip
            };
            let description = m
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            let attrs = m.get("attributes").and_then(|v| v.as_mapping());
            let max_age_seconds = attrs
                .and_then(|a| a.get("maxAgeSeconds"))
                .and_then(|v| v.as_u64());
            let threshold = attrs
                .and_then(|a| a.get("threshold"))
                .and_then(|v| v.as_f64());
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

/// Serialize a CG contract version as a valid ODCS v3.1.0 YAML document.
///
/// Always writes `x-contractgate-*` extensions (D-003) so the document can be
/// re-imported losslessly.  The ODCS-native `schema[].properties[]` section is
/// produced in parallel (D-001 dot-notation flattening) for ODCS-native
/// consumers that don't read our extensions.
///
/// # PII safety
/// `pii_salt` is NEVER included.  The `ContractIdentity` is used only for
/// `id` and `name`.
pub fn export_odcs(input: OdcsExportInput<'_>) -> Result<String, String> {
    let OdcsExportInput {
        identity,
        version,
        contract,
    } = input;

    let mut doc = Mapping::new();

    // ── Mandatory ODCS fields ────────────────────────────────────────────────
    doc.insert(v_str("apiVersion"), v_str("v3.1.0"));
    doc.insert(v_str("kind"), v_str("DataContract"));
    doc.insert(v_str("id"), v_str(identity.id.to_string()));
    doc.insert(v_str("version"), v_str(&version.version));
    doc.insert(
        v_str("status"),
        v_str(version_state_to_odcs_status(version.state)),
    );

    // ── Recommended ODCS fields ──────────────────────────────────────────────
    // D-004: name populates both dataProduct and schema[0].name
    doc.insert(v_str("dataProduct"), v_str(&identity.name));
    if let Some(desc) = &identity.description {
        doc.insert(v_str("description"), v_str(desc));
    }

    // ── schema[] section ─────────────────────────────────────────────────────
    let mut schema_entry = Mapping::new();
    schema_entry.insert(v_str("name"), v_str(&identity.name)); // D-004
    if let Some(desc) = &contract.description {
        schema_entry.insert(v_str("description"), v_str(desc));
    }
    let properties = flatten_to_odcs_properties("", &contract.ontology.entities, &contract.quality);
    schema_entry.insert(v_str("properties"), Value::Sequence(properties));

    doc.insert(
        v_str("schema"),
        Value::Sequence(vec![Value::Mapping(schema_entry)]),
    );

    // ── x-contractgate-* extensions ──────────────────────────────────────────
    // Always written (D-003) so imports can reconstruct losslessly.

    doc.insert(v_str("x-contractgate-version"), v_str("1.0"));

    // x-contractgate-ontology: verbatim CG contract as a YAML subtree.
    // Stored as a mapping (not a quoted string) so the ODCS document is valid
    // YAML throughout and can be round-tripped without string escaping.
    let ontology_value = serde_yaml::to_value(contract)
        .map_err(|e| format!("failed to serialize contract ontology: {e}"))?;
    doc.insert(v_str("x-contractgate-ontology"), ontology_value);

    // x-contractgate-glossary
    if !contract.glossary.is_empty() {
        let glossary_value = serde_yaml::to_value(&contract.glossary)
            .map_err(|e| format!("failed to serialize glossary: {e}"))?;
        doc.insert(v_str("x-contractgate-glossary"), glossary_value);
    }

    // x-contractgate-metrics
    if !contract.metrics.is_empty() {
        let metrics_value = serde_yaml::to_value(&contract.metrics)
            .map_err(|e| format!("failed to serialize metrics: {e}"))?;
        doc.insert(v_str("x-contractgate-metrics"), metrics_value);
    }

    // x-contractgate-compliance-mode (only written when true to keep docs tidy)
    if contract.compliance_mode {
        doc.insert(v_str("x-contractgate-compliance-mode"), Value::Bool(true));
    }

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

/// Parse an ODCS v3.1.0 YAML document and return a CG contract.
///
/// **Mode A — lossless:** the document contains `x-contractgate-version`.
/// The `x-contractgate-ontology` subtree is re-serialized as the YAML content.
/// `import_source = Odcs`.
///
/// **Mode B — stripped:** no `x-contractgate-version` key.  A best-effort
/// `Contract` is reconstructed from `schema[0].properties[]`.  Field types,
/// required flags, and any `x-cg-*` customProperties are mapped back.
/// `import_source = OdcsStripped`.
///
/// Validation constraints, PII transforms, glossary, and metrics not
/// recoverable from ODCS-native fields are silently omitted in Mode B — this
/// is the reason the caller must set `requires_review = true` (D-002) and
/// block promotion until human review clears it.
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

    // Detect Mode A vs Mode B.
    let has_cg_ext = mapping.contains_key("x-contractgate-version");

    if has_cg_ext {
        import_mode_a(mapping, version)
    } else {
        import_mode_b(mapping, version)
    }
}

// ── Mode A (lossless) ────────────────────────────────────────────────────────

fn import_mode_a(doc: &Mapping, version: String) -> Result<ImportResult, String> {
    let ontology = doc
        .get("x-contractgate-ontology")
        .ok_or_else(|| {
            "ODCS document has x-contractgate-version but is missing \
             x-contractgate-ontology"
                .to_string()
        })?
        .clone();

    // Re-serialize the ontology subtree to a YAML string.  This round-trips
    // through serde_yaml so key ordering may differ from the original, but
    // the resulting Contract is semantically identical.
    let yaml_content = serde_yaml::to_string(&ontology)
        .map_err(|e| format!("failed to serialize x-contractgate-ontology: {e}"))?;

    // Validate the recovered YAML is a parseable Contract.
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
    // Reconstruct a Contract from the ODCS schema section.
    let schema_seq = doc
        .get("schema")
        .and_then(|v| v.as_sequence())
        .ok_or_else(|| "ODCS document missing 'schema' array".to_string())?;

    let schema0 = schema_seq
        .first()
        .and_then(|v| v.as_mapping())
        .ok_or_else(|| "ODCS 'schema' array is empty or not a mapping".to_string())?;

    // Extract contract name from dataProduct (preferred) or schema[0].name.
    let name = doc
        .get("dataProduct")
        .and_then(|v| v.as_str())
        .or_else(|| schema0.get("name").and_then(|v| v.as_str()))
        .unwrap_or("imported_contract")
        .to_string();

    let description = doc
        .get("description")
        .and_then(|v| v.as_str())
        .or_else(|| schema0.get("description").and_then(|v| v.as_str()))
        .map(String::from);

    // Reconstruct field definitions and quality rules from schema[0].properties[].
    let (entities, quality) =
        if let Some(props) = schema0.get("properties").and_then(|v| v.as_sequence()) {
            let fields = unflatten_odcs_properties(props)?;
            // Collect quality rules from every property's quality[] array.
            let rules = extract_quality_rules_from_props(props);
            (fields, rules)
        } else {
            (vec![], vec![])
        };

    let contract = Contract {
        version: "1.0".to_string(),
        name,
        description,
        compliance_mode: false,
        egress_leakage_mode: EgressLeakageMode::Off,
        envelope: None,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality,
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| format!("failed to serialize reconstructed Contract: {e}"))?;

    Ok(ImportResult {
        version,
        yaml_content,
        import_source: ImportSource::OdcsStripped,
    })
}

/// Walk `schema[0].properties[]` and collect all quality rules across every
/// property entry.  Each property's `quality[]` array is parsed and the field
/// name (dot-notation) is stamped onto each rule.
fn extract_quality_rules_from_props(props: &[Value]) -> Vec<QualityRule> {
    let mut all_rules: Vec<QualityRule> = Vec::new();
    for p in props {
        let Some(m) = p.as_mapping() else { continue };
        let Some(name) = m.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(quality_seq) = m.get("quality").and_then(|v| v.as_sequence()) {
            all_rules.extend(odcs_quality_to_rules(name, quality_seq));
        }
    }
    all_rules
}

/// Reverse D-001 flattening: convert ODCS dot-notation property list back to a
/// nested CG field tree.
///
/// Strategy: collect all property entries, then group by the first dot-segment.
/// Entries without a dot are leaf fields.  Entries sharing a common prefix are
/// collected as children of an implicit Object parent.
fn unflatten_odcs_properties(props: &[Value]) -> Result<Vec<FieldDefinition>, String> {
    // Parse all entries into (name, FieldDefinition) pairs first.
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
    // Separate top-level (no dot) from nested (has dot).
    let mut top_level: Vec<FieldDefinition> = Vec::new();
    // Map: first_segment -> Vec<(remaining_path, FieldDefinition)>
    let mut nested: std::collections::BTreeMap<String, Vec<(String, FieldDefinition)>> =
        std::collections::BTreeMap::new();

    for (name, field) in pairs {
        if let Some(dot_pos) = name.find('.') {
            let parent = name[..dot_pos].to_string();
            let rest = name[dot_pos + 1..].to_string();
            // Re-name the field definition to just the leaf part (will be
            // re-set when building the parent object).
            let mut child = field;
            child.name = rest.clone();
            nested.entry(parent).or_default().push((rest, child));
        } else {
            top_level.push(field);
        }
    }

    // Build implicit Object nodes for each parent group.
    for (parent_name, children) in nested {
        let child_fields = build_field_tree(children)?;
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
        });
    }

    Ok(top_level)
}

/// Parse a single ODCS property mapping into a CG `FieldDefinition`.
/// Reads `logicalType`, `required`, and `x-cg-*` customProperties.
fn odcs_prop_to_field(m: &Mapping, full_name: &str) -> Result<FieldDefinition, String> {
    // Use the leaf segment as the field name; build_field_tree re-parents it.
    let leaf_name = full_name
        .rsplit('.')
        .next()
        .unwrap_or(full_name)
        .to_string();

    let logical = m
        .get("logicalType")
        .and_then(|v| v.as_str())
        .unwrap_or("string");
    let field_type = logical_to_field_type(logical);

    let required = m.get("required").and_then(|v| v.as_bool()).unwrap_or(false);

    // Parse x-cg-* customProperties back to CG constraints.
    let mut pattern: Option<String> = None;
    let mut allowed_values: Option<Vec<serde_json::Value>> = None;
    let mut min: Option<f64> = None;
    let mut max: Option<f64> = None;
    let mut min_length: Option<usize> = None;
    let mut max_length: Option<usize> = None;

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
                        allowed_values = Some(
                            seq.iter()
                                .filter_map(|v| serde_json::to_value(v).ok())
                                .collect(),
                        );
                    }
                }
                "x-cg-min" => {
                    min = val.and_then(|v| v.as_f64());
                }
                "x-cg-max" => {
                    max = val.and_then(|v| v.as_f64());
                }
                "x-cg-min-length" => {
                    min_length = val.and_then(|v| v.as_u64()).map(|n| n as usize);
                }
                "x-cg-max-length" => {
                    max_length = val.and_then(|v| v.as_u64()).map(|n| n as usize);
                }
                // Transform kind/style not reconstructed in Mode B —
                // transform secrets (salt) are absent; review required (D-002).
                _ => {}
            }
        }
    }

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
        properties: None,
        items: None,
        transform: None,
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
