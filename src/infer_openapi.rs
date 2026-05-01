//! OpenAPI / AsyncAPI contract inference — derive a draft `Contract` from
//! a `components/schemas` entry.
//!
//! `POST /contracts/infer/openapi`
//!
//! Accepts OpenAPI 3.x or AsyncAPI YAML / JSON strings.  Walks the JSON Schema
//! dialect used by both specs to produce a `Contract`.
//!
//! ## JSON Schema → FieldType mapping
//!
//! | JSON Schema `type` | FieldType |
//! |--------------------|-----------|
//! | `"string"` | String |
//! | `"integer"` | Integer |
//! | `"number"` | Float |
//! | `"boolean"` | Boolean |
//! | `"object"` (or has `properties`) | Object (recurse) |
//! | `"array"` | Array (recurse `items`) |
//! | `"null"` / absent / other | Any |
//!
//! `required` array at the parent object level determines field-level
//! required flags.  `enum`, `minimum`, `maximum`, `pattern`,
//! `minLength`, `maxLength` are passed through directly.

use crate::contract::{Contract, FieldDefinition, FieldType, Ontology};
use crate::error::{AppError, AppResult};
use axum::Json;
use serde_json::Value;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct InferOpenApiRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Full OpenAPI or AsyncAPI document (YAML or JSON string).
    pub openapi_source: String,
    /// Which schema from `components/schemas` to convert.
    /// Defaults to the first schema found.
    #[serde(default)]
    pub schema_name: Option<String>,
}

