//! CSV contract inference — derive a draft `Contract` from CSV content.
//!
//! `POST /contracts/infer/csv`
//!
//! Accepts a CSV body (as a UTF-8 string or base64-encoded) and returns a
//! YAML contract describing its shape.  The first row must be the header.
//! Type inference reuses the same engine as `POST /contracts/infer` (JSON).
//!
//! ## Delimiter handling
//!
//! Auto-detects comma, semicolon, and tab by scoring column-count consistency
//! across the first 20 rows.  Caller may override via the `delimiter` field.
//!
//! ## Type coercion order (per column)
//!
//! CSV values are strings on the wire; this module coerces them before handing
//! off to the shared inference engine:
//!
//! | Wire value                | Coerced to        |
//! |---------------------------|-------------------|
//! | empty / whitespace-only   | `null` (absent)   |
//! | `true` / `false` (any case) | boolean         |
//! | Parses as `i64`           | integer           |
//! | Parses as `f64`           | float             |
//! | Anything else             | string            |
//!
//! String-level pattern/enum detection (UUID, ISO date, enum) then runs
//! inside `infer_fields_from_objects_pub` exactly as for JSON samples.

use crate::contract::{Contract, EgressLeakageMode, Ontology};
use crate::error::{AppError, AppResult};
use crate::infer::{infer_fields_from_objects_pub, InferResponse};
use axum::Json;
use base64::Engine as _;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_CSV_BYTES: usize = 10 * 1024 * 1024; // 10 MB
const MAX_SAMPLE_ROWS: usize = 1_000;
const SNIFF_LINES: usize = 20;
const SNIFF_BYTES: usize = 4_096;

// ---------------------------------------------------------------------------
// Request / response
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct InferCsvRequest {
    /// Name for the generated contract.
    pub name: String,
    /// Optional description embedded in the contract.
    #[serde(default)]
    pub description: Option<String>,
    /// Raw CSV as a UTF-8 string.  One of `csv_content` or `base64` is required.
    #[serde(default)]
    pub csv_content: Option<String>,
    /// Base64-encoded CSV for binary-safe transport.
    #[serde(default)]
    pub base64: Option<String>,
    /// Override delimiter auto-detection.  Accepts `","`, `";"`, `"\t"`, or `"tab"`.
    #[serde(default)]
    pub delimiter: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn infer_csv_handler(Json(req): Json<InferCsvRequest>) -> AppResult<Json<InferResponse>> {
    // 1. Resolve raw CSV string.
    let raw = resolve_content(&req)?;

    if raw.is_empty() {
        return Err(AppError::BadRequest("CSV body is empty".into()));
    }
    if raw.len() > MAX_CSV_BYTES {
        return Err(AppError::BadRequest(format!(
            "CSV too large (max {} MB)",
            MAX_CSV_BYTES / 1024 / 1024
        )));
    }

    // 2. Determine delimiter.
    let delim = match &req.delimiter {
        Some(d) => parse_delimiter(d)?,
        None => detect_delimiter(raw.as_bytes()).ok_or_else(|| {
            AppError::BadRequest(
                "could not detect delimiter; pass delimiter field explicitly".into(),
            )
        })?,
    };

    // 3. Parse CSV → JSON objects.
    let rows = parse_csv(raw.as_bytes(), delim)?;
    if rows.is_empty() {
        return Err(AppError::BadRequest("CSV contains no data rows".into()));
    }

    let sample_count = rows.len();

    // 4. Run shared inference engine.
    let entities = infer_fields_from_objects_pub(&rows);
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
        sample_count,
    }))
}

// ---------------------------------------------------------------------------
// Content resolution
// ---------------------------------------------------------------------------

