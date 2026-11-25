//! Readiness Review: infer a draft contract from a CSV, validate every row,
//! and write a branded report (RFC-style sales-call kit).
//!
//! Reuses `infer::infer_fields_from_objects_pub` for the draft contract and
//! `scaffold::pii::detect_pii` to flag fields that should be masked in the
//! report. Validation is the same engine ingest uses.

use crate::contract::{Contract, EgressLeakageMode, Ontology};
use crate::infer::{infer_fields_from_objects_pub, samples_have_nulls};
use crate::scaffold::pii::{detect_pii, PiiCandidate};
use crate::validation::{validate, CompiledContract, Violation};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

/// Default PII detection threshold, matching `scaffold::ScaffoldConfig`.
const PII_THRESHOLD: f32 = 0.4;

/// Infer from at most this many rows (same cap as CSV inference). Every
/// row is still validated.
const INFER_SAMPLE_CAP: usize = 1_000;

#[derive(Debug, Clone)]
pub struct ReviewRow {
    /// 1-based data row number (the header is not counted).
    pub row_number: usize,
    pub values: Value,
    pub violations: Vec<Violation>,
}

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub contract: Contract,
    pub contract_yaml: String,
    pub pii: Vec<PiiCandidate>,
    pub rows_total: usize,
    pub rows_passed: usize,
    pub rows_rejected: usize,
    pub rejected: Vec<ReviewRow>,
    pub top_rules: Vec<(String, usize)>,
}

#[derive(Debug)]
pub struct ReviewError(pub String);

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ReviewError {}

