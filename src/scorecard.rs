//! Provider Data-Quality Scorecard (RFC-031).
//!
//! Three API surfaces:
//!   GET /scorecard/{source}         — full scorecard JSON
//!   GET /scorecard/{source}/drift   — active drift signals only
//!   GET /scorecard/{source}/export  — flat CSV (?format=csv)
//!
//! Reads from SQL views `provider_scorecard` and `provider_field_health`, plus
//! `provider_field_baseline` for drift detection.  No writes to the hot ingest
//! path — the <15ms p99 budget is untouched.
//!
//! ## Drift detection
//!
//! A drift signal fires when a field's current 24-hour violation rate deviates
//! from its trailing 30-day baseline by more than [`DRIFT_THRESHOLD_PCT`]
//! percentage points.  The baseline is populated by the daily
//! [`run_baseline_rollup`] job (`cargo run -- scorecard-rollup`).

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Drift fires when the absolute change from baseline exceeds this many
/// percentage points.  Fixed at 5 pp in v1; make configurable per-contract
/// in a later iteration (RFC-031 Open Question 2).
const DRIFT_THRESHOLD_PCT: f64 = 5.0;

/// Length of the trailing baseline window in days (RFC-031 Open Question 1).
const BASELINE_WINDOW_DAYS: i64 = 30;

// ---------------------------------------------------------------------------
// Row types — map directly to the SQL view columns.
// ---------------------------------------------------------------------------

/// One row from the `provider_scorecard` view.
#[derive(Debug, sqlx::FromRow, Serialize, Clone)]
pub struct ScorecardRow {
    pub source: String,
    pub contract_name: String,
    pub total_events: i64,
    pub passed: i64,
    pub quarantined: i64,
    /// Quarantine rate as a percentage (0.00–100.00), NULL if zero events.
    pub quarantine_pct: Option<f64>,
}

/// One row from the `provider_field_health` view.
#[derive(Debug, sqlx::FromRow, Serialize, Clone)]
pub struct FieldHealthRow {
    pub source: String,
    pub contract_name: String,
    pub field: String,
    /// Violation kind string (e.g. `"missing_required_field"`), aliased from
    /// the `kind` JSON key.
    pub code: String,
    pub violations: i64,
}

