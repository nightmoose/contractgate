//! Downstream impact analysis.
//!
//! Lineage is an org-scoped edge list in Postgres. An edge points from an
//! upstream asset to a downstream one. When a contract version is proposed
//! or published, a recursive walk finds the assets that depend on it and a
//! deterministic classifier labels each field change breaking or safe.
//!
//! Explanations are templates. `CONTRACTGATE_IMPACT_LLM_URL`, when set, may
//! rewrite the one-line summary. Asset explanations stay on the templates.
//! The variable is optional and unset by default. No secret is required.

#[cfg(test)]
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::contract::{Contract, FieldDefinition, FieldType};
use crate::error::{AppError, AppResult};
use crate::plan::{
    impact_analysis_allowed, impact_asset_limit, impact_edge_limit, impact_quota_message,
    impact_upgrade_message, IMPACT_LLM_URL_ENV, IMPACT_MAX_HOPS,
};
use crate::AppState;

const EDGE_KINDS_WALK: &[&str] = &["feeds", "displays", "refreshes"];
const SCHEMA_MISSING: &str = "Downstream impact analysis is not available until migration 054_lineage_impact.sql is applied.";

// ---------------------------------------------------------------------------
// Field-change classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub field: String,
    pub kind: String,
    pub severity: String,
    pub detail: String,
}

impl FieldChange {
    fn breaking(field: impl Into<String>, kind: &str, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: kind.to_string(),
            severity: "breaking".into(),
            detail: detail.into(),
        }
    }

    fn safe(field: impl Into<String>, kind: &str, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            kind: kind.to_string(),
            severity: "safe".into(),
            detail: detail.into(),
        }
    }

    pub fn sentence(&self) -> String {
        match self.kind.as_str() {
            "field_removed" => format!("Field `{}` was removed.", self.field),
            "field_renamed" => format!("{}.", self.detail.trim_end_matches('.')),
            "type_narrowed" => {
                format!("Field `{}` was narrowed from {}.", self.field, self.detail)
            }
            "type_incompatible" => {
                format!("Field `{}` changed type from {}.", self.field, self.detail)
            }
            "type_widened" => {
                format!("Field `{}` was widened from {}.", self.field, self.detail)
            }
            "required_added" => format!("Field `{}` is now required.", self.field),
            "required_relaxed" => format!("Field `{}` is no longer required.", self.field),
            "field_added_optional" => format!("Optional field `{}` was added.", self.field),
            "enum_value_removed" => {
                format!("Field `{}` no longer allows {}.", self.field, self.detail)
            }
            "enum_value_added" => format!("Field `{}` now allows {}.", self.field, self.detail),
            "constraint_narrowed" => format!(
                "Field `{}` has a tighter constraint ({}).",
                self.field, self.detail
            ),
            "constraint_widened" => format!(
                "Field `{}` has a looser constraint ({}).",
                self.field, self.detail
            ),
            _ => format!("Field `{}` changed ({}).", self.field, self.detail),
        }
    }
}

/// Classify field changes between two contract YAML documents.
/// `before` is `None` for the first version of a contract.
pub fn classify_yaml(before: Option<&str>, after: &str) -> Result<Vec<FieldChange>, String> {
    let after_contract = parse_contract(after)?;
    let before_contract = match before {
        Some(yaml) => Some(parse_contract(yaml)?),
        None => None,
    };
    let renames = collect_renames(after);
    Ok(classify_contracts(
        before_contract.as_ref(),
        &after_contract,
        &renames,
    ))
}

fn parse_contract(yaml: &str) -> Result<Contract, String> {
    serde_yaml::from_str(yaml).map_err(|e| format!("invalid contract YAML: {e}"))
}

fn collect_renames(yaml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return out;
    };
    let Some(entities) = value
        .get("ontology")
        .and_then(|o| o.get("entities"))
        .and_then(|e| e.as_sequence())
    else {
        return out;
    };
    walk_rename_fields(entities, "", &mut out);
    out
}

fn walk_rename_fields(
    fields: &[serde_yaml::Value],
    prefix: &str,
    out: &mut HashMap<String, String>,
) {
    for item in fields {
        let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let path = field_path(prefix, name);
        if let Some(old) = item.get("renamed_from").and_then(|v| v.as_str()) {
            if !old.is_empty() {
                out.insert(path.clone(), field_path(prefix, old));
            }
        }
        if let Some(props) = item.get("properties").and_then(|p| p.as_sequence()) {
            walk_rename_fields(props, &path, out);
        }
    }
}

fn classify_contracts(
    before: Option<&Contract>,
    after: &Contract,
    renames: &HashMap<String, String>,
) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    let empty = Vec::new();
    let before_fields = before
        .map(|c| c.ontology.entities.as_slice())
        .unwrap_or(&empty);
    diff_fields(
        before_fields,
        &after.ontology.entities,
        "",
        renames,
        &mut changes,
    );
    changes.sort_by(|a, b| (&a.field, &a.kind).cmp(&(&b.field, &b.kind)));
    changes
}

fn diff_fields(
    before: &[FieldDefinition],
    after: &[FieldDefinition],
    prefix: &str,
    renames: &HashMap<String, String>,
    out: &mut Vec<FieldChange>,
) {
    let a_map: HashMap<&str, &FieldDefinition> =
        before.iter().map(|f| (f.name.as_str(), f)).collect();
    let b_map: HashMap<&str, &FieldDefinition> =
        after.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut consumed_old: HashSet<&str> = HashSet::new();
    let mut consumed_new: HashSet<&str> = HashSet::new();

    for (new_name, new_field) in &b_map {
        let new_path = field_path(prefix, new_name);
        let Some(old_path) = renames.get(&new_path) else {
            continue;
        };
        let Some(old_name) = old_path.rsplit('.').next() else {
            continue;
        };
        if b_map.contains_key(old_name) {
            continue;
        }
        let Some(old_field) = a_map.get(old_name) else {
            continue;
        };
        out.push(FieldChange::breaking(
            new_path.clone(),
            "field_renamed",
            format!("`{old_name}` was renamed to `{new_name}`"),
        ));
        diff_one(old_field, new_field, &new_path, renames, out);
        consumed_old.insert(old_name);
        consumed_new.insert(new_name);
    }

    for (name, field) in &a_map {
        if consumed_old.contains(name) || b_map.contains_key(name) {
            continue;
        }
        out.push(FieldChange::breaking(
            field_path(prefix, name),
            "field_removed",
            format!("{:?}, {}", field.field_type, required_word(field.required)),
        ));
    }

    for (name, field) in &b_map {
        if consumed_new.contains(name) || a_map.contains_key(name) {
            continue;
        }
        let path = field_path(prefix, name);
        if field.required {
            out.push(FieldChange::breaking(
                path,
                "required_added",
                "new required field".to_string(),
            ));
        } else {
            out.push(FieldChange::safe(
                path,
                "field_added_optional",
                type_name(&field.field_type).to_string(),
            ));
        }
    }

    for (name, new_field) in &b_map {
        if consumed_new.contains(name) {
            continue;
        }
        if let Some(old_field) = a_map.get(name) {
            diff_one(
                old_field,
                new_field,
                &field_path(prefix, name),
                renames,
                out,
            );
        }
    }
}

fn diff_one(
    before: &FieldDefinition,
    after: &FieldDefinition,
    path: &str,
    renames: &HashMap<String, String>,
    out: &mut Vec<FieldChange>,
) {
    if let Some((kind, breaking)) = type_effect(&before.field_type, &after.field_type) {
        let detail = format!(
            "{} → {}",
            type_name(&before.field_type),
            type_name(&after.field_type)
        );
        out.push(if breaking {
            FieldChange::breaking(path, kind, detail)
        } else {
            FieldChange::safe(path, kind, detail)
        });
    }

    if before.required != after.required {
        if after.required {
            out.push(FieldChange::breaking(
                path,
                "required_added",
                "optional → required".to_string(),
            ));
        } else {
            out.push(FieldChange::safe(
                path,
                "required_relaxed",
                "required → optional".to_string(),
            ));
        }
    }

    match (&before.pattern, &after.pattern) {
        (a, b) if a == b => {}
        (None, Some(p)) => out.push(FieldChange::breaking(
            path,
            "constraint_narrowed",
            format!("pattern added: {p}"),
        )),
        (Some(p), None) => out.push(FieldChange::safe(
            path,
            "constraint_widened",
            format!("pattern removed: {p}"),
        )),
        (Some(a), Some(b)) => out.push(FieldChange::breaking(
            path,
            "constraint_narrowed",
            format!("pattern {a} → {b}"),
        )),
        (None, None) => {}
    }

    push_bound(path, "min", before.min, after.min, true, out);
    push_bound(path, "max", before.max, after.max, false, out);
    push_len(
        path,
        "min_length",
        before.min_length,
        after.min_length,
        true,
        out,
    );
    push_len(
        path,
        "max_length",
        before.max_length,
        after.max_length,
        false,
        out,
    );
    diff_enums(&before.allowed_values, &after.allowed_values, path, out);

    if before.field_type == FieldType::Object && after.field_type == FieldType::Object {
        if let (Some(a_props), Some(b_props)) = (&before.properties, &after.properties) {
            diff_fields(a_props, b_props, path, renames, out);
        }
    }
}

