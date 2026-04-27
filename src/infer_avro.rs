//! Avro contract inference — derive a draft `Contract` from an Avro schema
//! (`.avsc`) or a list of JSON sample objects.
//!
//! `POST /contracts/infer/avro`
//!
//! ## Two modes
//!
//! | Input | Mode | Notes |
//! |-------|------|-------|
//! | `schema` field | Schema-driven | Parse `.avsc` JSON → deterministic |
//! | `samples` field | Sample-driven | Delegates to `infer::infer_fields_from_objects` |
//!
//! When both are present, `schema` takes precedence.
//!
//! ## Avro → FieldType mapping
//!
//! | Avro type | FieldType |
//! |-----------|-----------|
//! | `"string"` / `"bytes"` | String |
//! | `"int"` / `"long"` | Integer |
//! | `"float"` / `"double"` | Float |
//! | `"boolean"` | Boolean |
//! | `"record"` | Object (recurse) |
//! | `"array"` | Array |
//! | `"map"` | Object |
//! | `"enum"` | String + allowed_values |
//! | union `["null", T]` | T with `required: false` |
//! | union of 2+ non-null | Any |
//! | `"null"` alone | skipped |

use crate::contract::{Contract, FieldDefinition, FieldType, Ontology};
use crate::error::{AppError, AppResult};
use crate::infer::infer_fields_from_objects_pub;
use axum::Json;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct InferAvroRequest {
    /// Name for the generated contract.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Avro schema as a JSON string (`.avsc` content).  Schema-driven mode.
    #[serde(default)]
    pub schema: Option<String>,
    /// Raw JSON objects to infer from.  Sample-driven fallback.
    #[serde(default)]
    pub samples: Vec<Value>,
}

