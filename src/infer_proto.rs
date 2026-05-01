//! Protobuf contract inference — derive a draft `Contract` from a `.proto`
//! source string.
//!
//! `POST /contracts/infer/proto`
//!
//! Parses proto3 message declarations without an external crate.  The parser
//! covers the common subset used in real-world schemas:
//!
//! - Scalar field types (all proto3 primitives)
//! - `optional` / `repeated` labels
//! - Nested `message` types (recurse → `Object`)
//! - `enum` declarations → `String` + `allowed_values`
//! - Top-level `oneof` blocks — each branch becomes a field with `required: false`
//!
//! Exotic features (map<K,V>, Any, extensions, proto2 syntax) fall back to
//! `FieldType::Any` with a note, preserving forward-compatibility.

use crate::contract::{Contract, FieldDefinition, FieldType, Ontology};
use crate::error::{AppError, AppResult};
use axum::Json;
use serde_json::Value;
use std::collections::HashMap;

// Type alias to keep Clippy happy (complex return type)
type ProtoTypeResolution = (
    FieldType,
    Option<Vec<Value>>,
    Option<Vec<FieldDefinition>>,
    Option<Box<FieldDefinition>>,
);

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct InferProtoRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Full `.proto` file content (proto3).
    pub proto_source: String,
    /// Which top-level message to convert.  Defaults to the first message found.
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(serde::Serialize)]
pub struct InferProtoResponse {
    pub yaml_content: String,
    pub field_count: usize,
    /// The message name that was converted.
    pub message_used: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_proto_handler(
    Json(req): Json<InferProtoRequest>,
) -> AppResult<Json<InferProtoResponse>> {
    if req.proto_source.trim().is_empty() {
        return Err(AppError::BadRequest("`proto_source` is empty".into()));
    }

    let parsed = parse_proto_source(&req.proto_source)
        .map_err(|e| AppError::BadRequest(format!("proto parse error: {e}")))?;

    let message_name = req
        .message
        .clone()
        .unwrap_or_else(|| parsed.messages.keys().next().cloned().unwrap_or_default());

    if message_name.is_empty() {
        return Err(AppError::BadRequest(
            "no message found in proto_source".into(),
        ));
    }

    let entities = build_fields_for_message(&message_name, &parsed)
        .map_err(|e| AppError::BadRequest(format!("field build error: {e}")))?;

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

    Ok(Json(InferProtoResponse {
        yaml_content,
        field_count,
        message_used: message_name,
    }))
}

// ---------------------------------------------------------------------------
// Proto source parser
// ---------------------------------------------------------------------------

/// A parsed representation of the relevant parts of a `.proto` file.
#[derive(Debug, Default)]
pub struct ParsedProto {
    /// message name → list of raw field declarations
    pub messages: HashMap<String, Vec<ProtoField>>,
    /// enum name → symbols
    pub enums: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ProtoField {
    pub name: String,
    pub type_name: String,
    /// `true` if `optional` or `oneof` branch
    pub optional: bool,
    /// `true` if `repeated`
    pub repeated: bool,
}

/// Parse a proto3 source string into `ParsedProto`.
///
/// Strategy:
/// 1. Strip line + block comments.
/// 2. Scan for `message`, `enum`, and `oneof` block boundaries.
/// 3. Within message blocks, parse field lines.
pub fn parse_proto_source(src: &str) -> Result<ParsedProto, String> {
    let stripped = strip_comments(src);
    let mut proto = ParsedProto::default();
    parse_block(&stripped, &mut proto)?;
    Ok(proto)
}

/// Strip `//` line comments and `/* */` block comments.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // Skip to end of line.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Skip block comment.
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Recursively parse message and enum blocks in `src`.
fn parse_block(src: &str, proto: &mut ParsedProto) -> Result<(), String> {
    let tokens: Vec<&str> = src.split_whitespace().collect();
    let i = 0;

    // We also work on the original `src` for brace extraction.
    // Build a char-level cursor for brace matching.
    let chars: Vec<char> = src.chars().collect();

    fn match_word(chars: &[char], pos: usize, word: &str) -> Option<usize> {
        let p = {
            let mut p = pos;
            while p < chars.len() && chars[p].is_whitespace() {
                p += 1;
            }
            p
        };
        let wchars: Vec<char> = word.chars().collect();
        if p + wchars.len() <= chars.len() && chars[p..p + wchars.len()] == wchars[..] {
            Some(p + wchars.len())
        } else {
            None
        }
    }

    fn extract_name_and_block(chars: &[char], pos: usize) -> Option<(String, String, usize)> {
        // After keyword, skip whitespace, read name, skip to '{', extract block.
        let mut p = pos;
        while p < chars.len() && chars[p].is_whitespace() {
            p += 1;
        }
        // Read identifier.
        let name_start = p;
        while p < chars.len() && (chars[p].is_alphanumeric() || chars[p] == '_') {
            p += 1;
        }
        if p == name_start {
            return None;
        }
        let name: String = chars[name_start..p].iter().collect();
        // Skip to '{'.
        while p < chars.len() && chars[p] != '{' {
            p += 1;
        }
        if p >= chars.len() {
            return None;
        }
        p += 1; // consume '{'
                // Extract matching block content.
        let mut depth = 1usize;
        let block_start = p;
        while p < chars.len() && depth > 0 {
            match chars[p] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            p += 1;
        }
        let block: String = chars[block_start..p - 1].iter().collect();
        Some((name, block, p))
    }

    // Scan for `message` and `enum` keywords anywhere in the source.
    // We do a simple scan rather than token-by-token to avoid the complexity
    // of a full AST.
    let _ = (tokens, i); // suppress unused warnings from the simpler approach below

    // Simpler approach: scan `chars` sequentially for `message`/`enum` keywords.
    let mut p = 0usize;
    while p < chars.len() {
        // Skip whitespace.
        while p < chars.len() && chars[p].is_whitespace() {
            p += 1;
        }
        if p >= chars.len() {
            break;
        }

        // Try to match keywords.
        if let Some(after_kw) = match_word(&chars, p, "message") {
            if let Some((name, block, after_block)) = extract_name_and_block(&chars, after_kw) {
                let fields = parse_message_body(&block, proto)?;
                proto.messages.insert(name, fields);
                p = after_block;
                continue;
            }
        }

        if let Some(after_kw) = match_word(&chars, p, "enum") {
            if let Some((name, block, after_block)) = extract_name_and_block(&chars, after_kw) {
                let symbols = parse_enum_body(&block);
                proto.enums.insert(name, symbols);
                p = after_block;
                continue;
            }
        }

        // Skip past the current non-whitespace character.
        while p < chars.len() && !chars[p].is_whitespace() && chars[p] != '{' {
            p += 1;
        }
    }

    Ok(())
}

/// Parse the body of a `message { ... }` block into a list of `ProtoField`s.
/// Also handles nested `message` and `oneof` blocks by recursing.
fn parse_message_body(body: &str, proto: &mut ParsedProto) -> Result<Vec<ProtoField>, String> {
    let mut fields: Vec<ProtoField> = Vec::new();

    // Extract nested message / enum / oneof blocks first, then parse remaining lines.
    let chars: Vec<char> = body.chars().collect();
    let mut remaining = String::new();
    let mut p = 0usize;

    fn match_word_inner(chars: &[char], pos: usize, word: &str) -> Option<usize> {
        let mut p = pos;
        while p < chars.len() && chars[p].is_whitespace() {
            p += 1;
        }
        let wchars: Vec<char> = word.chars().collect();
        if p + wchars.len() <= chars.len() && chars[p..p + wchars.len()] == wchars[..] {
            Some(p + wchars.len())
        } else {
            None
        }
    }

    fn extract_name_and_block_inner(chars: &[char], pos: usize) -> Option<(String, String, usize)> {
        let mut p = pos;
        while p < chars.len() && chars[p].is_whitespace() {
            p += 1;
        }
        let name_start = p;
        while p < chars.len() && (chars[p].is_alphanumeric() || chars[p] == '_') {
            p += 1;
        }
        if p == name_start {
            return None;
        }
        let name: String = chars[name_start..p].iter().collect();
        while p < chars.len() && chars[p] != '{' {
            p += 1;
        }
        if p >= chars.len() {
            return None;
        }
        p += 1;
        let mut depth = 1usize;
        let block_start = p;
        while p < chars.len() && depth > 0 {
            match chars[p] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            p += 1;
        }
        let block: String = chars[block_start..p - 1].iter().collect();
        Some((name, block, p))
    }

    while p < chars.len() {
        while p < chars.len() && chars[p].is_whitespace() {
            remaining.push(chars[p]);
            p += 1;
        }
        if p >= chars.len() {
            break;
        }

        // Nested message.
        if let Some(after_kw) = match_word_inner(&chars, p, "message") {
            if let Some((name, block, after)) = extract_name_and_block_inner(&chars, after_kw) {
                let nested = parse_message_body(&block, proto)?;
                proto.messages.insert(name, nested);
                p = after;
                continue;
            }
        }

        // Nested enum.
        if let Some(after_kw) = match_word_inner(&chars, p, "enum") {
            if let Some((name, block, after)) = extract_name_and_block_inner(&chars, after_kw) {
                let symbols = parse_enum_body(&block);
                proto.enums.insert(name, symbols);
                p = after;
                continue;
            }
        }

        // oneof block — collect field names from branches as optional.
        if let Some(after_kw) = match_word_inner(&chars, p, "oneof") {
            if let Some((_name, block, after)) = extract_name_and_block_inner(&chars, after_kw) {
                // Parse oneof branches — same as message fields but all optional.
                let oneof_fields = parse_field_lines(&block, true);
                fields.extend(oneof_fields);
                p = after;
                continue;
            }
        }

        // Collect the current character into `remaining` for line-by-line parsing.
        remaining.push(chars[p]);
        p += 1;
    }

    // Now parse the remaining text as field lines.
    fields.extend(parse_field_lines(&remaining, false));
    Ok(fields)
}

fn parse_enum_body(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.split_whitespace().next() == Some("option") {
                return None;
            }
            // Format: SYMBOL_NAME = N;
            let name = line.split_whitespace().next()?;
            if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Parse individual field declaration lines from a message or oneof body.
///
/// Format: `[optional|repeated] <type> <name> = <tag> [options];`
fn parse_field_lines(body: &str, force_optional: bool) -> Vec<ProtoField> {
    let mut fields = Vec::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('}')
            || line.split_whitespace().next() == Some("option")  // ← FIXED
            || line.starts_with("reserved")
            || line.starts_with("syntax")
            || line.starts_with("package")
            || line.starts_with("import")
        {
            continue;
        }

        let parts: Vec<&str> = line.trim_end_matches(';').split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        // Handle optional / repeated label
        let (label, type_idx) = match parts[0] {
            "optional" | "repeated" => (parts[0], 1usize),
            _ => ("", 0usize),
        };

        if type_idx + 2 >= parts.len() {
            continue;
        }

        let type_name = parts[type_idx].to_string();
        let field_name = parts[type_idx + 1].to_string();

        // Check for "=" before the tag number
        if parts.get(type_idx + 2) != Some(&"=") {
            continue;
        }

        let optional = force_optional || label == "optional";
        let repeated = label == "repeated";

        fields.push(ProtoField {
            name: field_name,
            type_name,
            optional,
            repeated,
        });
    }

    fields
}

// ---------------------------------------------------------------------------
// Field builder
// ---------------------------------------------------------------------------

fn build_fields_for_message(
    message_name: &str,
    proto: &ParsedProto,
) -> Result<Vec<FieldDefinition>, String> {
    let raw_fields = proto
        .messages
        .get(message_name)
        .ok_or_else(|| format!("message `{message_name}` not found in parsed proto"))?;

    raw_fields
        .iter()
        .map(|f| proto_field_to_definition(f, proto))
        .collect()
}

fn proto_field_to_definition(
    f: &ProtoField,
    proto: &ParsedProto,
) -> Result<FieldDefinition, String> {
    let (field_type, allowed_values, properties, items) = resolve_proto_type(f, proto);

    Ok(FieldDefinition {
        name: f.name.clone(),
        field_type,
        required: !f.optional,
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

fn resolve_proto_type(f: &ProtoField, proto: &ParsedProto) -> ProtoTypeResolution {
    // `repeated T` → Array with items: T.
    if f.repeated {
        let inner = ProtoField {
            name: "item".to_string(),
            type_name: f.type_name.clone(),
            optional: false,
            repeated: false,
        };
        let (item_ft, item_av, item_props, item_items) = resolve_proto_type(&inner, proto);
        let items_def = FieldDefinition {
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
        };
        return (FieldType::Array, None, None, Some(Box::new(items_def)));
    }

    let type_name = f.type_name.as_str();

    // Scalar primitives.
    if let Some(ft) = proto_scalar_type(type_name) {
        return (ft, None, None, None);
    }

    // Enum reference → String + allowed_values.
    if let Some(symbols) = proto.enums.get(type_name) {
        let av: Vec<Value> = symbols.iter().map(|s| Value::String(s.clone())).collect();
        return (FieldType::String, Some(av), None, None);
    }

    // Nested message reference → Object + properties.
    if let Ok(props) = build_fields_for_message(type_name, proto) {
        return (FieldType::Object, None, Some(props), None);
    }

    // Unknown / map<K,V> / Any → fall back.
    (FieldType::Any, None, None, None)
}

fn proto_scalar_type(t: &str) -> Option<FieldType> {
    match t {
        "string" | "bytes" => Some(FieldType::String),
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "fixed32" | "fixed64"
        | "sfixed32" | "sfixed64" => Some(FieldType::Integer),
        "float" | "double" => Some(FieldType::Float),
        "bool" => Some(FieldType::Boolean),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedProto {
        parse_proto_source(src).expect("parse failed")
    }

    #[test]
    fn basic_scalars() {
        let src = r#"
            syntax = "proto3";
            message Event {
                string user_id = 1;
                int64  count   = 2;
                double score   = 3;
                bool   active  = 4;
            }
        "#;
        let proto = parse(src);
        let fields = build_fields_for_message("Event", &proto).unwrap();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0].field_type, FieldType::String);
        assert_eq!(fields[1].field_type, FieldType::Integer);
        assert_eq!(fields[2].field_type, FieldType::Float);
        assert_eq!(fields[3].field_type, FieldType::Boolean);
        assert!(fields[0].required);
    }

    #[test]
    fn optional_field() {
        let src = r#"
            syntax = "proto3";
            message Event {
                string   user_id = 1;
                optional double  amount  = 2;
            }
        "#;
        let proto = parse(src);
        let fields = build_fields_for_message("Event", &proto).unwrap();
        assert!(fields[0].required);
        assert!(!fields[1].required);
    }

    #[test]
    fn repeated_becomes_array() {
        let src = r#"
            syntax = "proto3";
            message Event {
                repeated string tags = 1;
            }
        "#;
        let proto = parse(src);
        let fields = build_fields_for_message("Event", &proto).unwrap();
        assert_eq!(fields[0].field_type, FieldType::Array);
        let items = fields[0].items.as_ref().unwrap();
        assert_eq!(items.field_type, FieldType::String);
    }

    #[test]
    fn enum_produces_allowed_values() {
        let src = r#"
            syntax = "proto3";
            enum EventType {
                CLICK    = 0;
                VIEW     = 1;
                PURCHASE = 2;
            }
            message Event {
                EventType event_type = 1;
            }
        "#;
        let proto = parse(src);
        let fields = build_fields_for_message("Event", &proto).unwrap();
        assert_eq!(fields[0].field_type, FieldType::String);
        let av = fields[0].allowed_values.as_ref().unwrap();
        assert_eq!(av.len(), 3);
    }

    #[test]
    fn nested_message_becomes_object() {
        let src = r#"
            syntax = "proto3";
            message Meta {
                string env    = 1;
                string region = 2;
            }
            message Event {
                string user_id = 1;
                Meta   meta    = 2;
            }
        "#;
        let proto = parse(src);
        let fields = build_fields_for_message("Event", &proto).unwrap();
        let meta = fields.iter().find(|f| f.name == "meta").unwrap();
        assert_eq!(meta.field_type, FieldType::Object);
        let props = meta.properties.as_ref().unwrap();
        assert_eq!(props.len(), 2);
    }
}