fn type_effect(before: &FieldType, after: &FieldType) -> Option<(&'static str, bool)> {
    if before == after {
        return None;
    }
    Some(match (before, after) {
        (FieldType::Integer, FieldType::Float) => ("type_widened", false),
        (_, FieldType::Any) => ("type_widened", false),
        (FieldType::Float, FieldType::Integer) | (FieldType::Any, _) => ("type_narrowed", true),
        _ => ("type_incompatible", true),
    })
}

fn push_bound(
    path: &str,
    label: &str,
    before: Option<f64>,
    after: Option<f64>,
    higher_is_tighter: bool,
    out: &mut Vec<FieldChange>,
) {
    let effect = bound_effect(before, after, higher_is_tighter);
    push_effect(
        path,
        label,
        effect,
        bound_text(before),
        bound_text(after),
        out,
    );
}

fn push_len(
    path: &str,
    label: &str,
    before: Option<usize>,
    after: Option<usize>,
    higher_is_tighter: bool,
    out: &mut Vec<FieldChange>,
) {
    let effect = bound_effect(
        before.map(|n| n as f64),
        after.map(|n| n as f64),
        higher_is_tighter,
    );
    push_effect(path, label, effect, len_text(before), len_text(after), out);
}

fn push_effect(
    path: &str,
    label: &str,
    effect: Option<bool>,
    before: String,
    after: String,
    out: &mut Vec<FieldChange>,
) {
    let Some(breaking) = effect else {
        return;
    };
    let detail = format!("{label}: {before} → {after}");
    out.push(if breaking {
        FieldChange::breaking(path, "constraint_narrowed", detail)
    } else {
        FieldChange::safe(path, "constraint_widened", detail)
    });
}

/// `Some(true)` is tighter (breaking), `Some(false)` is looser (safe).
fn bound_effect(before: Option<f64>, after: Option<f64>, higher_is_tighter: bool) -> Option<bool> {
    match (before, after) {
        (None, None) => None,
        (Some(a), Some(b)) if (a - b).abs() < f64::EPSILON => None,
        (None, Some(_)) => Some(true),
        (Some(_), None) => Some(false),
        (Some(a), Some(b)) => Some(if higher_is_tighter { b > a } else { b < a }),
    }
}

fn diff_enums(
    before: &Option<Vec<Value>>,
    after: &Option<Vec<Value>>,
    path: &str,
    out: &mut Vec<FieldChange>,
) {
    match (before, after) {
        (None, None) => {}
        (None, Some(_)) => out.push(FieldChange::breaking(
            path,
            "constraint_narrowed",
            "enum constraint added".to_string(),
        )),
        (Some(_), None) => out.push(FieldChange::safe(
            path,
            "constraint_widened",
            "enum constraint removed".to_string(),
        )),
        (Some(a), Some(b)) => {
            let a_set: HashSet<String> = a.iter().map(enum_key).collect();
            let b_set: HashSet<String> = b.iter().map(enum_key).collect();
            let mut removed: Vec<_> = a_set.difference(&b_set).cloned().collect();
            let mut added: Vec<_> = b_set.difference(&a_set).cloned().collect();
            removed.sort();
            added.sort();
            for value in removed {
                out.push(FieldChange::breaking(path, "enum_value_removed", value));
            }
            for value in added {
                out.push(FieldChange::safe(path, "enum_value_added", value));
            }
        }
    }
}

fn enum_key(value: &Value) -> String {
    value
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| value.to_string())
}

fn field_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn top_field(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

fn type_name(field_type: &FieldType) -> &'static str {
    match field_type {
        FieldType::String => "string",
        FieldType::Integer => "integer",
        FieldType::Float => "float",
        FieldType::Boolean => "boolean",
        FieldType::Object => "object",
        FieldType::Array => "array",
        FieldType::Any => "any",
        FieldType::Date => "date",
        FieldType::Timestamp => "timestamp",
    }
}

fn required_word(required: bool) -> &'static str {
    if required {
        "required"
    } else {
        "optional"
    }
}

fn bound_text(value: Option<f64>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn len_text(value: Option<usize>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "none".to_string())
}

pub fn changed_top_fields(changes: &[FieldChange]) -> Vec<String> {
    let mut names: Vec<String> = changes
        .iter()
        .map(|c| top_field(&c.field).to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn changes_for<'a>(changes: &'a [FieldChange], path_fields: &[String]) -> Vec<&'a FieldChange> {
    if path_fields.iter().any(|field| field.is_empty()) {
        return changes.iter().collect();
    }
    let labels: HashSet<&str> = path_fields.iter().map(String::as_str).collect();
    changes
        .iter()
        .filter(|change| labels.contains(top_field(&change.field)))
        .collect()
}

// ---------------------------------------------------------------------------
// In-memory walk. The Postgres recursive query uses the same edge rule.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEdge {
    pub from: String,
    pub to: String,
    pub field: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reached {
    pub key: String,
    pub depth: i32,
}

/// Shortest downstream path under the impact edge rule.
///
/// An unscoped edge carries every field change. A field-scoped edge carries
/// only that field. `include_all` is used when the contract text did not
/// change, so the walk is the full blast radius.
#[cfg(test)]
pub fn reference_walk(
    edges: &[WalkEdge],
    starts: &[String],
    changed_fields: &[String],
    include_all: bool,
    max_hops: i32,
) -> Vec<Reached> {
    let mut adjacency: HashMap<&str, Vec<&WalkEdge>> = HashMap::new();
    for edge in edges {
        adjacency.entry(edge.from.as_str()).or_default().push(edge);
    }
    let changed: HashSet<&str> = changed_fields.iter().map(String::as_str).collect();
    let mut queue = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    for start in starts {
        if visited.insert(start.clone()) {
            queue.push_back((start.clone(), 0));
        }
    }
    let mut found = Vec::new();
    while let Some((key, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        let Some(outs) = adjacency.get(key.as_str()) else {
            continue;
        };
        for edge in outs {
            if !edge_allowed(edge.field.as_deref(), include_all, &changed) {
                continue;
            }
            if !visited.insert(edge.to.clone()) {
                continue;
            }
            let next = depth + 1;
            found.push(Reached {
                key: edge.to.clone(),
                depth: next,
            });
            queue.push_back((edge.to.clone(), next));
        }
    }
    found.sort_by(|a, b| (&a.key, a.depth).cmp(&(&b.key, b.depth)));
    found
}

#[cfg(test)]
fn edge_allowed(field: Option<&str>, include_all: bool, changed: &HashSet<&str>) -> bool {
    if include_all {
        return true;
    }
    match field {
        None => true,
        Some(name) => changed.contains(name),
    }
}

// ---------------------------------------------------------------------------
// Lineage documents: dbt manifest.json and an edge list (YAML or JSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAsset {
    external_key: String,
    kind: String,
    name: String,
    owner: Option<String>,
    contract_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEdge {
    from_key: String,
    to_key: String,
    edge_kind: String,
    field_name: Option<String>,
}

struct ParsedLineage {
    format: String,
    assets: Vec<ParsedAsset>,
    edges: Vec<ParsedEdge>,
}

fn parse_lineage_document(text: &str, format: &str) -> Result<ParsedLineage, String> {
    let detected = if format.is_empty() || format == "auto" {
        detect_format(text)
    } else {
        format.to_string()
    };
    match detected.as_str() {
        "dbt_manifest" => {
            let value = serde_json::from_str::<Value>(text)
                .map_err(|e| format!("dbt manifest is not JSON: {e}"))?;
            parse_dbt(&value)
        }
        "edges" => {
            let value = parse_json_or_yaml(text)?;
            parse_edges(&value)
        }
        other => Err(format!(
            "unknown lineage format '{other}'. Use edges or dbt_manifest."
        )),
    }
}

fn detect_format(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if value.get("nodes").is_some()
            && (value.get("metadata").is_some()
                || value.get("sources").is_some()
                || value.get("dbt_schema_version").is_some())
        {
            return "dbt_manifest".to_string();
        }
        if value.get("edges").is_some() {
            return "edges".to_string();
        }
    }
    "edges".to_string()
}

fn parse_json_or_yaml(text: &str) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Ok(value);
    }
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| format!("edge list is not JSON or YAML: {e}"))?;
    serde_json::to_value(yaml).map_err(|e| format!("edge list could not be read: {e}"))
}