#[derive(serde::Serialize)]
pub struct InferOpenApiResponse {
    pub yaml_content: String,
    pub field_count: usize,
    /// The schema name that was converted.
    pub schema_used: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_openapi_handler(
    Json(req): Json<InferOpenApiRequest>,
) -> AppResult<Json<InferOpenApiResponse>> {
    if req.openapi_source.trim().is_empty() {
        return Err(AppError::BadRequest("`openapi_source` is empty".into()));
    }

    // Try YAML first (superset of JSON), fall back to JSON.
    let doc: Value = serde_yaml::from_str(&req.openapi_source)
        .map_err(|e| AppError::BadRequest(format!("failed to parse openapi_source: {e}")))?;

    // Find `components/schemas`.  Handles both OpenAPI and AsyncAPI layouts.
    let schemas = find_component_schemas(&doc)
        .ok_or_else(|| AppError::BadRequest("no `components.schemas` found in document".into()))?;

    let schema_name = if let Some(ref name) = req.schema_name {
        name.clone()
    } else {
        schemas
            .as_object()
            .and_then(|m| m.keys().next().cloned())
            .ok_or_else(|| {
                AppError::BadRequest("no schemas found under `components.schemas`".into())
            })?
    };

    let schema_val = schemas.get(&schema_name).ok_or_else(|| {
        AppError::BadRequest(format!(
            "schema `{schema_name}` not found under `components.schemas`"
        ))
    })?;

    let entities = walk_json_schema_object(schema_val)
        .map_err(|e| AppError::BadRequest(format!("schema walk error: {e}")))?;

    let field_count = entities.len();
    let contract = Contract {
        version: "1.0".to_string(),
        name: req.name.clone(),
        description: req.description.clone(),
        compliance_mode: false,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| AppError::Internal(format!("yaml serialisation failed: {e}")))?;

    Ok(Json(InferOpenApiResponse {
        yaml_content,
        field_count,
        schema_used: schema_name,
    }))
}

// ---------------------------------------------------------------------------
// Schema walker
// ---------------------------------------------------------------------------

/// Locate `components.schemas` in an OpenAPI or AsyncAPI document value.
fn find_component_schemas(doc: &Value) -> Option<&Value> {
    doc.pointer("/components/schemas")
}

/// Walk a JSON Schema object and return its fields as `FieldDefinition`s.
///
/// `schema_val` must be an object with `type: "object"` (or have `properties`).
pub fn walk_json_schema_object(schema_val: &Value) -> Result<Vec<FieldDefinition>, String> {
    let obj = schema_val
        .as_object()
        .ok_or("schema must be a JSON object")?;

    let type_tag = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let has_properties = obj.contains_key("properties");

    if type_tag != "object" && !has_properties {
        return Err(format!(
            "schema type is `{type_tag}`, expected `object` or a schema with `properties`"
        ));
    }

    let properties = obj
        .get("properties")
        .and_then(|p| p.as_object())
        .ok_or("schema missing `properties`")?;

    // `required` is an array of field names at the object level.
    let required_fields: HashSet<&str> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    properties
        .iter()
        .map(|(field_name, field_schema)| {
            let is_required = required_fields.contains(field_name.as_str());
            json_schema_to_definition(field_name, field_schema, is_required)
        })
        .collect()
}

/// Convert a single property's JSON Schema into a `FieldDefinition`.
fn json_schema_to_definition(
    name: &str,
    schema: &Value,
    required: bool,
) -> Result<FieldDefinition, String> {
    let obj = schema
        .as_object()
        .ok_or_else(|| format!("property `{name}` schema must be a JSON object"))?;

    let type_tag = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let has_properties = obj.contains_key("properties");

    // Determine FieldType.
    let (field_type, properties, items) = match type_tag {
        "string" => (FieldType::String, None, None),
        "integer" => (FieldType::Integer, None, None),
        "number" => (FieldType::Float, None, None),
        "boolean" => (FieldType::Boolean, None, None),
        "object" | "" if has_properties => {
            // Nested object — recurse.
            let props = walk_json_schema_object(schema)?;
            (FieldType::Object, Some(props), None)
        }
        "array" => {
            // JSON Schema array: has `items` sub-schema.
            let items_schema = obj.get("items");
            let items_def = if let Some(items_s) = items_schema {
                let (item_ft, item_props, item_items) = resolve_schema_type(items_s);
                let item_av = items_s.get("enum").and_then(|e| e.as_array()).cloned();
                FieldDefinition {
                    name: "item".to_string(),
                    field_type: item_ft,
                    required: true,
                    pattern: None,
                    allowed_values: item_av,
                    min: None,
                    max: None,
                    min_length: None,
                    max_length: None,
                    properties: item_props,
                    items: item_items,
                    transform: None,
                }
            } else {
                FieldDefinition {
                    name: "item".to_string(),
                    field_type: FieldType::Any,
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
            };
            (FieldType::Array, None, Some(Box::new(items_def)))
        }
        _ => (FieldType::Any, None, None),
    };

    // Pass-through constraints.
    let pattern = obj
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let allowed_values = obj.get("enum").and_then(|e| e.as_array()).cloned();

    let min = obj
        .get("minimum")
        .and_then(|v| v.as_f64())
        .or_else(|| obj.get("exclusiveMinimum").and_then(|v| v.as_f64()));
    let max = obj
        .get("maximum")
        .and_then(|v| v.as_f64())
        .or_else(|| obj.get("exclusiveMaximum").and_then(|v| v.as_f64()));

    let min_length = obj
        .get("minLength")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let max_length = obj
        .get("maxLength")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    Ok(FieldDefinition {
        name: name.to_string(),
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
    })
}

/// Resolve a sub-schema to (FieldType, Option<properties>, Option<items>)
/// without the full required/name context.
fn resolve_schema_type(
    schema: &Value,
) -> (
    FieldType,
    Option<Vec<FieldDefinition>>,
    Option<Box<FieldDefinition>>,
) {
    let type_tag = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let has_properties = schema.get("properties").is_some();

    match type_tag {
        "string" => (FieldType::String, None, None),
        "integer" => (FieldType::Integer, None, None),
        "number" => (FieldType::Float, None, None),
        "boolean" => (FieldType::Boolean, None, None),
        "object" | "" if has_properties => {
            let props = walk_json_schema_object(schema).unwrap_or_default();
            (FieldType::Object, Some(props), None)
        }
        "array" => (FieldType::Array, None, None),
        _ => (FieldType::Any, None, None),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn walk(schema: &Value) -> Vec<FieldDefinition> {
        walk_json_schema_object(schema).expect("walk failed")
    }

    #[test]
    fn basic_types_and_required() {
        let schema = json!({
            "type": "object",
            "required": ["user_id", "event_type"],
            "properties": {
                "user_id":    {"type": "string"},
                "event_type": {"type": "string"},
                "amount":     {"type": "number"},
                "active":     {"type": "boolean"},
                "count":      {"type": "integer"}
            }
        });
        let fields = walk(&schema);
        // Order may vary — index by name.
        let by_name: std::collections::HashMap<&str, &FieldDefinition> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();

        assert_eq!(by_name["user_id"].field_type, FieldType::String);
        assert!(by_name["user_id"].required);
        assert_eq!(by_name["amount"].field_type, FieldType::Float);
        assert!(!by_name["amount"].required);
        assert_eq!(by_name["count"].field_type, FieldType::Integer);
        assert_eq!(by_name["active"].field_type, FieldType::Boolean);
    }

    #[test]
    fn enum_passthrough() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["active", "inactive"]
                }
            }
        });
        let fields = walk(&schema);
        let status = &fields[0];
        assert_eq!(status.field_type, FieldType::String);
        let av = status.allowed_values.as_ref().unwrap();
        assert_eq!(av.len(), 2);
    }

    #[test]
    fn constraints_passthrough() {
        let schema = json!({
            "type": "object",
            "properties": {
                "amount": {"type": "number", "minimum": 0.0, "maximum": 10000.0},
                "code":   {"type": "string", "pattern": "^[A-Z]{3}$", "minLength": 3, "maxLength": 3}
            }
        });
        let fields = walk(&schema);
        let by_name: std::collections::HashMap<&str, &FieldDefinition> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();

        assert_eq!(by_name["amount"].min, Some(0.0));
        assert_eq!(by_name["amount"].max, Some(10000.0));
        assert_eq!(by_name["code"].pattern.as_deref(), Some("^[A-Z]{3}$"));
        assert_eq!(by_name["code"].min_length, Some(3));
        assert_eq!(by_name["code"].max_length, Some(3));
    }

    #[test]
    fn nested_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "meta": {
                    "type": "object",
                    "properties": {
                        "env":    {"type": "string"},
                        "region": {"type": "string"}
                    }
                }
            }
        });
        let fields = walk(&schema);
        assert_eq!(fields[0].field_type, FieldType::Object);
        let props = fields[0].properties.as_ref().unwrap();
        assert_eq!(props.len(), 2);
    }

    #[test]
    fn array_field() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        });
        let fields = walk(&schema);
        assert_eq!(fields[0].field_type, FieldType::Array);
        let items = fields[0].items.as_ref().unwrap();
        assert_eq!(items.field_type, FieldType::String);
    }

    #[test]
    fn yaml_source_round_trip() {
        let yaml = r#"
openapi: "3.0.0"
components:
  schemas:
    Event:
      type: object
      required: [user_id]
      properties:
        user_id:
          type: string
        amount:
          type: number
"#;
        let doc: Value = serde_yaml::from_str(yaml).unwrap();
        let schemas = find_component_schemas(&doc).unwrap();
        let schema = schemas.get("Event").unwrap();
        let fields = walk(schema);
        assert_eq!(fields.len(), 2);
        let by_name: std::collections::HashMap<&str, &FieldDefinition> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();
        assert!(by_name["user_id"].required);
        assert!(!by_name["amount"].required);
    }
}