/// One row from `provider_field_baseline`.
#[derive(Debug, sqlx::FromRow)]
struct BaselineRow {
    pub field: String,
    pub contract_name: String,
    pub violation_rate: f64,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// An active drift signal — a field whose violation rate has jumped (or
/// dropped) significantly from its trailing 30-day baseline.
#[derive(Debug, Serialize, Clone)]
pub struct DriftSignal {
    pub source: String,
    pub contract_name: String,
    pub field: String,
    /// Violation rate in the last 24 h (0.0–1.0).
    pub current_rate: f64,
    /// Trailing 30-day baseline violation rate (0.0–1.0).
    pub baseline_rate: f64,
    /// Absolute delta in percentage points (positive = more violations).
    pub delta_pct: f64,
    /// Human-readable summary, e.g. `"↑ 12.3 pp since baseline"`.
    pub label: String,
}

/// Full scorecard response for one provider source.
#[derive(Debug, Serialize)]
pub struct ScorecardResponse {
    pub source: String,
    /// Per-contract pass/quarantine summary rows.
    pub summary: Vec<ScorecardRow>,
    /// Top 20 violations ranked by count (subset of `field_health`).
    pub top_violations: Vec<FieldHealthRow>,
    /// Full per-field violation breakdown.
    pub field_health: Vec<FieldHealthRow>,
    /// Active drift signals (fields that moved >5 pp from baseline).
    pub drift_signals: Vec<DriftSignal>,
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

/// Query the `provider_scorecard` view for one source.
pub async fn query_scorecard(pool: &PgPool, source: &str) -> AppResult<Vec<ScorecardRow>> {
    let rows = sqlx::query_as::<_, ScorecardRow>(
        r#"SELECT source,
                  contract_name,
                  total_events,
                  passed,
                  quarantined,
                  quarantine_pct::float8 AS quarantine_pct
           FROM provider_scorecard
           WHERE source = $1
           ORDER BY quarantine_pct DESC NULLS LAST, total_events DESC"#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Query the `provider_field_health` view for one source.
pub async fn query_field_health(pool: &PgPool, source: &str) -> AppResult<Vec<FieldHealthRow>> {
    let rows = sqlx::query_as::<_, FieldHealthRow>(
        r#"SELECT source, contract_name, field, code, violations
           FROM provider_field_health
           WHERE source = $1
           ORDER BY violations DESC"#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Compute active drift signals for a source.
///
/// Compares the last 24 h violation rates (computed live) against the most
/// recent rolling baseline stored in `provider_field_baseline`.  Returns an
/// empty vec when no baseline has been seeded yet.
pub async fn query_drift_signals(pool: &PgPool, source: &str) -> AppResult<Vec<DriftSignal>> {
    // Step 1 — total events per (source, contract_name) in the last 24 h.
    #[derive(sqlx::FromRow)]
    struct TotalsRow {
        contract_name: String,
        total_events: i64,
    }

    let totals: Vec<TotalsRow> = sqlx::query_as::<_, TotalsRow>(
        r#"SELECT c.name AS contract_name,
                  count(*) AS total_events
           FROM audit_log a
           JOIN contracts c ON c.id = a.contract_id
           LEFT JOIN contract_versions cv
               ON  cv.contract_id = a.contract_id
               AND cv.version     = a.contract_version
           WHERE COALESCE(cv.source, '(unsourced)') = $1
             AND a.created_at >= NOW() - INTERVAL '24 hours'
           GROUP BY c.name"#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;

    if totals.is_empty() {
        return Ok(vec![]);
    }

    // Step 2 — per-field violation counts in the last 24 h for this source.
    #[derive(sqlx::FromRow)]
    struct FieldCountRow {
        contract_name: String,
        field: String,
        cnt: i64,
    }

    let field_counts: Vec<FieldCountRow> = sqlx::query_as::<_, FieldCountRow>(
        r#"SELECT c.name AS contract_name,
                  v.field,
                  count(*) AS cnt
           FROM quarantine_events q
           JOIN contracts c ON c.id = q.contract_id
           LEFT JOIN contract_versions cv
               ON  cv.contract_id = q.contract_id
               AND cv.version     = q.contract_version,
           LATERAL jsonb_to_recordset(q.violation_details) AS v(field text)
           WHERE COALESCE(cv.source, '(unsourced)') = $1
             AND q.created_at >= NOW() - INTERVAL '24 hours'
             AND v.field IS NOT NULL
           GROUP BY c.name, v.field"#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;

    if field_counts.is_empty() {
        return Ok(vec![]);
    }

    // Step 3 — load the most recent baseline rows for this source.
    let baselines: Vec<BaselineRow> = sqlx::query_as::<_, BaselineRow>(
        r#"SELECT field,
                  contract_name,
                  violation_rate::float8 AS violation_rate
           FROM provider_field_baseline
           WHERE source = $1
             AND window_start = (
                 SELECT MAX(window_start)
                 FROM provider_field_baseline
                 WHERE source = $1
             )"#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;

    // Build lookup: (contract_name, field) → baseline violation_rate.
    let baseline_map: std::collections::HashMap<(String, String), f64> = baselines
        .into_iter()
        .map(|b| ((b.contract_name, b.field), b.violation_rate))
        .collect();

    // Build lookup: contract_name → total events.
    let totals_map: std::collections::HashMap<String, i64> =
        totals.into_iter().map(|r| (r.contract_name, r.total_events)).collect();

    // Step 4 — compare current rates to baseline; emit signals.
    let mut signals: Vec<DriftSignal> = Vec::new();

    for fc in &field_counts {
        let total = *totals_map.get(&fc.contract_name).unwrap_or(&1).max(&1);
        let current_rate = fc.cnt as f64 / total as f64;

        if let Some(&baseline_rate) = baseline_map
            .get(&(fc.contract_name.clone(), fc.field.clone()))
        {
            let delta_pct = (current_rate - baseline_rate) * 100.0;
            if delta_pct.abs() >= DRIFT_THRESHOLD_PCT {
                let direction = if delta_pct > 0.0 { "↑" } else { "↓" };
                signals.push(DriftSignal {
                    source: source.to_string(),
                    contract_name: fc.contract_name.clone(),
                    field: fc.field.clone(),
                    current_rate,
                    baseline_rate,
                    delta_pct,
                    label: format!("{} {:.1} pp since baseline", direction, delta_pct.abs()),
                });
            }
        }
    }

    // Sort by absolute drift descending so the biggest surprises come first.
    signals.sort_by(|a, b| {
        b.delta_pct
            .abs()
            .partial_cmp(&a.delta_pct.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(signals)
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// `GET /scorecard/{source}` — full scorecard JSON.
pub async fn scorecard_handler(
    State(state): State<Arc<AppState>>,
    Path(source): Path<String>,
) -> AppResult<Json<ScorecardResponse>> {
    let (summary, field_health, drift_signals) = tokio::try_join!(
        query_scorecard(&state.db, &source),
        query_field_health(&state.db, &source),
        query_drift_signals(&state.db, &source),
    )?;

    let top_violations: Vec<FieldHealthRow> = field_health.iter().take(20).cloned().collect();

    Ok(Json(ScorecardResponse {
        source,
        summary,
        top_violations,
        field_health,
        drift_signals,
    }))
}

/// `GET /scorecard/{source}/drift` — active drift signals only.
pub async fn drift_handler(
    State(state): State<Arc<AppState>>,
    Path(source): Path<String>,
) -> AppResult<Json<Vec<DriftSignal>>> {
    let signals = query_drift_signals(&state.db, &source).await?;
    Ok(Json(signals))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    format: Option<String>,
}

/// `GET /scorecard/{source}/export?format=csv` — flat CSV download.
///
/// v1: only `format=csv` is supported.  The CSV has two sections separated
/// by a blank line: a summary section and a field-violations section.
pub async fn export_handler(
    State(state): State<Arc<AppState>>,
    Path(source): Path<String>,
    Query(q): Query<ExportQuery>,
) -> AppResult<Response> {
    if q.format.as_deref().unwrap_or("csv") != "csv" {
        return Err(AppError::BadRequest(
            "Only format=csv is supported in v1".into(),
        ));
    }

    let (summary, field_health) = tokio::try_join!(
        query_scorecard(&state.db, &source),
        query_field_health(&state.db, &source),
    )?;

    let mut csv = String::with_capacity(4096);

    // Section 1 — summary.
    csv.push_str("# SCORECARD SUMMARY\n");
    csv.push_str("source,contract_name,total_events,passed,quarantined,quarantine_pct\n");
    for row in &summary {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            escape_csv(&row.source),
            escape_csv(&row.contract_name),
            row.total_events,
            row.passed,
            row.quarantined,
            row.quarantine_pct
                .map(|p| format!("{:.2}", p))
                .unwrap_or_default(),
        ));
    }

    // Section 2 — field violations.
    csv.push('\n');
    csv.push_str("# FIELD VIOLATIONS\n");
    csv.push_str("source,contract_name,field,violation_code,violation_count\n");
    for row in &field_health {
        csv.push_str(&format!(
            "{},{},{},{},{}\n",
            escape_csv(&row.source),
            escape_csv(&row.contract_name),
            escape_csv(&row.field),
            escape_csv(&row.code),
            row.violations,
        ));
    }

    let filename = format!(
        "scorecard-{}-{}.csv",
        source.replace(' ', "_"),
        chrono::Utc::now().format("%Y-%m-%d"),
    );

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .map_err(|_| AppError::Internal("invalid filename for CSV export".into()))?,
    );

    Ok((StatusCode::OK, headers, csv).into_response())
}

/// Minimal CSV value escaping: wrap in double-quotes and escape interior
/// double-quotes if the value contains commas, quotes, or newlines.
fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Daily baseline rollup job
// ---------------------------------------------------------------------------

/// Compute the trailing [`BASELINE_WINDOW_DAYS`]-day violation rates for all
/// (source, contract, field) combinations and upsert them into
/// `provider_field_baseline`.
///
/// Run daily via cron or `cargo run -- scorecard-rollup`.  Never touches the
/// hot ingest write path.
///
/// The job is idempotent: re-running for the same date is safe (ON CONFLICT DO
/// UPDATE).
pub async fn run_baseline_rollup(pool: &PgPool) -> AppResult<()> {
    tracing::info!("scorecard: starting daily baseline rollup ({}d window)", BASELINE_WINDOW_DAYS);

    let upserted = sqlx::query(
        r#"INSERT INTO provider_field_baseline
               (source, contract_name, field, window_start, null_rate, violation_rate)
           SELECT
               COALESCE(cv.source, '(unsourced)')              AS source,
               c.name                                           AS contract_name,
               v.field,
               CURRENT_DATE - $1                               AS window_start,
               -- null_rate: fraction of events in window where field is absent
               COALESCE(
                   count(*) FILTER (WHERE v.kind = 'missing_required_field')
                   ::numeric / NULLIF(totals.cnt, 0),
                   0
               )                                               AS null_rate,
               -- violation_rate: fraction of events in window tripping a rule
               count(*)::numeric / NULLIF(totals.cnt, 0)       AS violation_rate
           FROM quarantine_events q
           JOIN contracts c ON c.id = q.contract_id
           LEFT JOIN contract_versions cv
               ON  cv.contract_id = q.contract_id
               AND cv.version     = q.contract_version,
           LATERAL jsonb_to_recordset(q.violation_details) AS v(field text, kind text)
           JOIN (
               -- Total events per (source, contract) across the window.
               SELECT
                   COALESCE(cv2.source, '(unsourced)') AS source,
                   c2.name                             AS contract_name,
                   count(*)                            AS cnt
               FROM audit_log a2
               JOIN contracts c2 ON c2.id = a2.contract_id
               LEFT JOIN contract_versions cv2
                   ON  cv2.contract_id = a2.contract_id
                   AND cv2.version     = a2.contract_version
               WHERE a2.created_at >= NOW() - ($1 * INTERVAL '1 day')
               GROUP BY COALESCE(cv2.source, '(unsourced)'), c2.name
           ) totals
               ON  totals.source        = COALESCE(cv.source, '(unsourced)')
               AND totals.contract_name = c.name
           WHERE q.created_at >= NOW() - ($1 * INTERVAL '1 day')
             AND v.field IS NOT NULL
           GROUP BY
               COALESCE(cv.source, '(unsourced)'),
               c.name,
               v.field,
               totals.cnt
           ON CONFLICT (source, contract_name, field, window_start)
           DO UPDATE SET
               null_rate      = EXCLUDED.null_rate,
               violation_rate = EXCLUDED.violation_rate"#,
    )
    .bind(BASELINE_WINDOW_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    tracing::info!(
        "scorecard: baseline rollup complete — {} rows upserted",
        upserted
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests (no DB, no HTTP)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Drift threshold logic
    // -----------------------------------------------------------------------

    fn maybe_drift(current_rate: f64, baseline_rate: f64) -> Option<f64> {
        let delta_pct = (current_rate - baseline_rate) * 100.0;
        if delta_pct.abs() >= DRIFT_THRESHOLD_PCT {
            Some(delta_pct)
        } else {
            None
        }
    }

    #[test]
    fn drift_fires_above_threshold() {
        // 12% now vs 0% baseline → 12 pp → should fire.
        let d = maybe_drift(0.12, 0.0);
        assert!(d.is_some());
        assert!((d.unwrap() - 12.0).abs() < 0.001);
    }

    #[test]
    fn drift_suppressed_below_threshold() {
        // 3 pp delta → below 5 pp cutoff → silent.
        assert!(maybe_drift(0.03, 0.0).is_none());
    }

    #[test]
    fn drift_fires_on_improvement() {
        // Violation rate dropped 20% → 8% → -12 pp → fires (improvement is also notable).
        let d = maybe_drift(0.08, 0.20);
        assert!(d.is_some());
        assert!(d.unwrap() < 0.0);
    }

    #[test]
    fn drift_exactly_at_threshold() {
        // Exactly 5 pp → fires (>= not >).
        assert!(maybe_drift(0.05, 0.0).is_some());
    }

    #[test]
    fn drift_just_below_threshold() {
        // 4.99 pp → silent.
        assert!(maybe_drift(0.0499, 0.0).is_none());
    }

    // -----------------------------------------------------------------------
    // CSV escaping
    // -----------------------------------------------------------------------

    #[test]
    fn csv_plain_value() {
        assert_eq!(escape_csv("hello"), "hello");
    }

    #[test]
    fn csv_value_with_comma() {
        assert_eq!(escape_csv("hello,world"), "\"hello,world\"");
    }

    #[test]
    fn csv_value_with_quotes() {
        // Inner " → "" per RFC 4180.
        assert_eq!(escape_csv(r#"say "hi""#), r#""say ""hi"""#);
    }

    #[test]
    fn csv_value_with_newline() {
        assert_eq!(escape_csv("line1\nline2"), "\"line1\nline2\"");
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn baseline_constants() {
        assert_eq!(BASELINE_WINDOW_DAYS, 30);
        assert_eq!(DRIFT_THRESHOLD_PCT, 5.0);
    }
}