#[derive(serde::Serialize)]
pub struct InferAvroResponse {
    pub yaml_content: String,
    pub field_count: usize,
    /// `"schema"` or `"samples"`
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_avro_handler(
    Json(req): Json<InferAvroRequest>,
) -> AppResult<Json<InferAvroResponse>> {
    let (entities, mode) = if let Some(ref schema_str) = req.schema {
        // Schema-driven: parse .avsc JSON.
        let schema_val: Value = serde_json::from_str(schema_str)
            .map_err(|e| AppError::BadRequest(format!("invalid avsc JSON: {e}")))?;
        let entities = walk_avro_schema(&schema_val)
            .map_err(|e| AppError::BadRequest(format!("avsc walk error: {e}")))?;
        (entities, "schema")
    } else if !req.samples.is_empty() {
        // Sample-driven: reuse JSON inference logic.
        for (i, s) in req.samples.iter().enumerate() {
            if !s.is_object() {
                return Err(AppError::BadRequest(format!(
                    "sample[{i}] is not a JSON object"
                )));
            }
        }
        let entities = infer_fields_from_objects_pub(&req.samples);
        (entities, "samples")
    } else {
        return Err(AppError::BadRequest(
            "provide either `schema` (avsc JSON string) or `samples` (JSON objects)".into(),
        ));
    };

    let field_count = entities.len();
    let contract = Contract {
        version: "1.0".to_string(),
        name: req.name.clone(),
        description: req.description.clone(),
        compliance_mode: false,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
    };

    let yaml_content = serde_yaml::to_string(&contract)
        .map_err(|e| AppError::Internal(format!("yaml serialisation failed: {e}")))?;

    Ok(Json(InferAvroResponse {
        yaml_content,
        field_count,
        mode: mode.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Avro schema walker
// ---------------------------------------------------------------------------

/// Walk an Avro schema value and return a flat list of `FieldDefinition`s.
///
/// Accepts:
/// - A top-level `"record"` object with a `"fields"` array.
/// - A JSON string like `"string"`, `"int"`, etc. (used for inline field types).
pub fn walk_avro_schema(schema: &Value) -> Result<Vec<FieldDefinition>, String> {
    match schema {
        Value::Object(obj) => {
            let type_tag = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match type_tag {
                "record" => {
                    let fields = obj
                        .get("fields")
                        .and_then(|f| f.as_array())
                        .ok_or("record schema missing `fields` array")?;
                    fields.iter().map(avro_field_to_definition).collect()
                }
                _ => Err(format!(
                    "top-level schema must be a record, got type={type_tag:?}"
                )),
            }
        }
        _ => Err("top-level avsc must be a JSON object".to_string()),
    }
}

/// Convert a single Avro field object `{"name": ..., "type": ..., ...}` into
/// a `FieldDefinition`.
fn avro_field_to_definition(field: &Value) -> Result<FieldDefinition, String> {
    let obj = field
        .as_object()
        .ok_or("field entry is not a JSON object")?;

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("field missing `name`")?
        .to_string();

    let type_val = obj.get("type").ok_or("field missing `type`")?;

    let (field_type, required, allowed_values, properties, items) =
        resolve_avro_type(type_val)?;

    Ok(FieldDefinition {
        name,
        field_type,
        required,
        pattern: None,
        allowed_values,
        min: None,
        max: None,
        min_length: None,
        max_length: None,
        properties,
        items,
        transform: None,
    })
}

/// Resolve an Avro type value (string, object, or union array) into
/// `(FieldType, required, allowed_values, properties, items)`.
fn resolve_avro_type(
    type_val: &Value,
) -> Result<
    (
        FieldType,
        bool,
        Option<Vec<Value>>,
        Option<Vec<FieldDefinition>>,
        Option<Box<FieldDefinition>>,
    ),
    String,
> {
    match type_val {
        // Primitive string names: "string", "int", "long", etc.
        Value::String(s) => {
            let ft = avro_primitive_to_field_type(s)?;
            Ok((ft, true, None, None, None))
        }

        // Complex type object: {"type": "record"|"array"|"map"|"enum", ...}
        Value::Object(obj) => {
            let type_tag = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match type_tag {
                "record" => {
                    // Nested record → Object with properties.
                    let nested_fields = obj
                        .get("fields")
                        .and_then(|f| f.as_array())
                        .ok_or("nested record missing `fields`")?;
                    let props: Vec<FieldDefinition> = nested_fields
                        .iter()
                        .map(avro_field_to_definition)
                        .collect::<Result<_, _>>()?;
                    Ok((FieldType::Object, true, None, Some(props), None))
                }
                "array" => {
                    // Avro array: {"type": "array", "items": <type>}
                    let items_val =
                        obj.get("items").ok_or("array schema missing `items`")?;
                    let (item_ft, item_req, item_av, item_props, item_items) =
                        resolve_avro_type(items_val)?;
                    let items_def = FieldDefinition {
                        name: "item".to_string(),
                        field_type: item_ft,
                        required: item_req,
                        pattern: None,
                        allowed_values: item_av,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: item_props,
                        items: item_items,
                        transform: None,
                    };
                    Ok((FieldType::Array, true, None, None, Some(Box::new(items_def))))
                }
                "map" => {
                    // Avro map: string-keyed object.
                    Ok((FieldType::Object, true, None, None, None))
                }
                "enum" => {
                    // Avro enum: {"type": "enum", "symbols": [...]}
                    let symbols = obj
                        .get("symbols")
                        .and_then(|s| s.as_array())
                        .ok_or("enum schema missing `symbols`")?;
                    let allowed: Vec<Value> = symbols
                        .iter()
                        .filter_map(|s| s.as_str())
                        .map(|s| Value::String(s.to_string()))
                        .collect();
                    Ok((FieldType::String, true, Some(allowed), None, None))
                }
                "fixed" => Ok((FieldType::String, true, None, None, None)),
                other => Err(format!("unknown complex Avro type: {other:?}")),
            }
        }

        // Union: ["null", T] or ["null", T1, T2, ...]
        Value::Array(variants) => {
            // Separate nulls from non-null variants.
            let non_null: Vec<&Value> = variants
                .iter()
                .filter(|v| v.as_str() != Some("null") && v != &&&Value::Null)
                .collect();

            let has_null = non_null.len() < variants.len();

            if non_null.is_empty() {
                // Union of only nulls — skip / Any.
                return Ok((FieldType::Any, false, None, None, None));
            }

            if non_null.len() == 1 {
                // nullable(T) — common pattern.
                let (ft, _, av, props, items) = resolve_avro_type(non_null[0])?;
                let required = !has_null;
                return Ok((ft, required, av, props, items));
            }

            // Multiple non-null variants → Any.
            Ok((FieldType::Any, !has_null, None, None, None))
        }

        _ => Err(format!("unexpected Avro type value: {type_val}")),
    }
}

/// Map Avro primitive type name to `FieldType`.
fn avro_primitive_to_field_type(s: &str) -> Result<FieldType, String> {
    match s {
        "string" | "bytes" => Ok(FieldType::String),
        "int" | "long" => Ok(FieldType::Integer),
        "float" | "double" => Ok(FieldType::Float),
        "boolean" => Ok(FieldType::Boolean),
        "null" => {
            // A bare "null" type in a union is handled by the caller;
            // if we arrive here it means a field declared as just "null".
            Ok(FieldType::Any)
        }
        other => Err(format!("unknown Avro primitive type: {other:?}")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(s: &Value) -> Vec<FieldDefinition> {
        walk_avro_schema(s).expect("walk failed")
    }

    #[test]
    fn basic_primitives() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "user_id",    "type": "string"},
                {"name": "count",      "type": "int"},
                {"name": "score",      "type": "double"},
                {"name": "active",     "type": "boolean"}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0].field_type, FieldType::String);
        assert_eq!(fields[1].field_type, FieldType::Integer);
        assert_eq!(fields[2].field_type, FieldType::Float);
        assert_eq!(fields[3].field_type, FieldType::Boolean);
        assert!(fields[0].required);
    }

    #[test]
    fn nullable_union_marks_optional() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "amount", "type": ["null", "double"]}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields[0].field_type, FieldType::Float);
        assert!(!fields[0].required);
    }

