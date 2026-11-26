//! Two-way converter between dbt model contracts (`schema.yml`) and
//! DataContractGate contracts (RFC-120).

use crate::contract::{
    Contract, FieldDefinition, FieldType, GlossaryEntry, Ontology, QualityRule, QualityRuleType,
    UniqueScope,
};
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, BTreeSet};

const META_KEY: &str = "contractgate";

/// Result of importing a dbt `schema.yml`.
#[derive(Debug, Clone)]
pub struct ImportDbtResult {
    pub contract: Contract,
    pub yaml_content: String,
    pub version: String,
    pub warnings: Vec<String>,
    pub lossless: bool,
}

/// One semantic difference between a contract and a dbt model contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub path: String,
    pub expected: String,
    pub actual: String,
}

/// Request body for `POST /contracts/import/dbt`.
#[derive(Debug, serde::Deserialize)]
pub struct ImportDbtRequest {
    pub dbt_yaml: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_override: Option<String>,
    #[serde(default)]
    pub target_contract_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub contract_id: Option<uuid::Uuid>,
    #[serde(default)]
    #[allow(dead_code)]
    pub org_id: Option<uuid::Uuid>,
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

fn strip_typmod(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    match s.find('(') {
        Some(i) => s[..i].trim().to_string(),
        None => s,
    }
}

/// Map a dbt / warehouse `data_type` to a DataContractGate field type.
pub fn field_type_from_dbt(data_type: &str) -> Option<FieldType> {
    let t = strip_typmod(data_type);
    let t = t.replace(" character set utf8", "");
    let t = t.trim();
    Some(match t {
        "varchar" | "character varying" | "character" | "char" | "text" | "string" | "nvarchar"
        | "nchar" | "clob" | "nclob" | "varchar2" | "nvarchar2" | "tinytext" | "mediumtext"
        | "longtext" | "stringtype" => FieldType::String,
        "integer" | "int" | "int2" | "int4" | "int8" | "bigint" | "smallint" | "tinyint"
        | "serial" | "bigserial" | "smallserial" => FieldType::Integer,
        "float" | "float4" | "float8" | "double" | "double precision" | "real" | "numeric"
        | "decimal" | "number" | "money" | "numeric64" => FieldType::Float,
        "boolean" | "bool" => FieldType::Boolean,
        "date" => FieldType::Date,
        "timestamp"
        | "timestamptz"
        | "timestamp_ntz"
        | "timestamp_ltz"
        | "timestamp_tz"
        | "timestamp without time zone"
        | "timestamp with time zone"
        | "timestamp with local time zone"
        | "datetime"
        | "datetime2"
        | "smalldatetime" => FieldType::Timestamp,
        "object" | "variant" | "json" | "jsonb" | "struct" | "map" | "super" => FieldType::Object,
        "array" => FieldType::Array,
        "any" | "unknown" | "anytype" => FieldType::Any,
        _ => return None,
    })
}

/// Portable dbt `data_type` written on export.
pub fn dbt_data_type(field_type: &FieldType) -> Option<&'static str> {
    match field_type {
        FieldType::String => Some("varchar"),
        FieldType::Integer => Some("integer"),
        FieldType::Float => Some("float"),
        FieldType::Boolean => Some("boolean"),
        FieldType::Date => Some("date"),
        FieldType::Timestamp => Some("timestamp"),
        FieldType::Object | FieldType::Array | FieldType::Any => None,
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export a DataContractGate contract as a dbt model-contract `schema.yml`.
pub fn export_dbt(
    contract: &Contract,
    model: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let model_name = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(contract.name.as_str());
    let mut warnings = Vec::new();
    let mut columns = Vec::new();
    let mut nested_stash: Vec<Value> = Vec::new();
    let mut glossary: Vec<GlossaryEntry> = contract.glossary.clone();
    let unique_fields: BTreeSet<String> = contract
        .quality
        .iter()
        .filter(|q| q.rule_type == QualityRuleType::Uniqueness)
        .map(|q| q.field.clone())
        .collect();

    for field in &contract.ontology.entities {
        match dbt_data_type(&field.field_type) {
            Some(data_type) => {
                columns.push(export_column(
                    field,
                    data_type,
                    unique_fields.contains(&field.name),
                ));
            }
            None => {
                warnings.push(format!(
                    "field '{}' has type {:?} and is not a dbt model-contract column; stored in meta.contractgate",
                    field.name, field.field_type
                ));
                if let Ok(v) = serde_yaml::to_value(field) {
                    nested_stash.push(v);
                }
            }
        }
    }

    let mut model_map = Mapping::new();
    model_map.insert(v_str("name"), v_str(model_name));
    if let Some(desc) = contract.description.as_ref().filter(|s| !s.is_empty()) {
        model_map.insert(v_str("description"), v_str(desc));
    }
    let mut cfg = Mapping::new();
    let mut contract_cfg = Mapping::new();
    contract_cfg.insert(v_str("enforced"), Value::Bool(true));
    cfg.insert(v_str("contract"), Value::Mapping(contract_cfg));
    model_map.insert(v_str("config"), Value::Mapping(cfg));

    if let Some(meta) = export_model_meta(contract, nested_stash, &mut glossary) {
        model_map.insert(v_str("meta"), meta);
    }
    model_map.insert(v_str("columns"), Value::Sequence(columns));

    let mut root = Mapping::new();
    root.insert(v_str("version"), Value::Number(2.into()));
    root.insert(
        v_str("models"),
        Value::Sequence(vec![Value::Mapping(model_map)]),
    );
    let body = serde_yaml::to_string(&Value::Mapping(root))
        .map_err(|e| format!("serializing dbt schema.yml: {e}"))?;
    let yaml = format!(
        "# Generated by `contractgate export dbt`.\n\
         # The meta.contractgate block holds rules dbt model contracts cannot express.\n\
         # Keep it if you want a lossless round-trip back to DataContractGate.\n\
         {body}"
    );
    Ok((yaml, warnings))
}

fn export_column(field: &FieldDefinition, data_type: &str, unique: bool) -> Value {
    let mut col = Mapping::new();
    col.insert(v_str("name"), v_str(&field.name));
    col.insert(v_str("data_type"), v_str(data_type));

    let mut constraints = Vec::new();
    if field.required {
        constraints.push(constraint_type("not_null"));
    }
    if unique {
        constraints.push(constraint_type("unique"));
    }
    if field.min.is_some() || field.max.is_some() {
        let expr = match (field.min, field.max) {
            (Some(lo), Some(hi)) => format!(
                "{} >= {} and {} <= {}",
                field.name,
                num(lo),
                field.name,
                num(hi)
            ),
            (Some(lo), None) => format!("{} >= {}", field.name, num(lo)),
            (None, Some(hi)) => format!("{} <= {}", field.name, num(hi)),
            (None, None) => unreachable!(),
        };
        let mut check = Mapping::new();
        check.insert(v_str("type"), v_str("check"));
        check.insert(v_str("expression"), v_str(&expr));
        constraints.push(Value::Mapping(check));
    }
    if !constraints.is_empty() {
        col.insert(v_str("constraints"), Value::Sequence(constraints));
    }
    if let Some(values) = field.allowed_values.as_ref().filter(|v| !v.is_empty()) {
        let seq: Vec<Value> = values.iter().filter_map(json_to_yaml).collect();
        let mut acc = Mapping::new();
        acc.insert(v_str("values"), Value::Sequence(seq));
        let mut test = Mapping::new();
        test.insert(v_str("accepted_values"), Value::Mapping(acc));
        col.insert(v_str("tests"), Value::Sequence(vec![Value::Mapping(test)]));
    }

    if let Some(meta) = export_column_meta(field) {
        col.insert(v_str("meta"), meta);
    }
    Value::Mapping(col)
}

fn export_column_meta(field: &FieldDefinition) -> Option<Value> {
    let mut inner = Mapping::new();
    if let Some(p) = &field.pattern {
        inner.insert(v_str("pattern"), v_str(p));
    }
    if let Some(n) = field.min_length {
        inner.insert(v_str("min_length"), Value::Number(n.into()));
    }
    if let Some(n) = field.max_length {
        inner.insert(v_str("max_length"), Value::Number(n.into()));
    }
    if let Some(f) = &field.format {
        inner.insert(v_str("format"), v_str(f));
    }
    if let Some(t) = &field.transform {
        if let Ok(v) = serde_yaml::to_value(t) {
            inner.insert(v_str("transform"), v);
        }
    }
    if inner.is_empty() {
        return None;
    }
    let mut meta = Mapping::new();
    meta.insert(v_str(META_KEY), Value::Mapping(inner));
    Some(Value::Mapping(meta))
}

fn export_model_meta(
    contract: &Contract,
    nested_stash: Vec<Value>,
    glossary: &mut Vec<GlossaryEntry>,
) -> Option<Value> {
    let mut inner = Mapping::new();
    inner.insert(v_str("version"), v_str(&contract.version));
    if contract.compliance_mode {
        inner.insert(v_str("compliance_mode"), Value::Bool(true));
    }
    if contract.null_as_absent {
        inner.insert(v_str("null_as_absent"), Value::Bool(true));
    }
    if contract.on_fail_locked {
        inner.insert(v_str("on_fail_locked"), Value::Bool(true));
    }
    let on_fail = serde_yaml::to_string(&contract.on_fail).unwrap_or_default();
    if !on_fail.trim().eq_ignore_ascii_case("quarantine") {
        if let Ok(v) = serde_yaml::to_value(contract.on_fail) {
            inner.insert(v_str("on_fail"), v);
        }
    }
    if let Some(env) = &contract.envelope {
        if let Ok(v) = serde_yaml::to_value(env) {
            inner.insert(v_str("envelope"), v);
        }
    }
    if let Ok(v) = serde_yaml::to_value(&contract.quarantine_guard) {
        if v.as_mapping().is_some_and(|m| !m.is_empty()) {
            inner.insert(v_str("quarantine_guard"), v);
        }
    }
    if !contract.egress_leakage_mode_is_off() {
        if let Ok(v) = serde_yaml::to_value(contract.egress_leakage_mode) {
            inner.insert(v_str("egress_leakage_mode"), v);
        }
    }
    let leftover_quality: Vec<&QualityRule> = contract
        .quality
        .iter()
        .filter(|q| q.rule_type != QualityRuleType::Uniqueness)
        .collect();
    if !leftover_quality.is_empty() {
        if let Ok(v) = serde_yaml::to_value(&leftover_quality) {
            inner.insert(v_str("quality"), v);
        }
    }
    if !contract.cross_field.is_empty() {
        if let Ok(v) = serde_yaml::to_value(&contract.cross_field) {
            inner.insert(v_str("cross_field"), v);
        }
    }
    if !contract.metrics.is_empty() {
        if let Ok(v) = serde_yaml::to_value(&contract.metrics) {
            inner.insert(v_str("metrics"), v);
        }
    }
    if !glossary.is_empty() {
        if let Ok(v) = serde_yaml::to_value(&*glossary) {
            inner.insert(v_str("glossary"), v);
        }
    }
    if !nested_stash.is_empty() {
        inner.insert(v_str("nested_entities"), Value::Sequence(nested_stash));
    }

    // Always keep version so import can restore contract.version.
    let mut meta = Mapping::new();
    meta.insert(v_str(META_KEY), Value::Mapping(inner));
    Some(Value::Mapping(meta))
}

// EgressLeakageMode may not have a helper; handle via serde.
trait EgressOff {
    fn egress_leakage_mode_is_off(&self) -> bool;
}
impl EgressOff for Contract {
    fn egress_leakage_mode_is_off(&self) -> bool {
        serde_yaml::to_string(&self.egress_leakage_mode)
            .map(|s| s.trim() == "off")
            .unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a dbt `schema.yml` into a DataContractGate contract.
pub fn import_dbt(yaml: &str, model: Option<&str>) -> Result<ImportDbtResult, String> {
    let doc: Value =
        serde_yaml::from_str(yaml).map_err(|e| format!("parsing dbt schema.yml: {e}"))?;
    let root = doc
        .as_mapping()
        .ok_or_else(|| "dbt schema.yml must be a mapping".to_string())?;
    let models = root
        .get(Value::String("models".into()))
        .and_then(Value::as_sequence)
        .ok_or_else(|| "dbt schema.yml has no models: list".to_string())?;
    if models.is_empty() {
        return Err("dbt schema.yml has an empty models list".into());
    }
    let chosen = pick_model(models, model)?;
    let model_map = chosen
        .as_mapping()
        .ok_or_else(|| "dbt model entry must be a mapping".to_string())?;
    let model_name =
        str_field(model_map, "name").ok_or_else(|| "dbt model is missing name".to_string())?;

    let mut warnings = Vec::new();
    let meta = contractgate_meta(model_map);
    let lossless = meta.is_some();

    let mut entities: Vec<FieldDefinition> = Vec::new();
    let mut quality: Vec<QualityRule> = Vec::new();
    let mut glossary: Vec<GlossaryEntry> = Vec::new();

    if let Some(seq) = model_map
        .get(Value::String("columns".into()))
        .and_then(Value::as_sequence)
    {
        for col in seq {
            let Some(col_map) = col.as_mapping() else {
                warnings.push("skipping a column that is not a mapping".into());
                continue;
            };
            match import_column(col_map, &mut quality, &mut glossary, &mut warnings) {
                Ok(field) => entities.push(field),
                Err(e) => warnings.push(e),
            }
        }
    }

    // Model-level constraints (primary_key, unique, check, foreign_key).
    if let Some(seq) = model_map
        .get(Value::String("constraints".into()))
        .and_then(Value::as_sequence)
    {
        apply_model_constraints(seq, &mut entities, &mut quality, &mut warnings);
    }

    if let Some(stash) = meta
        .as_ref()
        .and_then(|m| m.get(Value::String("nested_entities".into())))
        .and_then(Value::as_sequence)
    {
        for item in stash {
            match serde_yaml::from_value::<FieldDefinition>(item.clone()) {
                Ok(f) => entities.push(f),
                Err(e) => warnings.push(format!("could not restore a nested field: {e}")),
            }
        }
    }

    if let Some(meta) = meta.as_ref() {
        if let Some(q) = meta.get(Value::String("quality".into())) {
            match serde_yaml::from_value::<Vec<QualityRule>>(q.clone()) {
                Ok(extra) => quality.extend(extra),
                Err(e) => warnings.push(format!("could not restore quality rules: {e}")),
            }
        }
        if let Some(g) = meta.get(Value::String("glossary".into())) {
            if let Ok(extra) = serde_yaml::from_value::<Vec<GlossaryEntry>>(g.clone()) {
                // Prefer restored glossary; column descriptions already appended.
                for entry in extra {
                    if !glossary.iter().any(|e| e.field == entry.field) {
                        glossary.push(entry);
                    }
                }
            }
        }
    }

    let description = str_field(model_map, "description");
    let version = meta
        .as_ref()
        .and_then(|m| str_field(m, "version"))
        .unwrap_or_else(|| "1.0.0".into());

    let mut contract = Contract {
        version: version.clone(),
        name: model_name,
        description,
        compliance_mode: bool_field(meta.as_ref(), "compliance_mode"),
        ontology: Ontology { entities },
        glossary,
        metrics: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("metrics".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
        quality,
        egress_leakage_mode: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("egress_leakage_mode".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
        envelope: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("envelope".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
        null_as_absent: bool_field(meta.as_ref(), "null_as_absent"),
        on_fail: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("on_fail".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
        on_fail_locked: bool_field(meta.as_ref(), "on_fail_locked"),
        quarantine_guard: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("quarantine_guard".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
        cross_field: meta
            .as_ref()
            .and_then(|m| m.get(Value::String("cross_field".into())))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok())
            .unwrap_or_default(),
    };

    // Dedup uniqueness rules (model-level PK plus quality).
    dedup_uniqueness(&mut contract.quality);

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| format!("serializing DataContractGate contract: {e}"))?;
    Ok(ImportDbtResult {
        contract,
        yaml_content,
        version,
        warnings,
        lossless,
    })
}

fn pick_model<'a>(models: &'a [Value], wanted: Option<&str>) -> Result<&'a Value, String> {
    let names: Vec<String> = models
        .iter()
        .filter_map(|m| m.as_mapping().and_then(|mm| str_field(mm, "name")))
        .collect();
    match wanted.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => models
            .iter()
            .find(|m| {
                m.as_mapping()
                    .and_then(|mm| str_field(mm, "name"))
                    .as_deref()
                    == Some(name)
            })
            .ok_or_else(|| {
                format!(
                    "model '{name}' not found; available models: {}",
                    names.join(", ")
                )
            }),
        None if models.len() == 1 => Ok(&models[0]),
        None => Err(format!(
            "schema.yml has {} models ({}); pass --model to choose one",
            models.len(),
            names.join(", ")
        )),
    }
}

fn import_column(
    col: &Mapping,
    quality: &mut Vec<QualityRule>,
    glossary: &mut Vec<GlossaryEntry>,
    warnings: &mut Vec<String>,
) -> Result<FieldDefinition, String> {
    let name = str_field(col, "name").ok_or_else(|| "column is missing name".to_string())?;
    let data_type = str_field(col, "data_type");
    let field_type = match data_type.as_deref() {
        Some(dt) => match field_type_from_dbt(dt) {
            Some(t) => t,
            None => {
                warnings.push(format!(
                    "column '{name}' has unrecognized data_type '{dt}'; treating as string"
                ));
                FieldType::String
            }
        },
        None => {
            warnings.push(format!(
                "column '{name}' has no data_type; treating as string"
            ));
            FieldType::String
        }
    };

    let mut required = false;
    let mut unique = false;
    let mut min = None;
    let mut max = None;
    let mut allowed_values = None;

    if let Some(seq) = col
        .get(Value::String("constraints".into()))
        .and_then(Value::as_sequence)
    {
        for c in seq {
            let Some(cm) = c.as_mapping() else { continue };
            let typ = str_field(cm, "type").unwrap_or_default();
            match typ.as_str() {
                "not_null" => required = true,
                "unique" | "primary_key" => {
                    unique = true;
                    required = typ == "primary_key" || required;
                }
                "check" => {
                    if let Some(expr) = str_field(cm, "expression") {
                        if !apply_check_expr(&name, &expr, &mut min, &mut max) {
                            warnings.push(format!(
                                "column '{name}': could not map check expression `{expr}` to min/max"
                            ));
                        }
                    }
                }
                "foreign_key" => warnings.push(format!(
                    "column '{name}': foreign_key constraint skipped (no FK concept in the contract format)"
                )),
                other if !other.is_empty() => {
                    warnings.push(format!("column '{name}': unknown constraint type '{other}'"));
                }
                _ => {}
            }
        }
    }

    if let Some(tests) = col.get(Value::String("tests".into())) {
        parse_tests(
            &name,
            tests,
            &mut required,
            &mut unique,
            &mut allowed_values,
            &mut min,
            &mut max,
            warnings,
        );
    }

    if let Some(desc) = str_field(col, "description") {
        glossary.push(GlossaryEntry {
            field: name.clone(),
            description: desc,
            constraints: None,
            synonyms: None,
        });
    }

    let col_meta = contractgate_meta(col);
    let pattern = col_meta.as_ref().and_then(|m| str_field(m, "pattern"));
    let format = col_meta.as_ref().and_then(|m| str_field(m, "format"));
    let min_length = col_meta
        .as_ref()
        .and_then(|m| m.get(Value::String("min_length".into())))
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let max_length = col_meta
        .as_ref()
        .and_then(|m| m.get(Value::String("max_length".into())))
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let transform = col_meta
        .as_ref()
        .and_then(|m| m.get(Value::String("transform".into())))
        .and_then(|v| serde_yaml::from_value(v.clone()).ok());

    if unique {
        quality.push(QualityRule {
            field: name.clone(),
            rule_type: QualityRuleType::Uniqueness,
            description: None,
            max_age_seconds: None,
            scope: Some(UniqueScope::Batch),
            threshold: None,
        });
    }

    Ok(FieldDefinition {
        distribution: None,
        name,
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
        transform,
        format,
    })
}

fn apply_model_constraints(
    seq: &[Value],
    entities: &mut [FieldDefinition],
    quality: &mut Vec<QualityRule>,
    warnings: &mut Vec<String>,
) {
    for c in seq {
        let Some(cm) = c.as_mapping() else { continue };
        let typ = str_field(cm, "type").unwrap_or_default();
        let cols = constraint_columns(cm);
        match typ.as_str() {
            "not_null" => {
                for name in &cols {
                    if let Some(f) = entities.iter_mut().find(|f| f.name == *name) {
                        f.required = true;
                    }
                }
            }
            "unique" | "primary_key" => {
                for name in &cols {
                    if typ == "primary_key" {
                        if let Some(f) = entities.iter_mut().find(|f| f.name == *name) {
                            f.required = true;
                        }
                    }
                    if !quality
                        .iter()
                        .any(|q| q.field == *name && q.rule_type == QualityRuleType::Uniqueness)
                    {
                        quality.push(QualityRule {
                            field: name.clone(),
                            rule_type: QualityRuleType::Uniqueness,
                            description: None,
                            max_age_seconds: None,
                            scope: Some(UniqueScope::Batch),
                            threshold: None,
                        });
                    }
                }
                if cols.len() > 1 {
                    warnings.push(format!(
                        "{typ} on ({}) is a composite key; uniqueness is recorded per column",
                        cols.join(", ")
                    ));
                }
            }
            "check" => {
                if let Some(expr) = str_field(cm, "expression") {
                    let target = cols.first().cloned().unwrap_or_default();
                    let mut min = None;
                    let mut max = None;
                    let field_name = if target.is_empty() {
                        guess_check_column(&expr).unwrap_or_default()
                    } else {
                        target
                    };
                    if apply_check_expr(&field_name, &expr, &mut min, &mut max) {
                        if let Some(f) = entities.iter_mut().find(|f| f.name == field_name) {
                            if min.is_some() {
                                f.min = min;
                            }
                            if max.is_some() {
                                f.max = max;
                            }
                        }
                    } else {
                        warnings.push(format!(
                            "model check expression `{expr}` could not be mapped to min/max"
                        ));
                    }
                }
            }
            "foreign_key" => warnings.push(
                "foreign_key constraint skipped (no FK concept in the contract format)".into(),
            ),
            other if !other.is_empty() => {
                warnings.push(format!("unknown model constraint type '{other}'"));
            }
            _ => {}
        }
    }
}

fn constraint_columns(cm: &Mapping) -> Vec<String> {
    if let Some(seq) = cm
        .get(Value::String("columns".into()))
        .and_then(Value::as_sequence)
    {
        return seq
            .iter()
            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
            .collect();
    }
    str_field(cm, "column").into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn parse_tests(
    name: &str,
    tests: &Value,
    required: &mut bool,
    unique: &mut bool,
    allowed_values: &mut Option<Vec<serde_json::Value>>,
    min: &mut Option<f64>,
    max: &mut Option<f64>,
    warnings: &mut Vec<String>,
) {
    let seq: Vec<&Value> = match tests {
        Value::Sequence(s) => s.iter().collect(),
        other => vec![other],
    };
    for t in seq {
        if let Some(s) = t.as_str() {
            match s {
                "not_null" => *required = true,
                "unique" => *unique = true,
                "relationships" => warnings.push(format!(
                    "column '{name}': relationships test skipped (no FK concept in the contract format)"
                )),
                other => warnings.push(format!("column '{name}': unknown test '{other}'")),
            }
            continue;
        }
        let Some(m) = t.as_mapping() else { continue };
        if let Some(av) = m.get(Value::String("accepted_values".into())) {
            if let Some(vals) = accepted_values_of(av) {
                *allowed_values = Some(vals);
            }
            continue;
        }
        if m.get(Value::String("relationships".into())).is_some()
            || m.get(Value::String("dbt_utils.relationships".into()))
                .is_some()
        {
            warnings.push(format!(
                "column '{name}': relationships test skipped (no FK concept in the contract format)"
            ));
            continue;
        }
        if let Some(range) = m
            .get(Value::String("dbt_utils.accepted_range".into()))
            .or_else(|| m.get(Value::String("accepted_range".into())))
            .and_then(Value::as_mapping)
        {
            *min = range
                .get(Value::String("min_value".into()))
                .and_then(yaml_f64)
                .or(*min);
            *max = range
                .get(Value::String("max_value".into()))
                .and_then(yaml_f64)
                .or(*max);
            continue;
        }
        if m.get(Value::String("not_null".into())).is_some() {
            *required = true;
            continue;
        }
        if m.get(Value::String("unique".into())).is_some() {
            *unique = true;
            continue;
        }
        if let Some(key) = m.keys().next().and_then(Value::as_str) {
            warnings.push(format!("column '{name}': test '{key}' is not mapped"));
        }
    }
}

fn accepted_values_of(av: &Value) -> Option<Vec<serde_json::Value>> {
    let seq = if let Some(m) = av.as_mapping() {
        m.get(Value::String("values".into()))
            .and_then(Value::as_sequence)?
    } else {
        av.as_sequence()?
    };
    let vals: Vec<serde_json::Value> = seq.iter().filter_map(yaml_to_json).collect();
    if vals.is_empty() {
        None
    } else {
        Some(vals)
    }
}

fn apply_check_expr(field: &str, expr: &str, min: &mut Option<f64>, max: &mut Option<f64>) -> bool {
    let mut any = false;
    // Split on `and` so `{col} >= a and {col} <= b` works.
    for part in expr.split(" and ") {
        let part = part.trim();
        if let Some((col, op, n)) = parse_cmp(part) {
            if col != field && col.rsplit('.').next().unwrap_or(col.as_str()) != field {
                continue;
            }
            match op.as_str() {
                ">=" => {
                    *min = Some(n);
                    any = true;
                }
                ">" => {
                    *min = Some(n); // inclusive approximation, noted in docs
                    any = true;
                }
                "<=" => {
                    *max = Some(n);
                    any = true;
                }
                "<" => {
                    *max = Some(n);
                    any = true;
                }
                _ => {}
            }
        }
    }
    any
}

fn parse_cmp(expr: &str) -> Option<(String, String, f64)> {
    let expr = expr.trim().trim_matches('(').trim_matches(')');
    let ops = [">=", "<=", ">", "<", "="];
    for op in ops {
        if let Some(i) = expr.find(op) {
            let left = expr[..i].trim();
            let right = expr[i + op.len()..].trim();
            if let Ok(n) = right.parse::<f64>() {
                return Some((left.to_string(), op.to_string(), n));
            }
        }
    }
    None
}

fn guess_check_column(expr: &str) -> Option<String> {
    parse_cmp(expr).map(|(c, _, _)| c)
}

// ---------------------------------------------------------------------------
// Check / drift
// ---------------------------------------------------------------------------

/// Compare a DataContractGate contract to a dbt `schema.yml`.
///
/// The dbt file is imported (including `meta.contractgate` when present)
/// and compared to `expected` on name, fields, types, required, enums,
/// uniqueness, min/max, lengths, pattern, and format.
pub fn check_against(
    expected: &Contract,
    dbt_yaml: &str,
    model: Option<&str>,
) -> Result<(Vec<Drift>, Vec<String>), String> {
    let imported = import_dbt(dbt_yaml, model)?;
    let mut drifts = compare_contracts(expected, &imported.contract);
    // If the caller passed a model name, the contract name may differ; that
    // is not drift when --model was used to select among several models.
    if model.is_some() {
        drifts.retain(|d| d.path != "name");
    }
    Ok((drifts, imported.warnings))
}

fn compare_contracts(expected: &Contract, actual: &Contract) -> Vec<Drift> {
    let mut drifts = Vec::new();
    if expected.name != actual.name {
        drifts.push(drift("name", &expected.name, &actual.name));
    }
    let exp_map: BTreeMap<&str, &FieldDefinition> = expected
        .ontology
        .entities
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let act_map: BTreeMap<&str, &FieldDefinition> = actual
        .ontology
        .entities
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    for name in exp_map.keys() {
        if !act_map.contains_key(name) {
            drifts.push(drift(&format!("field.{name}"), "present", "missing"));
        }
    }
    for name in act_map.keys() {
        if !exp_map.contains_key(name) {
            drifts.push(drift(&format!("field.{name}"), "absent", "present"));
        }
    }
    for (name, exp) in &exp_map {
        let Some(act) = act_map.get(name) else {
            continue;
        };
        if exp.field_type != act.field_type {
            drifts.push(drift(
                &format!("field.{name}.type"),
                &format!("{:?}", exp.field_type).to_ascii_lowercase(),
                &format!("{:?}", act.field_type).to_ascii_lowercase(),
            ));
        }
        if exp.required != act.required {
            drifts.push(drift(
                &format!("field.{name}.required"),
                &exp.required.to_string(),
                &act.required.to_string(),
            ));
        }
        if norm_enum(exp.allowed_values.as_ref()) != norm_enum(act.allowed_values.as_ref()) {
            drifts.push(drift(
                &format!("field.{name}.enum"),
                &fmt_enum(exp.allowed_values.as_ref()),
                &fmt_enum(act.allowed_values.as_ref()),
            ));
        }
        if exp.min != act.min {
            drifts.push(drift(
                &format!("field.{name}.min"),
                &fmt_opt_f64(exp.min),
                &fmt_opt_f64(act.min),
            ));
        }
        if exp.max != act.max {
            drifts.push(drift(
                &format!("field.{name}.max"),
                &fmt_opt_f64(exp.max),
                &fmt_opt_f64(act.max),
            ));
        }
        if exp.min_length != act.min_length {
            drifts.push(drift(
                &format!("field.{name}.min_length"),
                &fmt_opt_usize(exp.min_length),
                &fmt_opt_usize(act.min_length),
            ));
        }
        if exp.max_length != act.max_length {
            drifts.push(drift(
                &format!("field.{name}.max_length"),
                &fmt_opt_usize(exp.max_length),
                &fmt_opt_usize(act.max_length),
            ));
        }
        if exp.pattern != act.pattern {
            drifts.push(drift(
                &format!("field.{name}.pattern"),
                exp.pattern.as_deref().unwrap_or("<none>"),
                act.pattern.as_deref().unwrap_or("<none>"),
            ));
        }
        if exp.format != act.format {
            drifts.push(drift(
                &format!("field.{name}.format"),
                exp.format.as_deref().unwrap_or("<none>"),
                act.format.as_deref().unwrap_or("<none>"),
            ));
        }
    }
    let exp_u = unique_set(expected);
    let act_u = unique_set(actual);
    for name in exp_u.difference(&act_u) {
        drifts.push(drift(&format!("field.{name}.unique"), "true", "false"));
    }
    for name in act_u.difference(&exp_u) {
        drifts.push(drift(&format!("field.{name}.unique"), "false", "true"));
    }
    drifts.sort_by(|a, b| a.path.cmp(&b.path));
    drifts
}

fn unique_set(c: &Contract) -> BTreeSet<String> {
    c.quality
        .iter()
        .filter(|q| q.rule_type == QualityRuleType::Uniqueness)
        .map(|q| q.field.clone())
        .collect()
}

fn drift(path: &str, expected: &str, actual: &str) -> Drift {
    Drift {
        path: path.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    }
}

fn norm_enum(v: Option<&Vec<serde_json::Value>>) -> BTreeSet<String> {
    v.map(|vals| vals.iter().map(|x| x.to_string()).collect())
        .unwrap_or_default()
}

fn fmt_enum(v: Option<&Vec<serde_json::Value>>) -> String {
    let mut items: Vec<String> = norm_enum(v).into_iter().collect();
    items.sort();
    if items.is_empty() {
        "<none>".into()
    } else {
        items.join(",")
    }
}

fn fmt_opt_f64(v: Option<f64>) -> String {
    v.map(num).unwrap_or_else(|| "<none>".into())
}

fn fmt_opt_usize(v: Option<usize>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "<none>".into())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn v_str(s: &str) -> Value {
    Value::String(s.to_string())
}

fn num(n: f64) -> String {
    if (n - n.round()).abs() < f64::EPSILON && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn constraint_type(typ: &str) -> Value {
    let mut m = Mapping::new();
    m.insert(v_str("type"), v_str(typ));
    Value::Mapping(m)
}

fn json_to_yaml(v: &serde_json::Value) -> Option<Value> {
    serde_yaml::to_value(v).ok()
}

fn yaml_to_json(v: &Value) -> Option<serde_json::Value> {
    serde_json::to_value(v).ok()
}

fn yaml_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().or_else(|| n.as_i64().map(|i| i as f64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn str_field(m: &Mapping, key: &str) -> Option<String> {
    m.get(Value::String(key.into()))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn bool_field(m: Option<&Mapping>, key: &str) -> bool {
    m.and_then(|mm| mm.get(Value::String(key.into())))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn contractgate_meta(m: &Mapping) -> Option<Mapping> {
    m.get(Value::String("meta".into()))
        .and_then(Value::as_mapping)
        .and_then(|meta| meta.get(Value::String(META_KEY.into())))
        .and_then(Value::as_mapping)
        .cloned()
}

fn dedup_uniqueness(quality: &mut Vec<QualityRule>) {
    let mut seen = BTreeSet::new();
    quality.retain(|q| {
        if q.rule_type != QualityRuleType::Uniqueness {
            return true;
        }
        seen.insert(q.field.clone())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_orders() -> Contract {
        let yaml = include_str!("../tests/fixtures/dbt_yaml/orders.yaml");
        serde_yaml::from_str(yaml).expect("orders fixture")
    }

    #[test]
    fn typemod_and_aliases_map() {
        assert_eq!(field_type_from_dbt("VARCHAR(256)"), Some(FieldType::String));
        assert_eq!(
            field_type_from_dbt("character varying(20)"),
            Some(FieldType::String)
        );
        assert_eq!(field_type_from_dbt("BIGINT"), Some(FieldType::Integer));
        assert_eq!(field_type_from_dbt("numeric(10,2)"), Some(FieldType::Float));
        assert_eq!(
            field_type_from_dbt("timestamptz"),
            Some(FieldType::Timestamp)
        );
        assert_eq!(field_type_from_dbt("not-a-type"), None);
    }

    #[test]
    fn orders_round_trip_is_lossless_on_compared_fields() {
        let original = load_orders();
        let (yml, warnings) = export_dbt(&original, None).expect("export");
        assert!(yml.contains("config:"));
        assert!(yml.contains("enforced: true"));
        assert!(yml.contains("meta:"));
        assert!(yml.contains("contractgate"));
        // warnings only for things we stashed
        let imported = import_dbt(&yml, Some("orders")).expect("import");
        assert!(imported.lossless, "export writes meta.contractgate");
        let drifts = compare_contracts(&original, &imported.contract);
        assert!(
            drifts.is_empty(),
            "round-trip drifts: {drifts:?}; export warnings: {warnings:?}; import warnings: {:?}",
            imported.warnings
        );
        assert_eq!(imported.contract.version, "2.0.0");
        assert!(imported
            .contract
            .quality
            .iter()
            .any(|q| q.field == "created_at" && q.rule_type == QualityRuleType::Freshness));
        assert!(!imported.contract.cross_field.is_empty());
        let discount = imported
            .contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "discount_code")
            .unwrap();
        assert_eq!(discount.min_length, Some(4));
        assert_eq!(discount.max_length, Some(20));
        assert!(!discount.required);
        let order_id = imported
            .contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "order_id")
            .unwrap();
        assert_eq!(
            order_id.pattern.as_deref(),
            Some("^ord_[A-Za-z0-9]{10,32}$")
        );
        assert!(imported
            .contract
            .quality
            .iter()
            .any(|q| q.field == "order_id" && q.rule_type == QualityRuleType::Uniqueness));
    }

    #[test]
    fn native_dbt_import_is_best_effort() {
        let yaml = include_str!("../tests/fixtures/dbt_yaml/native_orders.yml");
        let imported = import_dbt(yaml, None).expect("import native");
        assert!(!imported.lossless);
        assert_eq!(imported.contract.name, "orders");
        assert_eq!(imported.contract.version, "1.0.0");
        let by_name: BTreeMap<_, _> = imported
            .contract
            .ontology
            .entities
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();
        assert!(by_name["order_id"].required);
        assert_eq!(by_name["order_id"].field_type, FieldType::String);
        assert_eq!(by_name["amount_usd"].min, Some(0.0));
        assert_eq!(by_name["amount_usd"].field_type, FieldType::Float);
        assert_eq!(
            by_name["status"].allowed_values.as_ref().map(|v| v.len()),
            Some(5)
        );
        assert!(!by_name["discount_code"].required);
        assert!(imported
            .contract
            .quality
            .iter()
            .any(|q| q.field == "order_id" && q.rule_type == QualityRuleType::Uniqueness));
        assert!(imported
            .warnings
            .iter()
            .any(|w| w.contains("relationships")));
        // typemod on status was varchar(16); still string
        assert_eq!(by_name["status"].field_type, FieldType::String);
        assert_eq!(by_name["created_at"].field_type, FieldType::Timestamp);
    }

    #[test]
    fn check_against_detects_min_drift() {
        let original = load_orders();
        let (mut yml, _) = export_dbt(&original, None).unwrap();
        // Break the exported check expression for amount_usd.
        yml = yml.replace("amount_usd >= 0", "amount_usd >= 10");
        let (drifts, _) = check_against(&original, &yml, Some("orders")).unwrap();
        assert!(
            drifts.iter().any(|d| d.path == "field.amount_usd.min"),
            "drifts: {drifts:?}"
        );
    }

    #[test]
    fn missing_model_is_an_error() {
        let yaml = include_str!("../tests/fixtures/dbt_yaml/native_orders.yml");
        let err = import_dbt(yaml, Some("payments")).unwrap_err();
        assert!(err.contains("payments"), "{err}");
    }

    #[test]
    fn parse_compound_check() {
        let mut min = None;
        let mut max = None;
        assert!(apply_check_expr(
            "amount_usd",
            "amount_usd >= 0 and amount_usd <= 99.5",
            &mut min,
            &mut max
        ));
        assert_eq!(min, Some(0.0));
        assert_eq!(max, Some(99.5));
    }
}