fn parse_edges(value: &Value) -> Result<ParsedLineage, String> {
    let edges = value
        .get("edges")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "edge list needs an `edges` array".to_string())?;
    if edges.is_empty() {
        return Err("lineage document has no edges".into());
    }
    let mut assets: HashMap<String, ParsedAsset> = HashMap::new();
    let mut parsed = Vec::new();
    let mut seen = HashSet::new();
    for edge in edges {
        let from = endpoint(edge, "upstream", "from")?;
        let to = endpoint(edge, "downstream", "to")?;
        let edge_kind = edge
            .get("edge")
            .or_else(|| edge.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("feeds");
        validate_edge_kind(edge_kind)?;
        if from.key == to.key {
            return Err(format!("lineage edge {} points at itself", from.key));
        }
        let field_name = edge
            .get("field")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let marker = (
            from.key.clone(),
            to.key.clone(),
            edge_kind.to_string(),
            field_name.clone().unwrap_or_default(),
        );
        if !seen.insert(marker) {
            continue;
        }
        let from_key = from.key.clone();
        let to_key = to.key.clone();
        remember_asset(&mut assets, from);
        remember_asset(&mut assets, to);
        parsed.push(ParsedEdge {
            from_key,
            to_key,
            edge_kind: edge_kind.to_string(),
            field_name,
        });
    }
    for edge in &parsed {
        if edge.edge_kind != "owned_by" {
            continue;
        }
        let owner_name = assets.get(&edge.to_key).map(|asset| asset.name.clone());
        if let (Some(name), Some(asset)) = (owner_name, assets.get_mut(&edge.from_key)) {
            if asset.owner.is_none() {
                asset.owner = Some(name);
            }
        }
    }
    Ok(ParsedLineage {
        format: "edges".into(),
        assets: assets.into_values().collect(),
        edges: parsed,
    })
}

struct Endpoint {
    key: String,
    kind: String,
    name: String,
    owner: Option<String>,
    contract_ref: Option<String>,
}

fn remember_asset(assets: &mut HashMap<String, ParsedAsset>, endpoint: Endpoint) {
    assets
        .entry(endpoint.key.clone())
        .and_modify(|existing| {
            if existing.owner.is_none() {
                existing.owner = endpoint.owner.clone();
            }
            if existing.contract_ref.is_none() {
                existing.contract_ref = endpoint.contract_ref.clone();
            }
        })
        .or_insert(ParsedAsset {
            external_key: endpoint.key,
            kind: endpoint.kind,
            name: endpoint.name,
            owner: endpoint.owner,
            contract_ref: endpoint.contract_ref,
        });
}

fn endpoint(edge: &Value, flat: &str, nested: &str) -> Result<Endpoint, String> {
    if let Some(node) = edge.get(nested) {
        if node.is_object() {
            return endpoint_object(node);
        }
    }
    if let Some(raw) = edge.get(flat).and_then(|v| v.as_str()) {
        let (kind, key) = split_typed_key(raw)?;
        let name = edge
            .get(format!("{flat}_name"))
            .and_then(|v| v.as_str())
            .unwrap_or(key)
            .to_string();
        let owner = edge
            .get("owner")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let contract_ref = (kind == "contract").then(|| key.to_string());
        return Ok(Endpoint {
            key: format!("{kind}:{key}"),
            kind: kind.to_string(),
            name,
            owner: if flat == "downstream" { owner } else { None },
            contract_ref,
        });
    }
    Err(format!(
        "each edge needs `{flat}` or `{nested}` with a kind and a key"
    ))
}

fn endpoint_object(node: &Value) -> Result<Endpoint, String> {
    let kind = node
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "lineage node needs a kind".to_string())?;
    validate_kind(kind)?;
    let key = node
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "lineage node needs a key".to_string())?;
    if key.is_empty() {
        return Err("lineage node key is empty".into());
    }
    let name = node
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(key)
        .to_string();
    let owner = node
        .get("owner")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let contract_ref = (kind == "contract").then(|| key.to_string());
    Ok(Endpoint {
        key: format!("{kind}:{key}"),
        kind: kind.to_string(),
        name,
        owner,
        contract_ref,
    })
}

fn split_typed_key(raw: &str) -> Result<(&str, &str), String> {
    let (kind, key) = raw
        .split_once(':')
        .ok_or_else(|| format!("lineage key '{raw}' must look like kind:name"))?;
    validate_kind(kind)?;
    if key.is_empty() {
        return Err(format!("lineage key '{raw}' is missing a name"));
    }
    Ok((kind, key))
}

fn validate_kind(kind: &str) -> Result<(), String> {
    match kind {
        "contract" | "dataset" | "table" | "model" | "dashboard" | "consumer" | "job" => Ok(()),
        _ => Err(format!("unknown lineage kind '{kind}'")),
    }
}

fn validate_edge_kind(kind: &str) -> Result<(), String> {
    match kind {
        "feeds" | "displays" | "refreshes" | "owned_by" => Ok(()),
        _ => Err(format!("unknown lineage edge '{kind}'")),
    }
}

fn parse_dbt(manifest: &Value) -> Result<ParsedLineage, String> {
    let mut assets: HashMap<String, ParsedAsset> = HashMap::new();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for bucket in ["nodes", "sources", "exposures"] {
        let Some(map) = manifest.get(bucket).and_then(|v| v.as_object()) else {
            continue;
        };
        for (id, node) in map {
            let resource = node
                .get("resource_type")
                .and_then(|v| v.as_str())
                .unwrap_or(match bucket {
                    "sources" => "source",
                    "exposures" => "exposure",
                    _ => "",
                });
            let Some(kind) = dbt_kind(resource, node.get("type").and_then(|v| v.as_str())) else {
                continue;
            };
            let unique_id = node
                .get("unique_id")
                .and_then(|v| v.as_str())
                .unwrap_or(id.as_str());
            let key = format!("{kind}:{unique_id}");
            let name = node
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(unique_id)
                .to_string();
            let owner = dbt_owner(node);
            let contract_ref = dbt_contract_ref(node);
            // The dbt node stays a model, table, or dashboard. The contract
            // link is a separate asset plus a feeds edge, so resolving the
            // contract id does not rename this node.
            assets.insert(
                key.clone(),
                ParsedAsset {
                    external_key: key.clone(),
                    kind: kind.to_string(),
                    name,
                    owner,
                    contract_ref: None,
                },
            );
            if let Some(contract_ref) = contract_ref {
                let contract_key = format!("contract:{contract_ref}");
                assets.entry(contract_key.clone()).or_insert(ParsedAsset {
                    external_key: contract_key.clone(),
                    kind: "contract".into(),
                    name: contract_ref.clone(),
                    owner: None,
                    contract_ref: Some(contract_ref),
                });
                push_edge(
                    &mut edges,
                    &mut seen,
                    contract_key,
                    key.clone(),
                    "feeds",
                    None,
                );
            }
            let depends = node
                .pointer("/depends_on/nodes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let edge_kind = if kind == "dashboard" {
                "displays"
            } else {
                "feeds"
            };
            for upstream in depends {
                let Some(upstream_id) = upstream.as_str() else {
                    continue;
                };
                let upstream_key = dbt_upstream_key(upstream_id);
                assets
                    .entry(upstream_key.clone())
                    .or_insert_with(|| ParsedAsset {
                        external_key: upstream_key.clone(),
                        kind: upstream_key
                            .split_once(':')
                            .map(|(k, _)| k.to_string())
                            .unwrap_or_else(|| "dataset".into()),
                        name: upstream_id
                            .rsplit('.')
                            .next()
                            .unwrap_or(upstream_id)
                            .to_string(),
                        owner: None,
                        contract_ref: None,
                    });
                push_edge(
                    &mut edges,
                    &mut seen,
                    upstream_key,
                    key.clone(),
                    edge_kind,
                    None,
                );
            }
        }
    }
    if edges.is_empty() {
        return Err("dbt manifest has no lineage edges".into());
    }
    Ok(ParsedLineage {
        format: "dbt_manifest".into(),
        assets: assets.into_values().collect(),
        edges,
    })
}

fn push_edge(
    edges: &mut Vec<ParsedEdge>,
    seen: &mut HashSet<(String, String, String)>,
    from_key: String,
    to_key: String,
    edge_kind: &str,
    field_name: Option<String>,
) {
    if from_key == to_key {
        return;
    }
    if seen.insert((from_key.clone(), to_key.clone(), edge_kind.to_string())) {
        edges.push(ParsedEdge {
            from_key,
            to_key,
            edge_kind: edge_kind.to_string(),
            field_name,
        });
    }
}

fn dbt_kind(resource_type: &str, exposure_type: Option<&str>) -> Option<&'static str> {
    match resource_type {
        "model" => Some("model"),
        "seed" | "snapshot" | "source" => Some("table"),
        "exposure" => Some(match exposure_type.unwrap_or("") {
            "dashboard" => "dashboard",
            _ => "consumer",
        }),
        _ => None,
    }
}

fn dbt_upstream_key(unique_id: &str) -> String {
    let kind = if unique_id.starts_with("model.") {
        "model"
    } else if unique_id.starts_with("source.")
        || unique_id.starts_with("seed.")
        || unique_id.starts_with("snapshot.")
    {
        "table"
    } else if unique_id.starts_with("exposure.") {
        "dashboard"
    } else {
        "dataset"
    };
    format!("{kind}:{unique_id}")
}