    #[test]
    fn enum_produces_allowed_values() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "event_type", "type": {
                    "type": "enum",
                    "name": "EventType",
                    "symbols": ["click", "view", "purchase"]
                }}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields[0].field_type, FieldType::String);
        let av = fields[0].allowed_values.as_ref().unwrap();
        assert_eq!(av.len(), 3);
    }

    #[test]
    fn nested_record_becomes_object_with_properties() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "meta", "type": {
                    "type": "record",
                    "name": "Meta",
                    "fields": [
                        {"name": "env",    "type": "string"},
                        {"name": "region", "type": "string"}
                    ]
                }}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields[0].field_type, FieldType::Object);
        let props = fields[0].properties.as_ref().unwrap();
        assert_eq!(props.len(), 2);
        assert!(props.iter().any(|p| p.name == "env"));
    }

    #[test]
    fn array_field() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "tags", "type": {"type": "array", "items": "string"}}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields[0].field_type, FieldType::Array);
        let items = fields[0].items.as_ref().unwrap();
        assert_eq!(items.field_type, FieldType::String);
    }

    #[test]
    fn multi_non_null_union_is_any() {
        let avsc = json!({
            "type": "record",
            "name": "Event",
            "fields": [
                {"name": "value", "type": ["null", "string", "int"]}
            ]
        });
        let fields = schema(&avsc);
        assert_eq!(fields[0].field_type, FieldType::Any);
    }

    #[test]
    fn rejects_non_record_top_level() {
        let avsc = json!({"type": "array", "items": "string"});
        assert!(walk_avro_schema(&avsc).is_err());
    }
}
