//! RFC-082 — windowed pilot-report aggregation over `audit_log`.
//!
//! Two cheap, index-backed reads: per-version pass/quarantine counts, and the
//! top violations (field + kind) for quarantined events. Both take an optional
//! `[from, to]` window. Callers verify contract ownership (org scope) first via
//! `get_contract_identity`; these queries are keyed by `contract_id`.

use crate::error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

type Ts = chrono::DateTime<chrono::Utc>;

#[derive(sqlx::FromRow)]
pub struct ReportVersionRow {
    pub contract_version: Option<String>,
    pub total: i64,
    pub passed: i64,
    pub quarantined: i64,
}

#[derive(sqlx::FromRow)]
pub struct ReportViolationRow {
    pub field: Option<String>,
    pub kind: Option<String>,
    pub count: i64,
}

/// Per-version pass/quarantine counts for a contract over an optional window.
pub async fn report_by_version(
    pool: &PgPool,
    contract_id: Uuid,
    from: Option<Ts>,
    to: Option<Ts>,
) -> AppResult<Vec<ReportVersionRow>> {
    let rows = sqlx::query_as::<_, ReportVersionRow>(
        r#"
        SELECT
            contract_version,
            count(*)                            AS total,
            count(*) FILTER (WHERE passed)      AS passed,
            count(*) FILTER (WHERE NOT passed)  AS quarantined
        FROM audit_log
        WHERE contract_id = $1
          AND ($2::timestamptz IS NULL OR created_at >= $2)
          AND ($3::timestamptz IS NULL OR created_at <= $3)
        GROUP BY contract_version
        ORDER BY total DESC
        "#,
    )
    .bind(contract_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Top violations (field + kind) across quarantined events for a contract over
/// an optional window. Unnests the `violation_details` JSONB array.
pub async fn report_top_violations(
    pool: &PgPool,
    contract_id: Uuid,
    from: Option<Ts>,
    to: Option<Ts>,
) -> AppResult<Vec<ReportViolationRow>> {
    let rows = sqlx::query_as::<_, ReportViolationRow>(
        r#"
        SELECT
            v->>'field' AS field,
            v->>'kind'  AS kind,
            count(*)    AS count
        FROM audit_log a
        CROSS JOIN LATERAL jsonb_array_elements(a.violation_details) AS v
        WHERE a.contract_id = $1
          AND NOT a.passed
          AND ($2::timestamptz IS NULL OR a.created_at >= $2)
          AND ($3::timestamptz IS NULL OR a.created_at <= $3)
        GROUP BY 1, 2
        ORDER BY count DESC
        LIMIT 20
        "#,
    )
    .bind(contract_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