fn dbt_owner(node: &Value) -> Option<String> {
    if let Some(owner) = node
        .pointer("/meta/owner")
        .and_then(|v| v.as_str())
        .or_else(|| node.pointer("/config/meta/owner").and_then(|v| v.as_str()))
    {
        return Some(owner.to_string());
    }
    let owner = node.get("owner")?;
    owner
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            owner
                .get("email")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            owner
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn dbt_contract_ref(node: &Value) -> Option<String> {
    for key in ["contract_id", "datacontractgate_contract"] {
        if let Some(value) = node
            .pointer(&format!("/meta/{key}"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                node.pointer(&format!("/config/meta/{key}"))
                    .and_then(|v| v.as_str())
            })
        {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Explanations
// ---------------------------------------------------------------------------

fn explain_asset(
    name: &str,
    kind: &str,
    owner: Option<&str>,
    hops: i32,
    path: &[String],
    changes: &[&FieldChange],
) -> String {
    let owner_bit = match owner.map(str::trim).filter(|s| !s.is_empty()) {
        Some(owner) => format!(", owned by {owner}"),
        None => ", with no owner recorded".to_string(),
    };
    let hop_word = if hops == 1 { "hop" } else { "hops" };
    let via = if path.is_empty() {
        name.to_string()
    } else {
        path.join(" → ")
    };
    let mut text = format!(
        "{name} ({kind}){owner_bit} is {hops} {hop_word} downstream of this contract via {via}."
    );
    let breaking: Vec<&FieldChange> = changes
        .iter()
        .copied()
        .filter(|c| c.severity == "breaking")
        .collect();
    let safe: Vec<&FieldChange> = changes
        .iter()
        .copied()
        .filter(|c| c.severity == "safe")
        .collect();
    if breaking.is_empty() && safe.is_empty() {
        text.push_str(
            " The contract version has no field changes, so this asset is not affected by a schema change.",
        );
    } else if breaking.is_empty() {
        text.push_str(" Nothing breaking reaches this asset.");
        if !safe.is_empty() {
            text.push_str(" Safe: ");
            text.push_str(&join_sentences(&safe));
        }
    } else {
        text.push_str(" Breaking: ");
        text.push_str(&join_sentences(&breaking));
        if !safe.is_empty() {
            text.push_str(" Safe: ");
            text.push_str(&join_sentences(&safe));
        }
    }
    text
}

fn join_sentences(changes: &[&FieldChange]) -> String {
    changes
        .iter()
        .map(|change| change.sentence())
        .collect::<Vec<_>>()
        .join(" ")
}

fn summary_sentence(breaking: usize, safe: usize, affected: usize) -> String {
    let assets = if affected == 1 { "asset" } else { "assets" };
    if breaking == 0 && safe == 0 {
        return format!(
            "No field changes. {affected} downstream {assets} registered on this contract."
        );
    }
    let breaking_word = if breaking == 1 { "change" } else { "changes" };
    let safe_word = if safe == 1 { "change" } else { "changes" };
    format!(
        "{breaking} breaking {breaking_word} and {safe} safe {safe_word}. {affected} downstream {assets} affected."
    )
}

async fn maybe_rewrite_summary(summary: String) -> (String, &'static str) {
    let Ok(url) = std::env::var(IMPACT_LLM_URL_ENV) else {
        return (summary, "template");
    };
    let url = url.trim();
    if url.is_empty() {
        return (summary, "template");
    }
    match rewrite_summary(url, &summary).await {
        Some(text) => (text, "llm"),
        None => (summary, "template"),
    }
}

async fn rewrite_summary(url: &str, summary: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
        .ok()?;
    let response = client
        .post(url)
        .json(&json!({
            "summary": summary,
            "instruction": "Rewrite this DataContractGate impact summary in one or two plain sentences. Do not add assets, owners, or fields that are not already in the summary."
        }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().await.ok()?;
    body.get("summary")
        .or_else(|| body.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct DownRow {
    external_key: String,
    kind: String,
    name: String,
    owner: Option<String>,
    depth: i32,
    name_path: Vec<String>,
    fields: Vec<String>,
}

async fn schema_ready(pool: &PgPool) -> AppResult<bool> {
    let ready: bool = sqlx::query_scalar("SELECT to_regclass('public.lineage_edges') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    Ok(ready)
}

fn missing_schema() -> AppError {
    AppError::Unavailable(SCHEMA_MISSING.to_string())
}

async fn contract_org(pool: &PgPool, contract_id: Uuid) -> AppResult<Uuid> {
    let org_id: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT org_id FROM contracts WHERE id = $1 AND deleted_at IS NULL")
            .bind(contract_id)
            .fetch_optional(pool)
            .await?;
    match org_id {
        Some(Some(id)) => Ok(id),
        Some(None) => Err(AppError::BadRequest(
            "contract has no organization, so lineage cannot be scoped".into(),
        )),
        None => Err(AppError::ContractNotFound(contract_id)),
    }
}

async fn ensure_entitled(pool: &PgPool, self_hosted: bool, org_id: Uuid) -> AppResult<String> {
    let plan = crate::storage::get_org_plan(pool, org_id)
        .await?
        .unwrap_or_else(|| "free".to_string());
    if impact_analysis_allowed(self_hosted, &plan) {
        Ok(plan)
    } else {
        Err(AppError::PlanResourceLimit {
            resource: "impact_analysis",
            limit: 0,
            current: 0,
            message: impact_upgrade_message(),
        })
    }
}

async fn load_downstream(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    changed_fields: &[String],
    include_all: bool,
) -> AppResult<Vec<DownRow>> {
    let kinds: Vec<String> = EDGE_KINDS_WALK.iter().map(|s| (*s).to_string()).collect();
    let rows = sqlx::query_as::<_, DownRow>(DOWNSTREAM_SQL)
        .bind(org_id)
        .bind(contract_id)
        .bind(&kinds)
        .bind(include_all)
        .bind(changed_fields)
        .bind(IMPACT_MAX_HOPS)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

const DOWNSTREAM_SQL: &str = r#"
WITH RECURSIVE walk AS (
    SELECT
        e.to_asset_id AS asset_id,
        1 AS depth,
        ARRAY[src.id, dst.id]::uuid[] AS id_path,
        ARRAY[src.name, dst.name]::text[] AS name_path,
        ARRAY[coalesce(e.field_name, '')]::text[] AS fields
    FROM lineage_edges e
    JOIN lineage_assets src ON src.id = e.from_asset_id
    JOIN lineage_assets dst ON dst.id = e.to_asset_id
    WHERE e.org_id = $1
      AND src.contract_id = $2
      AND e.edge_kind = ANY($3)
      AND ($4::bool OR e.field_name IS NULL OR e.field_name = ANY($5))
    UNION ALL
    SELECT
        e.to_asset_id,
        w.depth + 1,
        w.id_path || e.to_asset_id,
        w.name_path || dst.name,
        w.fields || coalesce(e.field_name, '')
    FROM lineage_edges e
    JOIN walk w ON e.from_asset_id = w.asset_id
    JOIN lineage_assets dst ON dst.id = e.to_asset_id
    WHERE e.org_id = $1
      AND e.edge_kind = ANY($3)
      AND w.depth < $6
      AND NOT e.to_asset_id = ANY(w.id_path)
      AND ($4::bool OR e.field_name IS NULL OR e.field_name = ANY($5))
)
SELECT DISTINCT ON (w.asset_id)
    a.external_key,
    a.kind,
    a.name,
    a.owner,
    w.depth,
    w.name_path,
    w.fields
FROM walk w
JOIN lineage_assets a ON a.id = w.asset_id
ORDER BY w.asset_id, w.depth
"#;

struct BuiltReport {
    body: Value,
}

async fn build_report(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    version: &str,
    version_id: Uuid,
    trigger: &str,
) -> AppResult<BuiltReport> {
    let (yaml,): (String,) =
        sqlx::query_as("SELECT yaml_content FROM contract_versions WHERE id = $1")
            .bind(version_id)
            .fetch_one(pool)
            .await?;
    let base = base_version(pool, contract_id, version_id, trigger).await?;
    let changes = classify_yaml(base.as_ref().map(|(_, yaml)| yaml.as_str()), &yaml)
        .map_err(AppError::BadRequest)?;
    let changed = changed_top_fields(&changes);
    let include_all = changes.is_empty();
    let downstream = load_downstream(pool, org_id, contract_id, &changed, include_all).await?;
    let mut affected = Vec::new();
    for row in &downstream {
        let relevant = changes_for(&changes, &row.fields);
        let owner = row.owner.as_deref();
        let explanation = explain_asset(
            &row.name,
            &row.kind,
            owner,
            row.depth,
            &row.name_path,
            &relevant,
        );
        affected.push(json!({
            "key": row.external_key,
            "kind": row.kind,
            "name": row.name,
            "owner": row.owner,
            "hops": row.depth,
            "path": row.name_path,
            "explanation": explanation,
        }));
    }
    affected.sort_by(|a, b| {
        let hops = a["hops"].as_i64().cmp(&b["hops"].as_i64());
        hops.then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let breaking = changes.iter().filter(|c| c.severity == "breaking").count();
    let safe = changes.len() - breaking;
    let summary = summary_sentence(breaking, safe, affected.len());
    let (summary, summary_source) = maybe_rewrite_summary(summary).await;
    let change_json: Vec<Value> = changes
        .iter()
        .map(|change| {
            json!({
                "field": change.field,
                "kind": change.kind,
                "severity": change.severity,
                "detail": change.detail,
                "explanation": change.sentence(),
            })
        })
        .collect();
    let body = json!({
        "contract_id": contract_id,
        "version": version,
        "base_version": base.as_ref().map(|(v, _)| v.clone()),
        "trigger": trigger,
        "summary": summary,
        "summary_source": summary_source,
        "explanation_source": "template",
        "breaking_count": breaking,
        "safe_count": safe,
        "affected_count": affected.len(),
        "changes": change_json,
        "affected": affected,
    });
    Ok(BuiltReport { body })
}

async fn base_version(
    pool: &PgPool,
    contract_id: Uuid,
    version_id: Uuid,
    trigger: &str,
) -> AppResult<Option<(String, String)>> {
    if trigger == "version_published" {
        let stable: Option<(String, String)> = sqlx::query_as(
            "SELECT version, yaml_content FROM contract_versions \
             WHERE contract_id = $1 AND id <> $2 AND state = 'stable' \
             ORDER BY promoted_at DESC NULLS LAST LIMIT 1",
        )
        .bind(contract_id)
        .bind(version_id)
        .fetch_optional(pool)
        .await?;
        if stable.is_some() {
            return Ok(stable);
        }
    }
    let previous: Option<(String, String)> = sqlx::query_as(
        "SELECT version, yaml_content FROM contract_versions \
         WHERE contract_id = $1 AND id <> $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(contract_id)
    .bind(version_id)
    .fetch_optional(pool)
    .await?;
    Ok(previous)
}

async fn store_report(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    version_id: Uuid,
    report: &Value,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO contract_impact_reports (
            org_id, contract_id, version_id, version, base_version, trigger,
            breaking_count, safe_count, affected_count, summary, report
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (contract_id, version, trigger) DO UPDATE SET
            version_id = EXCLUDED.version_id,
            base_version = EXCLUDED.base_version,
            breaking_count = EXCLUDED.breaking_count,
            safe_count = EXCLUDED.safe_count,
            affected_count = EXCLUDED.affected_count,
            summary = EXCLUDED.summary,
            report = EXCLUDED.report,
            created_at = now()
        "#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(version_id)
    .bind(report["version"].as_str().unwrap_or(""))
    .bind(report["base_version"].as_str())
    .bind(report["trigger"].as_str().unwrap_or("recompute"))
    .bind(report["breaking_count"].as_i64().unwrap_or(0) as i32)
    .bind(report["safe_count"].as_i64().unwrap_or(0) as i32)
    .bind(report["affected_count"].as_i64().unwrap_or(0) as i32)
    .bind(report["summary"].as_str().unwrap_or(""))
    .bind(report)
    .execute(pool)
    .await?;
    Ok(())
}

async fn compute_and_store(
    pool: &PgPool,
    org_id: Uuid,
    contract_id: Uuid,
    version: &str,
    trigger: &str,
) -> AppResult<Value> {
    let version_row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM contract_versions WHERE contract_id = $1 AND version = $2")
            .bind(contract_id)
            .bind(version)
            .fetch_optional(pool)
            .await?;
    let Some((version_id,)) = version_row else {
        return Err(AppError::VersionNotFound {
            contract_id,
            version: version.to_string(),
        });
    };
    let built = build_report(pool, org_id, contract_id, version, version_id, trigger).await?;
    store_report(pool, org_id, contract_id, version_id, &built.body).await?;
    Ok(built.body)
}

async fn load_stored(pool: &PgPool, contract_id: Uuid, version: &str) -> AppResult<Option<Value>> {
    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT report FROM contract_impact_reports \
         WHERE contract_id = $1 AND version = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(contract_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(report,)| report))
}

/// Called after a version is proposed or published. Failures are logged.
/// A missing migration or a plan that does not include the feature is a skip,
/// so creating a version still succeeds.
pub async fn record_version_impact(
    state: &AppState,
    org_id: Option<Uuid>,
    contract_id: Uuid,
    version: &str,
    trigger: &'static str,
) {
    let owner = match contract_org(&state.db, contract_id).await {
        Ok(id) => id,
        Err(_) => return,
    };
    let gate = org_id.unwrap_or(owner);
    let plan = match crate::storage::get_org_plan(&state.db, gate).await {
        Ok(plan) => plan.unwrap_or_else(|| "free".to_string()),
        Err(err) => {
            tracing::warn!(%err, "impact analysis skipped: plan lookup failed");
            return;
        }
    };
    if !impact_analysis_allowed(state.self_hosted, &plan) {
        return;
    }
    match schema_ready(&state.db).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(err) => {
            tracing::warn!(%err, "impact analysis skipped: schema probe failed");
            return;
        }
    }
    if let Err(err) = compute_and_store(&state.db, owner, contract_id, version, trigger).await {
        tracing::warn!(%err, %contract_id, version, trigger, "impact report was not stored");
    }
}

#[derive(Debug)]
struct ImportStats {
    format: String,
    assets: usize,
    edges: usize,
    contracts_linked: usize,
    unlinked_contracts: Vec<String>,
}

async fn replace_lineage(
    pool: &PgPool,
    org_id: Uuid,
    plan: &str,
    self_hosted: bool,
    parsed: ParsedLineage,
) -> AppResult<ImportStats> {
    if let Some(limit) = impact_asset_limit(self_hosted, plan) {
        if parsed.assets.len() as i64 > limit {
            return Err(AppError::PlanResourceLimit {
                resource: "lineage_assets",
                limit,
                current: parsed.assets.len() as i64,
                message: impact_quota_message("lineage assets", limit),
            });
        }
    }
    if let Some(limit) = impact_edge_limit(self_hosted, plan) {
        if parsed.edges.len() as i64 > limit {
            return Err(AppError::PlanResourceLimit {
                resource: "lineage_edges",
                limit,
                current: parsed.edges.len() as i64,
                message: impact_quota_message("lineage edges", limit),
            });
        }
    }
    let contracts: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM contracts WHERE org_id = $1 AND deleted_at IS NULL")
            .bind(org_id)
            .fetch_all(pool)
            .await?;
    let mut unlinked = Vec::new();
    let mut key_remap: HashMap<String, String> = HashMap::new();
    let mut assets = parsed.assets;
    for asset in &mut assets {
        let Some(contract_ref) = asset.contract_ref.clone() else {
            continue;
        };
        match resolve_contract(&contracts, &contract_ref) {
            Some((id, name)) => {
                let new_key = format!("contract:{id}");
                if asset.external_key != new_key {
                    key_remap.insert(asset.external_key.clone(), new_key.clone());
                }
                asset.external_key = new_key;
                asset.contract_ref = Some(id.to_string());
                asset.name = name;
            }
            None => unlinked.push(contract_ref),
        }
    }
    let mut edges = parsed.edges;
    for edge in &mut edges {
        if let Some(next) = key_remap.get(&edge.from_key) {
            edge.from_key = next.clone();
        }
        if let Some(next) = key_remap.get(&edge.to_key) {
            edge.to_key = next.clone();
        }
    }
    let mut dedup: HashMap<String, ParsedAsset> = HashMap::new();
    for asset in assets {
        dedup
            .entry(asset.external_key.clone())
            .and_modify(|existing| {
                if existing.contract_ref.is_none() {
                    existing.contract_ref = asset.contract_ref.clone();
                }
                if existing.owner.is_none() {
                    existing.owner = asset.owner.clone();
                }
            })
            .or_insert(asset);
    }
    let assets: Vec<ParsedAsset> = dedup.into_values().collect();
    let linked = assets
        .iter()
        .filter(|asset| {
            asset.kind == "contract"
                && asset
                    .contract_ref
                    .as_deref()
                    .and_then(|r| Uuid::parse_str(r).ok())
                    .is_some()
        })
        .count();

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM lineage_edges WHERE org_id = $1")
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM lineage_assets WHERE org_id = $1")
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    let mut ids: HashMap<String, Uuid> = HashMap::new();
    for asset in &assets {
        let id = Uuid::new_v4();
        let contract_id = asset
            .contract_ref
            .as_deref()
            .and_then(|r| Uuid::parse_str(r).ok());
        sqlx::query(
            "INSERT INTO lineage_assets (id, org_id, kind, external_key, name, owner, contract_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(org_id)
        .bind(&asset.kind)
        .bind(&asset.external_key)
        .bind(trim_name(&asset.name))
        .bind(asset.owner.as_deref().map(trim_name))
        .bind(contract_id)
        .execute(&mut *tx)
        .await?;
        ids.insert(asset.external_key.clone(), id);
    }
    for edge in &edges {
        let Some(from_id) = ids.get(&edge.from_key) else {
            continue;
        };
        let Some(to_id) = ids.get(&edge.to_key) else {
            continue;
        };
        if from_id == to_id {
            continue;
        }
        sqlx::query(
            "INSERT INTO lineage_edges (org_id, from_asset_id, to_asset_id, edge_kind, field_name) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org_id)
        .bind(from_id)
        .bind(to_id)
        .bind(&edge.edge_kind)
        .bind(edge.field_name.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    unlinked.sort();
    unlinked.dedup();
    Ok(ImportStats {
        format: parsed.format,
        assets: assets.len(),
        edges: edges.len(),
        contracts_linked: linked,
        unlinked_contracts: unlinked,
    })
}

fn resolve_contract(contracts: &[(Uuid, String)], contract_ref: &str) -> Option<(Uuid, String)> {
    if let Ok(id) = Uuid::parse_str(contract_ref) {
        if let Some((_, name)) = contracts.iter().find(|(cid, _)| *cid == id) {
            return Some((id, name.clone()));
        }
    }
    contracts
        .iter()
        .find(|(_, name)| name == contract_ref)
        .map(|(id, name)| (*id, name.clone()))
}

fn trim_name(name: &str) -> String {
    let trimmed = name.trim();
    let source = if trimmed.is_empty() {
        "unnamed"
    } else {
        trimmed
    };
    source.chars().take(240).collect()
}

fn optional_dev_org(state: &AppState, caller: Option<Uuid>, headers: &HeaderMap) -> Option<Uuid> {
    if caller.is_some() || state.auth_configured() {
        return caller;
    }
    headers
        .get("x-org-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s.trim()).ok())
}

fn require_org(state: &AppState, org_id: Option<Uuid>) -> AppResult<Uuid> {
    org_id.ok_or_else(|| {
        if state.auth_configured() {
            AppError::Unauthorized
        } else {
            AppError::BadRequest(
                "no org context: auth is disabled (CONTRACTGATE_DEV_NO_AUTH), so send an \
                 `x-org-id` header with an existing org id"
                    .into(),
            )
        }
    })
}

#[derive(Deserialize)]
pub struct ImportRequest {
    #[serde(default)]
    format: String,
    #[serde(default)]
    document: Option<String>,
    #[serde(default)]
    edges: Option<Value>,
    #[serde(default)]
    nodes: Option<Value>,
    #[serde(default)]
    sources: Option<Value>,
    #[serde(default)]
    exposures: Option<Value>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    dbt_schema_version: Option<Value>,
    /// Dev only, when the request has no API key. Production uses the caller org.
    #[serde(default)]
    contract_id: Option<Uuid>,
}

impl ImportRequest {
    fn document_text(&self) -> AppResult<String> {
        if let Some(document) = &self.document {
            return Ok(document.clone());
        }
        if self.nodes.is_some() || self.sources.is_some() || self.exposures.is_some() {
            return serde_json::to_string(&json!({
                "nodes": self.nodes.clone().unwrap_or_else(|| json!({})),
                "sources": self.sources.clone().unwrap_or_else(|| json!({})),
                "exposures": self.exposures.clone().unwrap_or_else(|| json!({})),
                "metadata": self.metadata.clone().unwrap_or_else(|| json!({})),
                "dbt_schema_version": self.dbt_schema_version.clone(),
            }))
            .map_err(|e| AppError::BadRequest(format!("lineage document could not be read: {e}")));
        }
        if let Some(edges) = &self.edges {
            return serde_json::to_string(&json!({ "edges": edges })).map_err(|e| {
                AppError::BadRequest(format!("lineage document could not be read: {e}"))
            });
        }
        Err(AppError::BadRequest(
            "lineage import needs `document` (a dbt manifest or an edge list) or an `edges` array"
                .into(),
        ))
    }
}

async fn read_gate(
    state: &AppState,
    caller: Option<Uuid>,
    headers: &HeaderMap,
    contract_id: Uuid,
) -> AppResult<(Uuid, Uuid)> {
    if state.auth_configured() && caller.is_none() {
        return Err(AppError::Unauthorized);
    }
    let caller = optional_dev_org(state, caller, headers);
    if let Some(org) = caller {
        crate::scope_for(
            state,
            contract_id,
            Some(org),
            crate::collaboration::CallerRole::Viewer,
        )
        .await?;
    }
    let owner = contract_org(&state.db, contract_id).await?;
    let gate = caller.unwrap_or(owner);
    Ok((gate, owner))
}

async fn import_checked(
    pool: &PgPool,
    org_id: Uuid,
    self_hosted: bool,
    text: &str,
    format: &str,
) -> AppResult<ImportStats> {
    let plan = ensure_entitled(pool, self_hosted, org_id).await?;
    if !schema_ready(pool).await? {
        return Err(missing_schema());
    }
    let parsed = parse_lineage_document(text, format).map_err(AppError::BadRequest)?;
    replace_lineage(pool, org_id, &plan, self_hosted, parsed).await
}

async fn import_org(
    state: &AppState,
    caller: Option<Uuid>,
    headers: &HeaderMap,
    contract_id: Option<Uuid>,
) -> AppResult<Uuid> {
    if let Some(id) = optional_dev_org(state, caller, headers) {
        return Ok(id);
    }
    // Dev mode has no API key. The contract page can name the contract, and
    // the lineage is stored on that contract's org. Authenticated calls never
    // take the org from the body.
    if !state.auth_configured() {
        if let Some(contract_id) = contract_id {
            return contract_org(&state.db, contract_id).await;
        }
    }
    require_org(state, None)
}

pub async fn import_lineage_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    headers: HeaderMap,
    Json(body): Json<ImportRequest>,
) -> AppResult<Json<Value>> {
    let org_id = import_org(&state, org_id, &headers, body.contract_id).await?;
    let text = body.document_text()?;
    let stats = import_checked(&state.db, org_id, state.self_hosted, &text, &body.format).await?;
    Ok(Json(json!({
        "format": stats.format,
        "assets": stats.assets,
        "edges": stats.edges,
        "contracts_linked": stats.contracts_linked,
        "unlinked_contracts": stats.unlinked_contracts,
    })))
}

pub async fn list_lineage_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let org_id = require_org(&state, optional_dev_org(&state, org_id, &headers))?;
    let _plan = ensure_entitled(&state.db, state.self_hosted, org_id).await?;
    if !schema_ready(&state.db).await? {
        return Err(missing_schema());
    }
    type AssetRow = (String, String, String, Option<String>, Option<Uuid>);
    let assets: Vec<AssetRow> = sqlx::query_as(
        "SELECT kind, external_key, name, owner, contract_id FROM lineage_assets \
         WHERE org_id = $1 ORDER BY kind, name LIMIT 500",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    type EdgeRow = (String, String, String, Option<String>);
    let edges: Vec<EdgeRow> = sqlx::query_as(
        "SELECT src.external_key, dst.external_key, e.edge_kind, e.field_name \
         FROM lineage_edges e \
         JOIN lineage_assets src ON src.id = e.from_asset_id \
         JOIN lineage_assets dst ON dst.id = e.to_asset_id \
         WHERE e.org_id = $1 \
         ORDER BY src.external_key, dst.external_key LIMIT 1000",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;
    let asset_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM lineage_assets WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&state.db)
            .await?;
    let edge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM lineage_edges WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(json!({
        "asset_count": asset_count,
        "edge_count": edge_count,
        "assets": assets.into_iter().map(|(kind, key, name, owner, contract_id)| json!({
            "kind": kind,
            "key": key,
            "name": name,
            "owner": owner,
            "contract_id": contract_id,
        })).collect::<Vec<_>>(),
        "edges": edges.into_iter().map(|(from, to, kind, field)| json!({
            "from": from,
            "to": to,
            "edge": kind,
            "field": field,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn impact_get_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    headers: HeaderMap,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<Value>> {
    let (gate, owner) = read_gate(&state, org_id, &headers, contract_id).await?;
    let _plan = ensure_entitled(&state.db, state.self_hosted, gate).await?;
    if !schema_ready(&state.db).await? {
        return Err(missing_schema());
    }
    if let Some(report) = load_stored(&state.db, contract_id, &version).await? {
        return Ok(Json(report));
    }
    let report = compute_and_store(&state.db, owner, contract_id, &version, "recompute").await?;
    Ok(Json(report))
}

pub async fn impact_recompute_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    headers: HeaderMap,
    Path((contract_id, version)): Path<(Uuid, String)>,
) -> AppResult<Json<Value>> {
    let (gate, owner) = read_gate(&state, org_id, &headers, contract_id).await?;
    let _plan = ensure_entitled(&state.db, state.self_hosted, gate).await?;
    if !schema_ready(&state.db).await? {
        return Err(missing_schema());
    }
    let report = compute_and_store(&state.db, owner, contract_id, &version, "recompute").await?;
    Ok(Json(report))
}

pub fn merge_openapi(paths: &mut serde_json::Map<String, Value>) {
    let auth = json!([{ "ApiKeyAuth": [] }]);
    let denied = json!({
        "description": "Plan does not include downstream impact analysis. error and code are plan_limit, resource is impact_analysis, and the body contains no assets, owners, or field changes.",
        "content": {"application/json": {"schema": {
            "type": "object",
            "properties": {
                "error": {"type": "string", "const": "plan_limit"},
                "code": {"type": "string", "const": "plan_limit"},
                "resource": {"type": "string"},
                "limit": {"type": "integer"},
                "current": {"type": "integer"},
                "message": {"type": "string"},
                "upgrade_url": {"type": "string"},
                "status": {"type": "integer"}
            }
        }}}
    });
    let missing = json!({
        "description": "Migration 054_lineage_impact.sql is not applied yet. The gateway still serves every other route."
    });
    paths.insert("/lineage".into(), json!({
        "get": {
            "operationId": "list_lineage",
            "summary": "List lineage assets and edges for the caller org",
            "responses": {
                "200": {"description": "Assets and edges", "content": {"application/json": {"schema": {"type": "object", "properties": {
                    "asset_count": {"type": "integer"},
                    "edge_count": {"type": "integer"},
                    "assets": {"type": "array", "items": {"type": "object"}},
                    "edges": {"type": "array", "items": {"type": "object"}}
                }}}}},
                "401": {"description": "Missing or invalid credentials"},
                "402": denied.clone(),
                "503": missing.clone()
            },
            "security": auth.clone()
        },
        "post": {
            "operationId": "import_lineage",
            "summary": "Replace the org lineage from a dbt manifest.json or an edge list",
            "description": "Accepts a JSON body with `document` (the file text) or an `edges` array. `format` is auto, edges, or dbt_manifest. Replaces the org graph. A dbt node links to a contract when meta.contract_id or meta.datacontractgate_contract is a contract id or name in this org.",
            "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "properties": {
                "document": {"type": "string"},
                "format": {"type": "string", "enum": ["auto", "edges", "dbt_manifest"]},
                "edges": {"type": "array", "items": {"type": "object"}},
                "contract_id": {"type": "string", "format": "uuid", "description": "Used only when the process has auth disabled and the request has no org. The lineage is stored on that contract's org."}
            }}}}},
            "responses": {
                "200": {"description": "Import counts. unlinked_contracts names refs that did not match a contract in this org.", "content": {"application/json": {"schema": {"type": "object", "properties": {
                    "format": {"type": "string"},
                    "assets": {"type": "integer"},
                    "edges": {"type": "integer"},
                    "contracts_linked": {"type": "integer"},
                    "unlinked_contracts": {"type": "array", "items": {"type": "string"}}
                }}}}},
                "400": {"description": "The document is not a manifest or an edge list"},
                "401": {"description": "Missing or invalid credentials"},
                "402": denied.clone(),
                "503": missing.clone()
            },
            "security": auth.clone()
        }
    }));
    paths.insert("/contracts/{id}/versions/{version}/impact".into(), json!({
        "get": {
            "operationId": "get_version_impact",
            "summary": "Downstream impact of a contract version",
            "description": "Returns the report stored when the version was proposed or published. If none is stored, computes one. Free plans receive 402 and no asset data.",
            "parameters": [
                {"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}},
                {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}
            ],
            "responses": {
                "200": {"description": "Breaking and safe field changes, affected assets, and a plain-English explanation per asset.", "content": {"application/json": {"schema": {"type": "object", "properties": {
                    "summary": {"type": "string"},
                    "summary_source": {"type": "string"},
                    "explanation_source": {"type": "string"},
                    "breaking_count": {"type": "integer"},
                    "safe_count": {"type": "integer"},
                    "affected_count": {"type": "integer"},
                    "base_version": {"type": ["string", "null"]},
                    "trigger": {"type": "string"},
                    "changes": {"type": "array", "items": {"type": "object", "properties": {
                        "field": {"type": "string"},
                        "kind": {"type": "string"},
                        "severity": {"type": "string"},
                        "detail": {"type": "string"},
                        "explanation": {"type": "string"}
                    }}},
                    "affected": {"type": "array", "items": {"type": "object", "properties": {
                        "key": {"type": "string"},
                        "kind": {"type": "string"},
                        "name": {"type": "string"},
                        "owner": {"type": ["string", "null"]},
                        "hops": {"type": "integer"},
                        "path": {"type": "array", "items": {"type": "string"}},
                        "explanation": {"type": "string"}
                    }}}
                }}}}},
                "401": {"description": "Missing or invalid credentials"},
                "402": denied.clone(),
                "404": {"description": "Contract or version not found"},
                "503": missing.clone()
            },
            "security": auth.clone()
        },
        "post": {
            "operationId": "recompute_version_impact",
            "summary": "Recompute downstream impact for a contract version",
            "description": "Walks the current lineage and stores a new report. Use this after importing lineage for a version that already exists.",
            "parameters": [
                {"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}},
                {"name": "version", "in": "path", "required": true, "schema": {"type": "string"}}
            ],
            "responses": {
                "200": {"description": "The recomputed report.", "content": {"application/json": {"schema": {"type": "object", "properties": {
                    "summary": {"type": "string"},
                    "affected": {"type": "array", "items": {"type": "object"}},
                    "changes": {"type": "array", "items": {"type": "object"}},
                    "breaking_count": {"type": "integer"},
                    "safe_count": {"type": "integer"}
                }}}}},
                "401": {"description": "Missing or invalid credentials"},
                "402": denied,
                "404": {"description": "Contract or version not found"},
                "503": missing
            },
            "security": auth
        }
    }));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    const V1: &str = r#"
version: "1.0.0"
name: orders
ontology:
  entities:
    - name: amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: false
      enum: [USD, EUR]
    - name: status
      type: string
      required: false
"#;

    const V2: &str = r#"
version: "1.1.0"
name: orders
ontology:
  entities:
    - name: amount
      type: integer
      required: true
      min: 1
    - name: currency
      type: string
      required: true
      enum: [USD]
    - name: note
      type: string
      required: false
"#;

    fn kinds(changes: &[FieldChange]) -> Vec<String> {
        changes
            .iter()
            .map(|c| format!("{}:{}", c.field, c.kind))
            .collect()
    }

    #[test]
    fn classifies_breaking_and_safe_field_changes() {
        let changes = classify_yaml(Some(V1), V2).unwrap();
        let labels = kinds(&changes);
        assert!(
            labels.contains(&"amount:type_narrowed".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"amount:constraint_narrowed".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"currency:required_added".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"currency:enum_value_removed".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"note:field_added_optional".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("status:field_removed")),
            "{labels:?}"
        );
        let narrowed = changes.iter().find(|c| c.kind == "type_narrowed").unwrap();
        assert_eq!(narrowed.severity, "breaking");
        assert!(narrowed.sentence().contains("narrowed"));
        let added = changes
            .iter()
            .find(|c| c.kind == "field_added_optional")
            .unwrap();
        assert_eq!(added.severity, "safe");
        assert!(added.sentence().contains("Optional field"));
    }

    #[test]
    fn rename_is_breaking_and_not_a_remove_plus_add() {
        let renamed = r#"
version: "1.2.0"
name: orders
ontology:
  entities:
    - name: order_total
      type: number
      required: true
      renamed_from: amount
    - name: currency
      type: string
      required: false
      enum: [USD, EUR]
    - name: status
      type: string
      required: false
"#;
        let changes = classify_yaml(Some(V1), renamed).unwrap();
        let labels = kinds(&changes);
        assert!(
            labels.contains(&"order_total:field_renamed".to_string()),
            "{labels:?}"
        );
        assert!(!labels
            .iter()
            .any(|l| l.contains("field_removed") || l.contains("field_added")));
        let rename = changes.iter().find(|c| c.kind == "field_renamed").unwrap();
        assert_eq!(rename.severity, "breaking");
        assert!(rename.sentence().contains("renamed"));
    }

    #[test]
    fn widening_a_type_is_safe() {
        let wider = r#"
version: "1.0.0"
name: orders
ontology:
  entities:
    - name: amount
      type: number
      required: true
"#;
        let before = r#"
version: "1.0.0"
name: orders
ontology:
  entities:
    - name: amount
      type: integer
      required: true
"#;
        let changes = classify_yaml(Some(before), wider).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "type_widened");
        assert_eq!(changes[0].severity, "safe");
    }

    #[test]
    fn reference_walk_is_shortest_field_scoped_and_stops_cycles() {
        let edges = vec![
            WalkEdge {
                from: "contract:orders".into(),
                to: "model:stg".into(),
                field: None,
            },
            WalkEdge {
                from: "model:stg".into(),
                to: "model:mart".into(),
                field: None,
            },
            WalkEdge {
                from: "model:mart".into(),
                to: "model:stg".into(),
                field: None,
            },
            WalkEdge {
                from: "contract:orders".into(),
                to: "model:mart".into(),
                field: None,
            },
            WalkEdge {
                from: "model:stg".into(),
                to: "dashboard:revenue".into(),
                field: None,
            },
            WalkEdge {
                from: "contract:orders".into(),
                to: "model:amount_only".into(),
                field: Some("amount".into()),
            },
            WalkEdge {
                from: "contract:orders".into(),
                to: "model:status_only".into(),
                field: Some("status".into()),
            },
        ];
        let reached = reference_walk(
            &edges,
            &["contract:orders".into()],
            &["amount".into()],
            false,
            8,
        );
        let depth = |key: &str| reached.iter().find(|r| r.key == key).map(|r| r.depth);
        assert_eq!(depth("model:stg"), Some(1));
        assert_eq!(depth("model:mart"), Some(1));
        assert_eq!(depth("dashboard:revenue"), Some(2));
        assert_eq!(depth("model:amount_only"), Some(1));
        assert_eq!(depth("model:status_only"), None);
        assert!(!reached.iter().any(|r| r.key == "contract:orders"));

        let all = reference_walk(&edges, &["contract:orders".into()], &[], true, 8);
        assert!(all.iter().any(|r| r.key == "model:status_only"));

        let mut chain = Vec::new();
        for i in 0..12 {
            chain.push(WalkEdge {
                from: format!("n{i}"),
                to: format!("n{}", i + 1),
                field: None,
            });
        }
        let capped = reference_walk(&chain, &["n0".into()], &[], true, 8);
        assert_eq!(capped.len(), 8);
        assert!(capped.iter().all(|r| r.depth <= 8));
    }

    #[test]
    fn parses_an_edge_list_and_a_dbt_manifest() {
        let yaml = r#"
edges:
  - upstream: contract:orders
    downstream: model:stg_orders
    downstream_name: stg_orders
    owner: finance@example.com
  - from: { kind: model, key: stg_orders, name: stg_orders }
    to: { kind: dashboard, key: finance.revenue, name: Revenue, owner: bi@example.com }
    edge: displays
"#;
        let parsed = parse_lineage_document(yaml, "auto").unwrap();
        assert_eq!(parsed.format, "edges");
        assert_eq!(parsed.edges.len(), 2);
        assert!(parsed
            .assets
            .iter()
            .any(|a| a.external_key == "model:stg_orders"));
        assert!(parsed
            .assets
            .iter()
            .any(|a| a.kind == "dashboard" && a.owner.as_deref() == Some("bi@example.com")));

        let manifest = r#"{
          "metadata": {"dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json"},
          "nodes": {
            "model.acme.stg_orders": {
              "unique_id": "model.acme.stg_orders",
              "resource_type": "model",
              "name": "stg_orders",
              "meta": {"owner": "finance", "contract_id": "orders"},
              "depends_on": {"nodes": ["source.acme.raw.orders"]}
            }
          },
          "sources": {
            "source.acme.raw.orders": {
              "unique_id": "source.acme.raw.orders",
              "resource_type": "source",
              "name": "orders"
            }
          },
          "exposures": {
            "exposure.acme.revenue": {
              "unique_id": "exposure.acme.revenue",
              "resource_type": "exposure",
              "type": "dashboard",
              "name": "Revenue",
              "owner": {"email": "bi@example.com"},
              "depends_on": {"nodes": ["model.acme.stg_orders"]}
            }
          }
        }"#;
        let parsed = parse_lineage_document(manifest, "dbt_manifest").unwrap();
        assert!(parsed.edges.iter().any(|e| e.edge_kind == "displays"));
        assert!(parsed
            .assets
            .iter()
            .any(|a| a.kind == "contract" && a.contract_ref.as_deref() == Some("orders")));
        assert!(parsed
            .assets
            .iter()
            .any(|a| a.owner.as_deref() == Some("bi@example.com")));
    }

    #[test]
    fn asset_explanation_names_the_owner_and_the_breaking_change() {
        let changes = classify_yaml(Some(V1), V2).unwrap();
        let text = explain_asset(
            "Revenue",
            "dashboard",
            Some("bi@example.com"),
            2,
            &["orders".into(), "stg_orders".into(), "Revenue".into()],
            &changes.iter().collect::<Vec<_>>(),
        );
        assert!(text.contains("Revenue (dashboard)"));
        assert!(text.contains("owned by bi@example.com"));
        assert!(text.contains("2 hops"));
        assert!(text.contains("Breaking:"));
        assert!(text.contains("narrowed"));
        assert!(text.contains("Safe:"));
        assert!(!text.contains("ContractGate"));
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL and migration 054"]
    async fn sql_downstream_matches_reference_walk() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        sqlx::query("INSERT INTO orgs (id, name, slug, plan) VALUES ($1, $2, $3, 'impact')")
            .bind(org)
            .bind("impact test")
            .bind(format!("impact-{}", org.simple()))
            .execute(&pool)
            .await
            .unwrap();
        let yaml = V1;
        let (identity, _) = crate::storage::create_contract(
            &pool,
            "orders",
            None,
            yaml,
            Default::default(),
            Some(org),
        )
        .await
        .unwrap();
        let contract_id = identity.id;
        let document = format!(
            r#"
edges:
  - upstream: contract:{contract_id}
    downstream: model:stg
    downstream_name: stg
    owner: finance@example.com
  - upstream: model:stg
    downstream: model:mart
    downstream_name: mart
  - upstream: model:mart
    downstream: model:stg
  - upstream: contract:{contract_id}
    downstream: model:mart
  - upstream: model:stg
    downstream: dashboard:revenue
    downstream_name: Revenue
    downstream_kind: dashboard
    edge: displays
    owner: bi@example.com
  - upstream: contract:{contract_id}
    downstream: model:amount_only
    downstream_name: amount only
    field: amount
  - upstream: contract:{contract_id}
    downstream: model:region_only
    downstream_name: region only
    field: region
"#
        );
        let parsed = parse_lineage_document(&document, "edges").unwrap();
        let walk_edges: Vec<WalkEdge> = parsed
            .edges
            .iter()
            .filter(|e| EDGE_KINDS_WALK.contains(&e.edge_kind.as_str()))
            .map(|e| WalkEdge {
                from: if e.from_key.starts_with("contract:") {
                    format!("contract:{contract_id}")
                } else {
                    e.from_key.clone()
                },
                to: e.to_key.clone(),
                field: e.field_name.clone(),
            })
            .collect();
        let stats = replace_lineage(&pool, org, "impact", false, parsed)
            .await
            .unwrap();
        assert!(stats.contracts_linked >= 1, "{stats:?}");
        let rows = load_downstream(&pool, org, contract_id, &["amount".into()], false)
            .await
            .unwrap();
        let mut from_sql: Vec<Reached> = rows
            .iter()
            .map(|r| Reached {
                key: r.external_key.clone(),
                depth: r.depth,
            })
            .collect();
        from_sql.sort_by(|a, b| a.key.cmp(&b.key));
        let mut from_walk = reference_walk(
            &walk_edges,
            &[format!("contract:{contract_id}")],
            &["amount".to_string()],
            false,
            IMPACT_MAX_HOPS,
        );
        from_walk.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(from_sql, from_walk, "sql={from_sql:?} walk={from_walk:?}");
        assert!(from_sql.iter().any(|r| r.key == "dashboard:revenue"));
        assert!(from_sql.iter().all(|r| r.key != "model:region_only"));

        let v2 = V2;
        crate::storage::create_version(&pool, contract_id, "1.1.0", v2, Some(org))
            .await
            .unwrap();
        let report = compute_and_store(&pool, org, contract_id, "1.1.0", "version_created")
            .await
            .unwrap();
        assert!(report["breaking_count"].as_i64().unwrap() >= 1);
        let affected = report["affected"].as_array().unwrap();
        assert!(affected.iter().any(|a| a["key"] == "dashboard:revenue"));
        assert!(affected.iter().all(|a| a["key"] != "model:region_only"));
        let revenue = affected
            .iter()
            .find(|a| a["key"] == "dashboard:revenue")
            .unwrap();
        let explanation = revenue["explanation"].as_str().unwrap();
        assert!(explanation.contains("Breaking"));
        assert!(explanation.contains("Revenue"));

        let free = Uuid::new_v4();
        sqlx::query("INSERT INTO orgs (id, name, slug, plan) VALUES ($1, 'free', $2, 'free')")
            .bind(free)
            .bind(format!("free-{}", free.simple()))
            .execute(&pool)
            .await
            .unwrap();
        let denied = ensure_entitled(&pool, false, free).await.unwrap_err();
        let response = denied.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::PAYMENT_REQUIRED);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "plan_limit");
        assert_eq!(body["resource"], "impact_analysis");
        assert!(body.get("affected").is_none());
        assert!(body.get("assets").is_none());
        let message = body["message"].as_str().unwrap();
        assert!(message.contains("Impact plan"));
        assert!(!message.contains("stg"));
        assert!(!message.contains("Revenue"));
        let import_denied = import_checked(&pool, free, false, &document, "edges").await;
        assert!(import_denied.is_err());
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM lineage_assets WHERE org_id = $1")
                .bind(free)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);

        sqlx::query("DELETE FROM contracts WHERE org_id = $1 OR org_id = $2")
            .bind(org)
            .bind(free)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1 OR id = $2")
            .bind(org)
            .bind(free)
            .execute(&pool)
            .await
            .unwrap();
    }
}
