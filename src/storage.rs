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
// Internal row helper for aggregate stats.
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct StatsRow {
    total: i64,
    passed: i64,
    avg_us: f64,
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

pub async fn update_contract_active(pool: &PgPool, id: Uuid, active: bool) -> AppResult<()> {
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

/// Fetch summary statistics for the dashboard monitor.
///
/// Uses explicit casts to guarantee the Postgres return types that sqlx expects:
///   - `COUNT(*)` → bigint (i64)
///   - `SUM(...)::bigint` → bigint (i64), COALESCE ensures non-NULL
///   - `AVG(...)::float8`  → double precision (f64), COALESCE ensures non-NULL
pub async fn ingestion_stats(
    pool: &PgPool,
    contract_id: Option<Uuid>,
) -> AppResult<IngestionStats> {
    let r = if let Some(cid) = contract_id {
        sqlx::query_as::<_, StatsRow>(
            r#"
            SELECT
                COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
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
                COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
            FROM audit_log
            "#,
        )
        .fetch_one(pool)
        .await?
    };

    Ok(IngestionStats {
        total_events: r.total,
        passed_events: r.passed,
        failed_events: r.total - r.passed,
        pass_rate: if r.total > 0 {
            r.passed as f64 / r.total as f64
        } else {
            0.0
        },
        avg_validation_us: r.avg_us,
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
}
