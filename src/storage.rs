//! Supabase (PostgreSQL) storage layer for ContractGate.
//!
//! All database access goes through this module.  Uses `sqlx` with **runtime**
//! (non-macro) query execution so the crate builds without requiring a live
//! `DATABASE_URL` at compile time.  To enable compile-time query verification,
//! run `cargo sqlx prepare` against a real database and commit the `.sqlx/`
//! directory, then switch to `query!` / `query_as!` macros.

use crate::contract::{Contract, ContractSummary, StoredContract};
use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Internal row helper — maps directly to the `contracts` table columns.
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ContractRow {
    id: Uuid,
    name: String,
    version: String,
    active: bool,
    yaml_content: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl ContractRow {
    fn into_stored(self, parsed: Option<Contract>) -> StoredContract {
        StoredContract {
            id: self.id,
            name: self.name,
            version: self.version,
            active: self.active,
            yaml_content: self.yaml_content,
            created_at: self.created_at,
            updated_at: self.updated_at,
            parsed,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal row helpers for aggregate stats.
// ---------------------------------------------------------------------------

/// Maps the COUNT/SUM/AVG aggregate query for ingestion stats.
/// All fields use Option<> for safety across different row-count scenarios.
#[derive(sqlx::FromRow)]
struct StatsRow {
    total: Option<i64>,
    passed: Option<i64>,
    avg_us: Option<f64>,
}

/// Maps the percentile_disc aggregate query results.
#[derive(sqlx::FromRow)]
struct PercRow {
    p50: Option<i64>,
    p95: Option<i64>,
    p99: Option<i64>,
}

// ---------------------------------------------------------------------------
// Contract CRUD
// ---------------------------------------------------------------------------

pub async fn create_contract(pool: &PgPool, yaml_content: &str) -> AppResult<StoredContract> {
    // Parse first to validate the YAML and extract name/version
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let row = sqlx::query_as::<_, ContractRow>(
        r#"
        INSERT INTO contracts (id, name, version, active, yaml_content, created_at, updated_at)
        VALUES ($1, $2, $3, true, $4, $5, $6)
        RETURNING id, name, version, active, yaml_content, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&parsed.name)
    .bind(&parsed.version)
    .bind(yaml_content)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(row.into_stored(Some(parsed)))
}

pub async fn get_contract(pool: &PgPool, id: Uuid) -> AppResult<StoredContract> {
    let row = sqlx::query_as::<_, ContractRow>(
        r#"
        SELECT id, name, version, active, yaml_content, created_at, updated_at
        FROM contracts
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ContractNotFound(id))?;

    let mut sc = row.into_stored(None);
    sc.parse()
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    Ok(sc)
}

pub async fn list_contracts(pool: &PgPool) -> AppResult<Vec<ContractSummary>> {
    let rows = sqlx::query_as::<_, ContractSummary>(
        r#"
        SELECT id, name, version, active
        FROM contracts
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Replace a contract's YAML content.  Parses the YAML first so invalid
/// contracts are rejected before touching the database.
pub async fn update_contract_yaml(
    pool: &PgPool,
    id: Uuid,
    yaml_content: &str,
) -> AppResult<()> {
    // Validate YAML before writing
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let result = sqlx::query(
        r#"
        UPDATE contracts
        SET yaml_content = $1, name = $2, version = $3, updated_at = NOW()
        WHERE id = $4
        "#,
    )
    .bind(yaml_content)
    .bind(&parsed.name)
    .bind(&parsed.version)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::ContractNotFound(id));
    }
    Ok(())
}

pub async fn update_contract_active(
    pool: &PgPool,
    id: Uuid,
    active: bool,
) -> AppResult<()> {
    let result = sqlx::query(
        r#"UPDATE contracts SET active = $1, updated_at = NOW() WHERE id = $2"#,
    )
    .bind(active)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::ContractNotFound(id));
    }
    Ok(())
}

pub async fn delete_contract(pool: &PgPool, id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM contracts WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Quarantine events
// ---------------------------------------------------------------------------

/// Write a failed event to the quarantine table.
///
/// Called fire-and-forget from the ingest handler — errors are logged but
/// do not affect the HTTP response.
pub async fn quarantine_event(
    pool: &PgPool,
    contract_id: Uuid,
    payload: serde_json::Value,
    violation_count: i32,
    violation_details: serde_json::Value,
    validation_us: i64,
    source_ip: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, payload, violation_count, violation_details,
             validation_us, source_ip, status, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(payload)
    .bind(violation_count)
    .bind(violation_details)
    .bind(validation_us)
    .bind(source_ip)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// Insert a record into the audit log after each ingestion attempt.
pub async fn log_audit_entry(
    pool: &PgPool,
    contract_id: Uuid,
    passed: bool,
    violation_count: i32,
    violation_details: serde_json::Value,
    raw_event: serde_json::Value,
    validation_us: i64,
    source_ip: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, passed, violation_count, violation_details,
             raw_event, validation_us, source_ip, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(passed)
    .bind(violation_count)
    .bind(violation_details)
    .bind(raw_event)
    .bind(validation_us)
    .bind(source_ip)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch recent audit entries for the dashboard monitor.
pub async fn recent_audit_entries(
    pool: &PgPool,
    contract_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<AuditEntry>> {
    let rows = if let Some(cid) = contract_id {
        sqlx::query_as::<_, AuditEntry>(
            r#"
            SELECT id, contract_id, passed, violation_count, violation_details,
                   raw_event, validation_us, source_ip, created_at
            FROM audit_log
            WHERE contract_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(cid)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, AuditEntry>(
            r#"
            SELECT id, contract_id, passed, violation_count, violation_details,
                   raw_event, validation_us, source_ip, created_at
            FROM audit_log
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}

/// Fetch summary statistics for the dashboard monitor, including p50/p95/p99 latency.
///
/// Uses two separate runtime queries — one for counts/avg, one for percentiles —
/// to avoid compile-time DATABASE_URL dependency while staying fast (both are
/// simple aggregates with index support on `validation_us`).
pub async fn ingestion_stats(
    pool: &PgPool,
    contract_id: Option<Uuid>,
) -> AppResult<IngestionStats> {
    // -----------------------------------------------------------------------
    // Query 1: aggregate counts + average latency
    // -----------------------------------------------------------------------
    let stats: StatsRow = if let Some(cid) = contract_id {
        sqlx::query_as::<_, StatsRow>(
            r#"
            SELECT
                COUNT(*)::bigint                                                    AS total,
                COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint       AS passed,
                COALESCE(AVG(validation_us::float8), 0.0)                          AS avg_us
            FROM audit_log
            WHERE contract_id = $1
            "#,
        )
        .bind(cid)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, StatsRow>(
            r#"
            SELECT
                COUNT(*)::bigint                                                    AS total,
                COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint       AS passed,
                COALESCE(AVG(validation_us::float8), 0.0)                          AS avg_us
            FROM audit_log
            "#,
        )
        .fetch_one(pool)
        .await?
    };

    let total = stats.total.unwrap_or(0);
    let passed = stats.passed.unwrap_or(0);
    let avg_us = stats.avg_us.unwrap_or(0.0);

    // -----------------------------------------------------------------------
    // Query 2: percentile latencies (p50 / p95 / p99)
    //
    // percentile_disc is a built-in ordered-set aggregate in PostgreSQL 9.4+.
    // Returns NULL when the input set is empty — handled safely by Option<i64>.
    // -----------------------------------------------------------------------
    let perc: PercRow = if let Some(cid) = contract_id {
        sqlx::query_as::<_, PercRow>(
            r#"
            SELECT
                percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
            FROM audit_log
            WHERE contract_id = $1
            "#,
        )
        .bind(cid)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, PercRow>(
            r#"
            SELECT
                percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
            FROM audit_log
            "#,
        )
        .fetch_one(pool)
        .await?
    };

    Ok(IngestionStats {
        total_events: total,
        passed_events: passed,
        failed_events: total - passed,
        pass_rate: if total > 0 {
            passed as f64 / total as f64
        } else {
            0.0
        },
        avg_validation_us: avg_us,
        p50_validation_us: perc.p50.unwrap_or(0),
        p95_validation_us: perc.p95.unwrap_or(0),
        p99_validation_us: perc.p99.unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// Data models returned from the DB
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct IngestionStats {
    pub total_events: i64,
    pub passed_events: i64,
    pub failed_events: i64,
    pub pass_rate: f64,
    pub avg_validation_us: f64,
    /// Median (p50) validation latency in microseconds
    pub p50_validation_us: i64,
    /// 95th-percentile validation latency in microseconds
    pub p95_validation_us: i64,
    /// 99th-percentile validation latency in microseconds (target: <15 000 µs)
    pub p99_validation_us: i64,
}