fn resolve_content(req: &InferCsvRequest) -> AppResult<String> {
    match (&req.csv_content, &req.base64) {
        (Some(s), _) => Ok(s.clone()),
        (None, Some(b)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b)
                .map_err(|e| AppError::BadRequest(format!("invalid base64: {e}")))?;
            String::from_utf8(bytes)
                .map_err(|_| AppError::BadRequest("decoded base64 is not valid UTF-8".into()))
        }
        (None, None) => Err(AppError::BadRequest(
            "one of csv_content or base64 is required".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Delimiter detection
// ---------------------------------------------------------------------------

/// Auto-detect delimiter by scoring column-count consistency across up to
/// `SNIFF_LINES` lines sampled from the first `SNIFF_BYTES` of the file.
/// Returns `None` only when no candidate delimiter ever appears.
fn detect_delimiter(data: &[u8]) -> Option<u8> {
    let snippet = &data[..data.len().min(SNIFF_BYTES)];
    let text = std::str::from_utf8(snippet).ok()?;
    let lines: Vec<&str> = text.lines().take(SNIFF_LINES).collect();
    if lines.len() < 2 {
        return None;
    }

    // Tie-break order: comma > tab > semicolon.
    let candidates: &[(u8, u32)] = &[(b',', 3), (b'\t', 2), (b';', 1)];
    let mut best: Option<(u8, usize, u32)> = None; // (delim, consistent_rows, tiebreak)

    for &(delim, tiebreak) in candidates {
        let counts: Vec<usize> = lines
            .iter()
            .map(|l| l.bytes().filter(|&b| b == delim).count())
            .collect();

        let max_count = *counts.iter().max().unwrap_or(&0);
        if max_count == 0 {
            continue; // delimiter never appears
        }

        let mode = most_common(&counts);
        let consistent = counts.iter().filter(|&&c| c == mode).count();

        let is_better = match &best {
            None => true,
            Some((_, best_consistent, best_tie)) => {
                consistent > *best_consistent
                    || (consistent == *best_consistent && tiebreak > *best_tie)
            }
        };
        if is_better {
            best = Some((delim, consistent, tiebreak));
        }
    }

    best.map(|(d, _, _)| d)
}

fn most_common(counts: &[usize]) -> usize {
    let mut freq: HashMap<usize, usize> = HashMap::new();
    for &c in counts {
        *freq.entry(c).or_insert(0) += 1;
    }
    freq.into_iter()
        .max_by_key(|(_, v)| *v)
        .map(|(k, _)| k)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Delimiter parsing
// ---------------------------------------------------------------------------

fn parse_delimiter(s: &str) -> AppResult<u8> {
    match s {
        "," => Ok(b','),
        ";" => Ok(b';'),
        "\t" | "\\t" | "tab" => Ok(b'\t'),
        other if other.len() == 1 => Ok(other.as_bytes()[0]),
        _ => Err(AppError::BadRequest(format!(
            "unsupported delimiter {s:?}; use \",\", \";\", \"\\t\", or \"tab\""
        ))),
    }
}

// ---------------------------------------------------------------------------
// CSV parsing
// ---------------------------------------------------------------------------

/// Parse CSV bytes into a vec of JSON objects (one per row), capped at
/// `MAX_SAMPLE_ROWS`.  Returns `AppError::BadRequest` on malformed input or
/// duplicate column names.
fn parse_csv(data: &[u8], delimiter: u8) -> AppResult<Vec<Value>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(false)
        .trim(csv::Trim::Fields)
        .from_reader(data);

    // Clone headers immediately — the borrow checker requires it before we
    // call `records()`.
    let headers = rdr
        .headers()
        .map_err(|e| AppError::BadRequest(format!("failed to read CSV headers: {e}")))?
        .clone();

    // Reject duplicate column names.
    let mut seen: HashSet<&str> = HashSet::new();
    for h in headers.iter() {
        if !seen.insert(h) {
            return Err(AppError::BadRequest(format!("duplicate column: {h}")));
        }
    }

    let header_vec: Vec<&str> = headers.iter().collect();

    let mut rows: Vec<Value> = Vec::new();
    for (i, result) in rdr.records().enumerate() {
        if i >= MAX_SAMPLE_ROWS {
            break;
        }
        let record = result
            .map_err(|e| AppError::BadRequest(format!("CSV parse error at row {}: {e}", i + 2)))?;

        let mut obj = serde_json::Map::new();
        for (header, field) in header_vec.iter().zip(record.iter()) {
            obj.insert(header.to_string(), coerce_csv_value(field));
        }
        rows.push(Value::Object(obj));
    }

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Value coercion
// ---------------------------------------------------------------------------

/// Coerce a raw CSV field string into the most specific JSON value.
/// Empty/whitespace → `null` so the inference engine treats it as absent.
fn coerce_csv_value(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::Null;
    }
    match t.to_ascii_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = t.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(t.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- delimiter detection ------------------------------------------------

    #[test]
    fn detects_comma() {
        let csv = "a,b,c\n1,2,3\n4,5,6\n";
        assert_eq!(detect_delimiter(csv.as_bytes()), Some(b','));
    }

    #[test]
    fn detects_semicolon() {
        let csv = "a;b;c\n1;2;3\n4;5;6\n";
        assert_eq!(detect_delimiter(csv.as_bytes()), Some(b';'));
    }

    #[test]
    fn detects_tab() {
        let csv = "a\tb\tc\n1\t2\t3\n4\t5\t6\n";
        assert_eq!(detect_delimiter(csv.as_bytes()), Some(b'\t'));
    }

    #[test]
    fn comma_wins_tie() {
        // Both comma and semicolon appear equally — comma should win.
        let csv = "a,b;c\n1,2;3\n4,5;6\n";
        assert_eq!(detect_delimiter(csv.as_bytes()), Some(b','));
    }

    // ---- delimiter parsing --------------------------------------------------

    #[test]
    fn parse_delimiter_tab_aliases() {
        assert_eq!(parse_delimiter("\t").unwrap(), b'\t');
        assert_eq!(parse_delimiter("\\t").unwrap(), b'\t');
        assert_eq!(parse_delimiter("tab").unwrap(), b'\t');
    }

    #[test]
    fn parse_delimiter_rejects_multi_char() {
        assert!(parse_delimiter(",,").is_err());
    }

    // ---- value coercion ----------------------------------------------------

    #[test]
    fn coerces_empty_to_null() {
        assert_eq!(coerce_csv_value(""), Value::Null);
        assert_eq!(coerce_csv_value("   "), Value::Null);
    }

    #[test]
    fn coerces_booleans() {
        assert_eq!(coerce_csv_value("true"), Value::Bool(true));
        assert_eq!(coerce_csv_value("TRUE"), Value::Bool(true));
        assert_eq!(coerce_csv_value("false"), Value::Bool(false));
    }

    #[test]
    fn coerces_integer() {
        assert_eq!(coerce_csv_value("42"), Value::Number(42i64.into()));
    }

    #[test]
    fn coerces_float() {
        let v = coerce_csv_value("3.14");
        assert!(matches!(v, Value::Number(_)));
    }

    #[test]
    fn coerces_string_fallback() {
        assert_eq!(
            coerce_csv_value("hello"),
            Value::String("hello".to_string())
        );
    }

    // ---- CSV parsing -------------------------------------------------------

    #[test]
    fn parses_basic_csv() {
        let csv = "id,name,score\n1,alice,9.5\n2,bob,8.0\n";
        let rows = parse_csv(csv.as_bytes(), b',').unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], Value::Number(1i64.into()));
        assert_eq!(rows[0]["name"], Value::String("alice".to_string()));
    }

    #[test]
    fn rejects_duplicate_headers() {
        let csv = "id,name,id\n1,alice,1\n";
        assert!(parse_csv(csv.as_bytes(), b',').is_err());
    }

    #[test]
    fn treats_empty_fields_as_null() {
        let csv = "a,b\n1,\n2,3\n";
        let rows = parse_csv(csv.as_bytes(), b',').unwrap();
        assert_eq!(rows[0]["b"], Value::Null);
        assert_eq!(rows[1]["b"], Value::Number(3i64.into()));
    }

    #[test]
    fn caps_at_max_sample_rows() {
        // Build a CSV with MAX_SAMPLE_ROWS + 10 data rows.
        let header = "x\n";
        let data_rows = (0..MAX_SAMPLE_ROWS + 10)
            .map(|i| format!("{i}\n"))
            .collect::<String>();
        let csv = format!("{header}{data_rows}");
        let rows = parse_csv(csv.as_bytes(), b',').unwrap();
        assert_eq!(rows.len(), MAX_SAMPLE_ROWS);
    }

    // ---- inference integration ---------------------------------------------

    #[test]
    fn infers_types_from_csv() {
        let csv = "user_id,amount,active\nu1,10,true\nu2,20,false\n";
        let rows = parse_csv(csv.as_bytes(), b',').unwrap();
        let fields = infer_fields_from_objects_pub(&rows);
        let by_name: HashMap<&str, _> = fields.iter().map(|f| (f.name.as_str(), f)).collect();

        assert_eq!(
            by_name["user_id"].field_type,
            crate::contract::FieldType::String
        );
        assert_eq!(
            by_name["amount"].field_type,
            crate::contract::FieldType::Integer
        );
        assert_eq!(
            by_name["active"].field_type,
            crate::contract::FieldType::Boolean
        );
    }

    #[test]
    fn optional_when_empty_field() {
        let csv = "x,y\n1,\n2,3\n";
        let rows = parse_csv(csv.as_bytes(), b',').unwrap();
        let fields = infer_fields_from_objects_pub(&rows);
        let by_name: HashMap<&str, _> = fields.iter().map(|f| (f.name.as_str(), f)).collect();
        assert!(by_name["x"].required);
        assert!(!by_name["y"].required);
    }
}
