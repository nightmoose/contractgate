//! Violation report formatters for shadow enforcement (RFC-024 §G).
//!
//! Three sinks: markdown (default), JSON, Prometheus push.
//!
//! Developer tooling — not part of the patent-core validation engine.

use crate::validation::Violation;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Aggregated shadow-enforcement violation report.
#[derive(Debug, Default)]
pub struct ViolationReport {
    pub contract_name: String,
    pub contract_source: String,
    pub total_events: u64,
    pub violated_events: u64,
    /// Per-field per-rule aggregated counts.
    pub field_violations: Vec<FieldViolationSummary>,
}

#[derive(Debug, Clone)]
pub struct FieldViolationSummary {
    pub field: String,
    pub rule: String,
    pub count: u64,
    /// Rate of violation among total events.
    pub rate: f64,
    /// One representative sample value (redacted if > 64 chars).
    pub sample_value: Option<String>,
}

impl ViolationReport {
    pub fn violation_rate(&self) -> f64 {
        if self.total_events == 0 {
            0.0
        } else {
            self.violated_events as f64 / self.total_events as f64
        }
    }

    /// Build a report from a flat list of per-event violation lists.
    pub fn from_violations(
        contract_name: &str,
        contract_source: &str,
        total: u64,
        all_violations: Vec<Vec<Violation>>,
    ) -> Self {
        let violated_events = all_violations.iter().filter(|v| !v.is_empty()).count() as u64;

        // Aggregate by (field, kind).
        let mut agg: HashMap<(String, String), u64> = HashMap::new();
        for per_event in &all_violations {
            for v in per_event {
                // kind → snake_case string via serde
                let kind_str = serde_json::to_string(&v.kind)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string();
                let key = (v.field.clone(), kind_str);
                *agg.entry(key).or_insert(0) += 1;
            }
        }

        let mut field_violations: Vec<FieldViolationSummary> = agg
            .into_iter()
            .map(|((field, rule), count)| FieldViolationSummary {
                field,
                rule,
                count,
                rate: if total > 0 {
                    count as f64 / total as f64
                } else {
                    0.0
                },
                sample_value: None,
            })
            .collect();
        // Sort by count desc for readability.
        field_violations.sort_by_key(|b| std::cmp::Reverse(b.count));

        Self {
            contract_name: contract_name.to_string(),
            contract_source: contract_source.to_string(),
            total_events: total,
            violated_events,
            field_violations,
        }
    }
}

// ---------------------------------------------------------------------------
// Markdown formatter
// ---------------------------------------------------------------------------

