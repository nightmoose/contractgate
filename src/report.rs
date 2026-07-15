//! RFC-082 — exportable pilot report.
//!
//! `GET /contracts/{id}/report?from=&to=&format=json|csv` — an org-scoped,
//! windowed "here's what we caught" report for a single contract: pass rate,
//! per-version split, and top violations. JSON (default) or downloadable CSV.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::storage;
use crate::{AppState, OrgId};

type Ts = chrono::DateTime<chrono::Utc>;

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    pub from: Option<Ts>,
    pub to: Option<Ts>,
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Window {
    pub from: Option<Ts>,
    pub to: Option<Ts>,
}

#[derive(Debug, Serialize)]
pub struct Totals {
    pub total: i64,
    pub passed: i64,
    pub quarantined: i64,
    pub pass_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct VersionBreakdown {
    pub contract_version: Option<String>,
    pub total: i64,
    pub passed: i64,
    pub quarantined: i64,
}

#[derive(Debug, Serialize)]
pub struct ViolationBreakdown {
    pub field: Option<String>,
    pub kind: Option<String>,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct PilotReport {
    pub contract_id: Uuid,
    pub contract_name: String,
    pub window: Window,
    pub generated_at: Ts,
    pub totals: Totals,
    pub by_version: Vec<VersionBreakdown>,
    pub top_violations: Vec<ViolationBreakdown>,
}

/// `GET /contracts/{id}/report`
pub async fn report_handler(
    State(state): State<Arc<AppState>>,
    OrgId(org_id): OrgId,
    Path(contract_id): Path<Uuid>,
    Query(q): Query<ReportQuery>,
) -> AppResult<Response> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    // Org scope: 404 if the contract isn't the caller's.
    let identity = storage::get_contract_identity(&state.db, contract_id, org_id).await?;

    let (versions, violations) = tokio::try_join!(
        storage::report_by_version(&state.db, contract_id, q.from, q.to),
        storage::report_top_violations(&state.db, contract_id, q.from, q.to),
    )?;

    let mut total = 0i64;
    let mut passed = 0i64;
    let mut quarantined = 0i64;
    let by_version: Vec<VersionBreakdown> = versions
        .into_iter()
        .map(|r| {
            total += r.total;
            passed += r.passed;
            quarantined += r.quarantined;
            VersionBreakdown {
                contract_version: r.contract_version,
                total: r.total,
                passed: r.passed,
                quarantined: r.quarantined,
            }
        })
        .collect();
    let pass_rate = if total > 0 {
        passed as f64 / total as f64
    } else {
        0.0
    };
    let top_violations: Vec<ViolationBreakdown> = violations
        .into_iter()
        .map(|r| ViolationBreakdown {
            field: r.field,
            kind: r.kind,
            count: r.count,
        })
        .collect();

    let report = PilotReport {
        contract_id,
        contract_name: identity.name,
        window: Window {
            from: q.from,
            to: q.to,
        },
        generated_at: chrono::Utc::now(),
        totals: Totals {
            total,
            passed,
            quarantined,
            pass_rate,
        },
        by_version,
        top_violations,
    };

    match q.format.as_deref() {
        Some("csv") => Ok(csv_response(&report)),
        None | Some("json") => Ok(Json(report).into_response()),
        Some(other) => Err(AppError::BadRequest(format!(
            "unsupported format '{other}' (use json or csv)"
        ))),
    }
}

fn csv_response(r: &PilotReport) -> Response {
    let mut csv = String::with_capacity(2048);

    csv.push_str("# TOTALS\n");
    csv.push_str("contract_name,contract_id,from,to,total,passed,quarantined,pass_rate\n");
    csv.push_str(&format!(
        "{},{},{},{},{},{},{},{:.4}\n",
        escape_csv(&r.contract_name),
        r.contract_id,
        r.window.from.map(|t| t.to_rfc3339()).unwrap_or_default(),
        r.window.to.map(|t| t.to_rfc3339()).unwrap_or_default(),
        r.totals.total,
        r.totals.passed,
        r.totals.quarantined,
        r.totals.pass_rate,
    ));

    csv.push_str("\n# BY VERSION\ncontract_version,total,passed,quarantined\n");
    for v in &r.by_version {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            escape_csv(v.contract_version.as_deref().unwrap_or("")),
            v.total,
            v.passed,
            v.quarantined,
        ));
    }

    csv.push_str("\n# TOP VIOLATIONS\nfield,kind,count\n");
    for v in &r.top_violations {
        csv.push_str(&format!(
            "{},{},{}\n",
            escape_csv(v.field.as_deref().unwrap_or("")),
            escape_csv(v.kind.as_deref().unwrap_or("")),
            v.count,
        ));
    }

    let filename = format!("pilot-report-{}.csv", r.contract_name);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"").parse().unwrap(),
    );
    (StatusCode::OK, headers, csv).into_response()
}

/// RFC 4180 minimal CSV escaping (mirrors `scorecard::escape_csv`).
fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PilotReport {
        PilotReport {
            contract_id: Uuid::nil(),
            contract_name: "hero_events".into(),
            window: Window {
                from: None,
                to: None,
            },
            generated_at: chrono::Utc::now(),
            totals: Totals {
                total: 100,
                passed: 96,
                quarantined: 4,
                pass_rate: 0.96,
            },
            by_version: vec![VersionBreakdown {
                contract_version: Some("1.0.0".into()),
                total: 100,
                passed: 96,
                quarantined: 4,
            }],
            top_violations: vec![ViolationBreakdown {
                field: Some("method".into()),
                kind: Some("invalid_enum".into()),
                count: 4,
            }],
        }
    }

    /// Lock the JSON wire shape so a rename can't silently break a client.
    #[test]
    fn pilot_report_wire_shape() {
        let v = serde_json::to_value(sample()).unwrap();
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> = [
            "contract_id",
            "contract_name",
            "window",
            "generated_at",
            "totals",
            "by_version",
            "top_violations",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected);
        // Totals sub-shape.
        let tkeys: std::collections::BTreeSet<&str> = v["totals"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            tkeys,
            ["total", "passed", "quarantined", "pass_rate"]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn csv_has_all_three_sections() {
        let resp = csv_response(&sample());
        assert_eq!(resp.status(), StatusCode::OK);
        // escape_csv passthrough for a plain value.
        assert_eq!(escape_csv("method"), "method");
        assert_eq!(escape_csv("a,b"), "\"a,b\"");
    }
}
