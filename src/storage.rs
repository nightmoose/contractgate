//! Supabase (PostgreSQL) storage layer for ContractGate.
//!
//! All database access goes through this module.  Uses `sqlx` with compile-time
//! query checking disabled (`query!` macro) to keep the build self-contained —
//! switch to `query!` with DATABASE_URL set for full compile-time verification.

use crate::contract::{Contract, ContractSummary, StoredContract};
use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Contract CRUD
// ---------------------------------------------------------------------------

pub async fn create_contract(
    pool: &PgPool,
    yaml_content: &str,
) -> AppResult<StoredContract> {
    // Parse first to validate the YAML
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let row = sqlx::query!(
        r#"
        INSERT INTO contracts (id, name, version, active, yaml_content, created_at, updated_at)
        VALUES ($1, $2, $3, true, $4, $5, $6)
        RETURNING id, name, version, active, yaml_content, created_at, updated_at
        "#,
        id,
        parsed.name,
        parsed.version,
        yaml_content,
        now,
        now
    )
    .fetch_one(pool)
    .await?;

    Ok(StoredContract {
        id: row.id,
        name: row.name,
        version: row.version,
        active: row.active,
        yaml_content: row.yaml_content,
        created_at: row.created_at,
        updated_at: row.updated_at,
        parsed: Some(parsed),
    })
}

pub async fn get_contract(pool: &PgPool, id: Uuid) -> AppResult<StoredContract> {
    let row = sqlx::query!(
        r#"
        SELECT id, name, version, active, yaml_content, created_at, updated_at
        FROM contracts
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ContractNotFound(id))?;

    let mut sc = StoredContract {
        id: row.id,
        name: row.name,
        version: row.version,
        active: row.active,
        yaml_content: row.yaml_content,
        created_at: row.created_at,
        updated_at: row.updated_at,
        parsed: None,
    };
    sc.parse().map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    Ok(sc)
}

pub async fn list_contracts(pool: &PgPool) -> AppResult<Vec<ContractSummary>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, version, active
        FROM contracts
        ORDER BY created_at DESC
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ContractSummary {
            id: r.id,
            name: r.name,
            version: r.version,
            active: r.active,
        })
        .collect())
}

pub async fn update_contract_active(
    pool: &PgPool,
    id: Uuid,
    active: bool,
) -> AppResult<()> {
    let result = sqlx::query!(
        r#"UPDATE contracts SET active = $1, updated_at = NOW() WHERE id = $2"#,
        active,
        id
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::ContractNotFound(id));
    }
    Ok(())
}

pub async fn delete_contract(pool: &PgPool, id: Uuid) -> AppResult<()> {
    sqlx::query!("DELETE FROM contracts WHERE id = $1", id)
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
    sqlx::query!(
        r#"
        INSERT INTO audit_log
            (id, contract_id, passed, violation_count, violation_details,
             raw_event, validation_us, source_ip, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        "#,
        Uuid::new_v4(),
        contract_id,
        passed,
        violation_count,
        violation_details,
        raw_event,
        validation_us,
        source_ip
    )
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
        sqlx::query_as!(
            AuditEntry,
            r#"
            SELECT id, contract_id, passed, violation_count, violation_details,
                   raw_event, validation_us, source_ip, created_at
            FROM audit_log
            WHERE contract_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
            cid,
            limit,
            offset
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as!(
            AuditEntry,
            r#"
            SELECT id, contract_id, passed, violation_count, violation_details,
                   raw_event, validation_us, source_ip, created_at
            FROM audit_log
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            limit,
            offset
        )
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}

/// Fetch summary statistics for the dashboard monitor.
pub async fn ingestion_stats(
    pool: &PgPool,
    contract_id: Option<Uuid>,
) -> AppResult<IngestionStats> {
    let (total, passed, avg_us) = if let Some(cid) = contract_id {
        let r = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as total,
                SUM(CASE WHEN passed THEN 1 ELSE 0 END) as passed,
                AVG(validation_us) as avg_us
            FROM audit_log
            WHERE contract_id = $1
            "#,
            cid
        )
        .fetch_one(pool)
        .await?;
        (r.total.unwrap_or(0), r.passed.unwrap_or(0), r.avg_us.unwrap_or(0.0))
    } else {
        let r = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as total,
                SUM(CASE WHEN passed THEN 1 ELSE 0 END) as passed,
                AVG(validation_us) as avg_us
            FROM audit_log
            "#
        )
        .fetch_one(pool)
        .await?;
        (r.total.unwrap_or(0), r.passed.unwrap_or(0), r.avg_us.unwrap_or(0.0))
    };

    Ok(IngestionStats {
        total_events: total,
        passed_events: passed,
        failed_events: total - passed,
        pass_rate: if total > 0 { passed as f64 / total as f64 } else { 0.0 },
        avg_validation_us: avg_us,
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