pub fn format_markdown(report: &ViolationReport) -> String {
    let mut out = String::new();

    out.push_str("# Shadow Enforcement Report\n\n");
    out.push_str(&format!("**Contract:** {}\n", report.contract_name));
    out.push_str(&format!("**Source:** {}\n", report.contract_source));
    out.push_str(&format!("**Total events:** {}\n", report.total_events));
    out.push_str(&format!(
        "**Violated events:** {} ({:.1}%)\n\n",
        report.violated_events,
        report.violation_rate() * 100.0
    ));

    if report.field_violations.is_empty() {
        out.push_str("✅ No violations detected.\n");
        return out;
    }

    out.push_str("## Violations\n\n");
    out.push_str("| Field | Rule | Count | Rate | Sample Value |\n");
    out.push_str("|-------|------|------:|-----:|-------------|\n");

    for fv in &report.field_violations {
        let sample = fv
            .sample_value
            .as_deref()
            .unwrap_or("-")
            .replace('|', "\\|");
        out.push_str(&format!(
            "| `{}` | `{}` | {} | {:.1}% | `{}` |\n",
            fv.field,
            fv.rule,
            fv.count,
            fv.rate * 100.0,
            sample
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// JSON formatter
// ---------------------------------------------------------------------------

pub fn format_json(report: &ViolationReport) -> String {
    let violations_json: Vec<serde_json::Value> = report
        .field_violations
        .iter()
        .map(|fv| {
            serde_json::json!({
                "field": fv.field,
                "rule": fv.rule,
                "count": fv.count,
                "rate": fv.rate,
                "sample_value": fv.sample_value,
            })
        })
        .collect();

    let root = serde_json::json!({
        "contract": report.contract_name,
        "source": report.contract_source,
        "total_events": report.total_events,
        "violated_events": report.violated_events,
        "violation_rate": report.violation_rate(),
        "violations": violations_json,
    });

    serde_json::to_string_pretty(&root).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Prometheus push (blocking reqwest)
// ---------------------------------------------------------------------------

/// Push violation metrics to a Prometheus Pushgateway.
///
/// Requires `PUSHGATEWAY_URL` or the caller to pass the URL explicitly.
/// On missing URL, returns an `Err` with a clear message — the caller
/// should exit with code 2.
pub fn push_prometheus(report: &ViolationReport, pushgateway_url: &str) -> anyhow::Result<()> {
    use std::fmt::Write as FmtWrite;

    let job = "contractgate_shadow";
    let url = format!("{pushgateway_url}/metrics/job/{job}");

    let mut body = String::new();
    // Gauges: total events, violated events, violation_rate.
    writeln!(
        body,
        "# HELP contractgate_shadow_total_events Total events checked in shadow mode."
    )?;
    writeln!(body, "# TYPE contractgate_shadow_total_events gauge")?;
    writeln!(
        body,
        "contractgate_shadow_total_events{{contract=\"{}\"}} {}",
        report.contract_name, report.total_events
    )?;

    writeln!(
        body,
        "# HELP contractgate_shadow_violation_rate Fraction of events with violations."
    )?;
    writeln!(body, "# TYPE contractgate_shadow_violation_rate gauge")?;
    writeln!(
        body,
        "contractgate_shadow_violation_rate{{contract=\"{}\"}} {:.6}",
        report.contract_name,
        report.violation_rate()
    )?;

    writeln!(
        body,
        "# HELP contractgate_shadow_violations_total Violation count per field and rule."
    )?;
    writeln!(body, "# TYPE contractgate_shadow_violations_total gauge")?;
    for fv in &report.field_violations {
        writeln!(
            body,
            "contractgate_shadow_violations_total{{contract=\"{}\",field=\"{}\",rule=\"{}\"}} {}",
            report.contract_name, fv.field, fv.rule, fv.count
        )?;
    }

    let client = reqwest::blocking::Client::new();
    client
        .put(&url)
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(body)
        .send()
        .map_err(|e| anyhow::anyhow!("Prometheus push failed: {e}"))?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("Prometheus push HTTP error: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_violation(field: &str, rule: &str) -> Violation {
        use crate::validation::ViolationKind;
        Violation {
            field: field.to_string(),
            message: format!("{field} failed {rule}"),
            kind: ViolationKind::TypeMismatch,
        }
    }

    #[test]
    fn violation_rate_zero_for_clean_run() {
        let report = ViolationReport {
            total_events: 100,
            violated_events: 0,
            ..Default::default()
        };
        assert_eq!(report.violation_rate(), 0.0);
    }

    #[test]
    fn violation_rate_calculation() {
        let report = ViolationReport {
            total_events: 100,
            violated_events: 10,
            ..Default::default()
        };
        assert!((report.violation_rate() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn markdown_output_contains_contract_name() {
        let report = ViolationReport {
            contract_name: "my_contract".to_string(),
            total_events: 5,
            ..Default::default()
        };
        let md = format_markdown(&report);
        assert!(md.contains("my_contract"));
        assert!(md.contains("No violations"));
    }

    #[test]
    fn json_output_is_valid_json() {
        let report = ViolationReport::from_violations(
            "test",
            "topic:test",
            10,
            vec![vec![make_violation("user_id", "MISSING_REQUIRED")]],
        );
        let json_str = format_json(&report);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["contract"], "test");
        assert_eq!(parsed["total_events"], 10);
    }

    #[test]
    fn from_violations_aggregates_correctly() {
        let all = vec![
            vec![make_violation("field_a", "TYPE_MISMATCH")],
            vec![make_violation("field_a", "TYPE_MISMATCH")],
            vec![],
        ];
        let report = ViolationReport::from_violations("c", "t", 3, all);
        assert_eq!(report.violated_events, 2);
        assert_eq!(report.field_violations[0].count, 2);
        assert_eq!(report.field_violations[0].field, "field_a");
    }
}