/// Parse `csv_text`, infer a draft contract named `name`, and validate every
/// row. `description` is optional contract metadata.
pub fn review_csv(
    csv_text: &str,
    name: &str,
    description: Option<&str>,
) -> Result<ReviewResult, ReviewError> {
    let rows = parse_csv(csv_text.as_bytes())?;
    if rows.is_empty() {
        return Err(ReviewError("CSV contains no data rows".into()));
    }

    // Infer from rows that match the majority type of each column, so
    // obvious outliers (a bad email in a column of emails, a negative
    // amount in a column of non-negative numbers) are validated against
    // a draft that still describes the core of the file. Fall back to
    // every row when that filter would leave too little to infer from.
    let infer_owned = rows_for_inference(&rows);
    let infer_sample = if infer_owned.len() > INFER_SAMPLE_CAP {
        &infer_owned[..INFER_SAMPLE_CAP]
    } else {
        &infer_owned[..]
    };
    let entities = infer_fields_from_objects_pub(infer_sample);
    let field_names: Vec<String> = entities.iter().map(|f| f.name.clone()).collect();
    let pii = detect_pii(&field_names, infer_sample, PII_THRESHOLD);

    let contract = Contract {
        version: "1.0".to_string(),
        name: name.to_string(),
        description: description.map(str::to_string).or_else(|| {
            Some(format!(
                "Draft contract inferred from {n} CSV row(s) by `contractgate review`.",
                n = infer_sample.len()
            ))
        }),
        compliance_mode: false,
        egress_leakage_mode: EgressLeakageMode::Off,
        ontology: Ontology { entities },
        glossary: vec![],
        metrics: vec![],
        quality: vec![],
        envelope: None,
        null_as_absent: samples_have_nulls(infer_sample),
        on_fail: crate::contract::OnFailMode::Quarantine,
        on_fail_locked: false,
        quarantine_guard: crate::contract::QuarantineGuardConfig::default(),
        cross_field: Vec::new(),
    };

    let contract_yaml = serde_yaml::to_string(&contract)
        .map_err(|e| ReviewError(format!("serializing the draft contract: {e}")))?;

    let compiled = CompiledContract::compile(contract.clone())
        .map_err(|e| ReviewError(format!("compiling the draft contract: {e:#}")))?;

    let mut rejected = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let result = validate(&compiled, row);
        if !result.passed {
            rejected.push(ReviewRow {
                row_number: i + 1,
                values: row.clone(),
                violations: result.violations,
            });
        }
    }

    let mut rule_counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rejected {
        let mut seen = HashSet::new();
        for v in &row.violations {
            let key = format!("{} ({})", v.field, kind_label(&v.kind));
            if seen.insert(key.clone()) {
                *rule_counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    let mut top_rules: Vec<(String, usize)> = rule_counts.into_iter().collect();
    top_rules.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let rows_total = rows.len();
    let rows_rejected = rejected.len();
    Ok(ReviewResult {
        contract,
        contract_yaml,
        pii,
        rows_total,
        rows_passed: rows_total - rows_rejected,
        rows_rejected,
        rejected,
        top_rules,
    })
}

/// Write `contract.yaml`, `report.html`, and `rejected.csv` under `out_dir`.
pub fn write_review_outputs(out_dir: &Path, review: &ReviewResult) -> Result<(), ReviewError> {
    fs::create_dir_all(out_dir)
        .map_err(|e| ReviewError(format!("creating {}: {e}", out_dir.display())))?;

    let contract_path = out_dir.join("contract.yaml");
    fs::write(&contract_path, &review.contract_yaml)
        .map_err(|e| ReviewError(format!("writing {}: {e}", contract_path.display())))?;

    let report_path = out_dir.join("report.html");
    fs::write(&report_path, render_report_html(review))
        .map_err(|e| ReviewError(format!("writing {}: {e}", report_path.display())))?;

    let rejected_path = out_dir.join("rejected.csv");
    write_rejected_csv(&rejected_path, review)
        .map_err(|e| ReviewError(format!("writing {}: {e}", rejected_path.display())))?;

    Ok(())
}

pub fn render_report_html(review: &ReviewResult) -> String {
    let pii_names: HashSet<&str> = review.pii.iter().map(|p| p.field_name.as_str()).collect();
    let pass_pct = if review.rows_total == 0 {
        0.0
    } else {
        (review.rows_passed as f64 / review.rows_total as f64) * 100.0
    };

    let mut pii_rows = String::new();
    if review.pii.is_empty() {
        pii_rows.push_str("<tr><td colspan=\"3\">No PII fields flagged.</td></tr>");
    } else {
        for p in &review.pii {
            pii_rows.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{:.0}%</td><td>{}</td></tr>",
                esc(&p.field_name),
                p.confidence * 100.0,
                esc(&p.reason)
            ));
        }
    }

    let mut rule_rows = String::new();
    if review.top_rules.is_empty() {
        rule_rows.push_str("<tr><td colspan=\"2\">No failing rules.</td></tr>");
    } else {
        for (rule, n) in review.top_rules.iter().take(10) {
            rule_rows.push_str(&format!(
                "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(rule),
                n
            ));
        }
    }

    let sample_n = review.rejected.len().min(25);
    let mut sample_rows = String::new();
    if sample_n == 0 {
        sample_rows.push_str("<p class=\"ok\">No rejected rows.</p>");
    } else {
        sample_rows.push_str("<table><thead><tr><th>Row</th><th>Reasons</th><th>Values (PII masked)</th></tr></thead><tbody>");
        for row in review.rejected.iter().take(sample_n) {
            let reasons: Vec<String> = row
                .violations
                .iter()
                .map(|v| format!("{}: {}", v.field, v.message))
                .collect();
            sample_rows.push_str(&format!(
                "<tr><td class=\"num\">{}</td><td>{}</td><td><pre>{}</pre></td></tr>",
                row.row_number,
                esc(&reasons.join("; ")),
                esc(&format_row_masked(&row.values, &pii_names))
            ));
        }
        sample_rows.push_str("</tbody></table>");
        if review.rejected.len() > sample_n {
            sample_rows.push_str(&format!(
                "<p class=\"note\">Showing {sample_n} of {} rejected rows. The full set is in rejected.csv (PII masked).</p>",
                review.rejected.len()
            ));
        }
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DataContractGate Readiness Review: {name}</title>
<style>
  :root {{ --ink:#102033; --muted:#5b6b7c; --line:#d7dee6; --bg:#f6f8fa; --card:#fff; --accent:#0f6e6a; --bad:#9b2c2c; --ok:#1f7a4d; }}
  * {{ box-sizing: border-box; }}
  body {{ margin:0; font: 16px/1.5 ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif; color:var(--ink); background:var(--bg); }}
  header {{ background:var(--ink); color:#fff; padding:28px 32px; }}
  header .brand {{ letter-spacing:.04em; font-size:12px; text-transform:uppercase; opacity:.8; }}
  header h1 {{ margin:6px 0 0; font-size:28px; font-weight:650; }}
  header p {{ margin:8px 0 0; opacity:.85; }}
  main {{ max-width:960px; margin:0 auto; padding:24px 20px 64px; }}
  .cards {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(160px,1fr)); gap:12px; margin:20px 0 28px; }}
  .card {{ background:var(--card); border:1px solid var(--line); border-radius:10px; padding:16px 18px; }}
  .card .label {{ color:var(--muted); font-size:12px; text-transform:uppercase; letter-spacing:.04em; }}
  .card .value {{ font-size:28px; font-weight:650; margin-top:4px; }}
  .card .value.bad {{ color:var(--bad); }}
  .card .value.ok {{ color:var(--ok); }}
  h2 {{ font-size:18px; margin:28px 0 10px; }}
  table {{ width:100%; border-collapse:collapse; background:var(--card); border:1px solid var(--line); border-radius:10px; overflow:hidden; }}
  th, td {{ text-align:left; padding:10px 12px; border-bottom:1px solid var(--line); vertical-align:top; font-size:14px; }}
  th {{ background:#eef2f6; font-weight:600; }}
  td.num, th.num {{ text-align:right; font-variant-numeric:tabular-nums; }}
  pre {{ margin:0; white-space:pre-wrap; font:12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; }}
  code {{ font:12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; }}
  .ok {{ color:var(--ok); }}
  .note {{ color:var(--muted); font-size:13px; }}
  footer {{ margin-top:36px; color:var(--muted); font-size:13px; }}
</style>
</head>
<body>
<header>
  <div class="brand">DataContractGate</div>
  <h1>Readiness Review</h1>
  <p>Draft contract <strong>{name}</strong> inferred and checked against every CSV row. PII fields are flagged and masked below.</p>
</header>
<main>
  <section class="cards">
    <div class="card"><div class="label">Rows checked</div><div class="value">{total}</div></div>
    <div class="card"><div class="label">Passed</div><div class="value ok">{passed}</div></div>
    <div class="card"><div class="label">Rejected</div><div class="value {rej_class}">{rejected}</div></div>
    <div class="card"><div class="label">Pass rate</div><div class="value">{pct:.1}%</div></div>
  </section>

  <h2>Top failing rules</h2>
  <table><thead><tr><th>Rule</th><th class="num">Rejected rows</th></tr></thead><tbody>
  {rule_rows}
  </tbody></table>

  <h2>PII fields flagged</h2>
  <table><thead><tr><th>Field</th><th>Confidence</th><th>Why</th></tr></thead><tbody>
  {pii_rows}
  </tbody></table>

  <h2>Sample rejected rows</h2>
  {sample_rows}

  <footer>
    Generated by <strong>DataContractGate</strong> <code>contractgate review</code>.
    The draft contract is a starting point; review it before you promote it.
    Files written alongside this report: contract.yaml and rejected.csv.
  </footer>
</main>
</body>
</html>
"##,
        name = esc(&review.contract.name),
        total = review.rows_total,
        passed = review.rows_passed,
        rejected = review.rows_rejected,
        rej_class = if review.rows_rejected == 0 {
            "ok"
        } else {
            "bad"
        },
        pct = pass_pct,
        rule_rows = rule_rows,
        pii_rows = pii_rows,
        sample_rows = sample_rows,
    )
}

fn write_rejected_csv(path: &Path, review: &ReviewResult) -> Result<(), std::io::Error> {
    let pii_names: HashSet<&str> = review.pii.iter().map(|p| p.field_name.as_str()).collect();
    let headers = field_order(review);
    let mut writer = csv::Writer::from_path(path)?;
    let mut head: Vec<String> = vec!["_row".into(), "_reasons".into()];
    head.extend(headers.iter().cloned());
    writer.write_record(&head)?;
    for row in &review.rejected {
        let reasons = row
            .violations
            .iter()
            .map(|v| format!("{}: {}", v.field, v.message))
            .collect::<Vec<_>>()
            .join("; ");
        let mut rec: Vec<String> = vec![row.row_number.to_string(), reasons];
        let obj = row.values.as_object();
        for h in &headers {
            let raw = obj.and_then(|o| o.get(h)).cloned().unwrap_or(Value::Null);
            rec.push(cell_for_csv(h, &raw, &pii_names));
        }
        writer.write_record(&rec)?;
    }
    writer.flush()?;
    Ok(())
}

fn field_order(review: &ReviewResult) -> Vec<String> {
    review
        .contract
        .ontology
        .entities
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

fn cell_for_csv(field: &str, value: &Value, pii: &HashSet<&str>) -> String {
    if value.is_null() {
        return String::new();
    }
    let rendered = match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    };
    if pii.contains(field) {
        mask_pii(&rendered)
    } else {
        rendered
    }
}

fn format_row_masked(value: &Value, pii: &HashSet<&str>) -> String {
    let Some(obj) = value.as_object() else {
        return value.to_string();
    };
    let mut out = String::new();
    for (k, v) in obj {
        let shown = if v.is_null() {
            "null".into()
        } else if pii.contains(k.as_str()) {
            mask_pii(&value_as_text(v))
        } else {
            value_as_text(v)
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{k}: {shown}"));
    }
    out
}

fn value_as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Mask a PII value for display. Emails keep a hint of the shape; other
/// strings keep the first and last character when long enough.
pub fn mask_pii(value: &str) -> String {
    if let Some((user, domain)) = value.split_once('@') {
        let user_m = mask_token(user);
        let domain_m = match domain.rsplit_once('.') {
            Some((host, tld)) => format!("{}.{}", mask_token(host), tld),
            None => mask_token(domain),
        };
        return format!("{user_m}@{domain_m}");
    }
    mask_token(value)
}

fn mask_token(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    match chars.len() {
        0 => String::new(),
        1 => "*".into(),
        2 => format!("{}*", chars[0]),
        _ => format!("{}***{}", chars[0], chars[chars.len() - 1]),
    }
}

fn kind_label(kind: &crate::validation::ViolationKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{kind:?}"))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Keep rows whose non-null values match the majority type of each field.
/// If fewer than 20% of rows (or fewer than 3) would remain, return all rows.
fn rows_for_inference(rows: &[Value]) -> Vec<Value> {
    let mut field_counts: BTreeMap<String, BTreeMap<ValueClass, usize>> = BTreeMap::new();
    for row in rows {
        let Some(obj) = row.as_object() else { continue };
        for (k, v) in obj {
            if v.is_null() {
                continue;
            }
            let class = classify_value(v);
            *field_counts
                .entry(k.clone())
                .or_default()
                .entry(class)
                .or_insert(0) += 1;
        }
    }
    let majority: BTreeMap<String, ValueClass> = field_counts
        .into_iter()
        .filter_map(|(k, counts)| {
            counts
                .into_iter()
                .max_by_key(|(_, n)| *n)
                .map(|(class, _)| (k, class))
        })
        .collect();

    let kept: Vec<Value> = rows
        .iter()
        .filter(|row| row_matches_majority(row, &majority))
        .cloned()
        .collect();
    let min_keep = 3.max(rows.len().div_ceil(5)); // 20%, at least 3
    if kept.len() >= min_keep {
        kept
    } else {
        rows.to_vec()
    }
}

fn row_matches_majority(row: &Value, majority: &BTreeMap<String, ValueClass>) -> bool {
    let Some(obj) = row.as_object() else {
        return false;
    };
    for (k, v) in obj {
        if v.is_null() {
            continue;
        }
        if let Some(class) = majority.get(k) {
            if classify_value(v) != *class {
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ValueClass {
    Bool,
    Integer,
    Float,
    Date,
    Timestamp,
    Uuid,
    Email,
    String,
}

fn classify_value(v: &Value) -> ValueClass {
    match v {
        Value::Bool(_) => ValueClass::Bool,
        Value::Number(n) if n.is_i64() || n.is_u64() => ValueClass::Integer,
        Value::Number(_) => ValueClass::Float,
        Value::String(s) => classify_string(s),
        _ => ValueClass::String,
    }
}

fn classify_string(s: &str) -> ValueClass {
    let t = s.trim();
    if looks_like_email(t) {
        return ValueClass::Email;
    }
    if looks_like_uuid(t) {
        return ValueClass::Uuid;
    }
    if looks_like_timestamp(t) {
        return ValueClass::Timestamp;
    }
    if looks_like_date(t) {
        return ValueClass::Date;
    }
    ValueClass::String
}

fn looks_like_email(s: &str) -> bool {
    let Some((user, domain)) = s.split_once('@') else {
        return false;
    };
    !user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
}

fn looks_like_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && s[0..4].bytes().all(|c| c.is_ascii_digit())
        && s[5..7].bytes().all(|c| c.is_ascii_digit())
        && s[8..10].bytes().all(|c| c.is_ascii_digit())
}

fn looks_like_timestamp(s: &str) -> bool {
    s.len() >= 19
        && looks_like_date(&s[..10])
        && (s.as_bytes()[10] == b'T' || s.as_bytes()[10] == b' ')
}

fn parse_csv(data: &[u8]) -> Result<Vec<Value>, ReviewError> {
    let delim = detect_delimiter(data).unwrap_or(b',');
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .flexible(false)
        .trim(csv::Trim::Fields)
        .from_reader(data);
    let headers = rdr
        .headers()
        .map_err(|e| ReviewError(format!("failed to read CSV headers: {e}")))?
        .clone();
    let mut seen: HashSet<&str> = HashSet::new();
    for h in headers.iter() {
        if !seen.insert(h) {
            return Err(ReviewError(format!("duplicate column: {h}")));
        }
    }
    let header_vec: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
    let mut rows = Vec::new();
    for (i, result) in rdr.records().enumerate() {
        let record =
            result.map_err(|e| ReviewError(format!("CSV parse error at row {}: {e}", i + 2)))?;
        let mut obj = Map::new();
        for (header, field) in header_vec.iter().zip(record.iter()) {
            obj.insert(header.clone(), coerce_csv_value(field));
        }
        rows.push(Value::Object(obj));
    }
    Ok(rows)
}

fn detect_delimiter(data: &[u8]) -> Option<u8> {
    let preview = data.split(|&b| b == b'\n').next().unwrap_or(data);
    let mut scores = [(b',', 0usize), (b';', 0), (b'\t', 0), (b'|', 0)];
    for (d, n) in scores.iter_mut() {
        *n = preview.iter().filter(|b| **b == *d).count();
    }
    scores.sort_by_key(|a| std::cmp::Reverse(a.1));
    if scores[0].1 == 0 {
        None
    } else {
        Some(scores[0].0)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "\
order_id,customer_email,amount,status,created_at
ORD-000001,ada@example.com,12.5,paid,2026-01-02
ORD-000002,bob@example.com,8.0,pending,2026-01-03
ORD-000003,not-an-email, -4,lost,yesterday
ORD-000001,ada@example.com,3.0,paid,2026-01-04
";

    #[test]
    fn review_flags_bad_rows_and_masks_email() {
        let review = review_csv(CSV, "orders", None).expect("review");
        assert!(review.rows_total >= 4);
        assert!(review.rows_rejected >= 1);
        assert!(review.pii.iter().any(|p| p.field_name == "customer_email"));
        let html = render_report_html(&review);
        assert!(html.contains("DataContractGate"));
        assert!(html.contains("Readiness Review"));
        assert!(
            !html.contains("ada@example.com") && !html.contains("bob@example.com"),
            "PII must be masked in the HTML"
        );
        let yaml = &review.contract_yaml;
        assert!(
            yaml.contains("customer_email") && yaml.contains("order_id"),
            "{yaml}"
        );
        // Row 3 is the planted outlier (bad email, negative amount, bad date).
        assert!(
            review.rejected.iter().any(|r| r.row_number == 3),
            "planted outlier row should be rejected: {:?}",
            review
                .rejected
                .iter()
                .map(|r| (
                    r.row_number,
                    r.violations
                        .iter()
                        .map(|v| v.message.clone())
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn mask_pii_keeps_email_shape() {
        let masked = mask_pii("ada@example.com");
        assert!(masked.contains('@'));
        assert!(!masked.contains("ada@example.com"));
        assert!(masked.ends_with(".com"));
    }

    #[test]
    fn empty_csv_is_an_error() {
        let err = review_csv("a,b\n", "t", None).unwrap_err();
        assert!(err.0.contains("no data"));
    }
}
