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
/// Kept as a single-event helper for ad-hoc use (scripts, manual replays).
/// The live ingest handler uses [`quarantine_events_batch`] — see RFC-001.
#[allow(clippy::too_many_arguments, dead_code)]
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
#[allow(clippy::too_many_arguments)]
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

// ---------------------------------------------------------------------------
// Batch insert helpers (RFC-001)
//
// At 1 000 events per request, the old pattern of `tokio::spawn` per event
// would burn through 1 000+ concurrent Postgres connections per batch.  The
// helpers below collapse each side-effect kind (audit / quarantine / forward)
// into a SINGLE multi-row INSERT via PostgreSQL's `UNNEST`, which means a
// batch of 1 000 events uses at most 3 connections to durably record the
// outcome.
//
// The insert structs are plain data: the ingest handler fills them in once
// per event after validation and then hands the vectors to the helpers.
// ---------------------------------------------------------------------------

/// One row to insert into `audit_log`.  Everything needed to reconstruct the
/// ingestion decision; IDs and timestamps are filled in server-side.
#[derive(Debug, Clone)]
pub struct AuditEntryInsert {
    pub contract_id: Uuid,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
}

/// One row to insert into `quarantine_events`.  Only failed events get one.
#[derive(Debug, Clone)]
pub struct QuarantineEventInsert {
    pub contract_id: Uuid,
    pub payload: serde_json::Value,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
}

/// One row to insert into `forwarded_events`.  Only passing events get one.
#[derive(Debug, Clone)]
pub struct ForwardEventInsert {
    pub contract_id: Uuid,
    pub payload: serde_json::Value,
}

/// Batch-insert audit log entries in a single round-trip.
///
/// Uses `UNNEST` of typed arrays so one SQL statement handles the whole batch
/// regardless of size.  Rows in the input slice keep their order; the
/// database assigns each a fresh UUID via `uuid_generate_v4()`.
pub async fn log_audit_entries_batch(
    pool: &PgPool,
    entries: &[AuditEntryInsert],
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    // Split the struct-of-arrays columns for UNNEST.
    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let passed: Vec<bool> = entries.iter().map(|e| e.passed).collect();
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> =
        entries.iter().map(|e| e.violation_details.clone()).collect();
    let raw_events: Vec<serde_json::Value> =
        entries.iter().map(|e| e.raw_event.clone()).collect();
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    // Postgres's UNNEST needs a concrete nullable text[] — represent absent
    // source_ips as empty strings and convert back to NULL in SQL via NULLIF.
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();

    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, passed, violation_count, violation_details,
             raw_event, validation_us, source_ip, created_at)
        SELECT
            uuid_generate_v4(),
            contract_id, passed, violation_count, violation_details,
            raw_event, validation_us,
            NULLIF(source_ip, ''),
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::bool[], $3::int[], $4::jsonb[],
            $5::jsonb[], $6::bigint[], $7::text[]
        ) AS t(contract_id, passed, violation_count, violation_details,
               raw_event, validation_us, source_ip)
        "#,
    )
    .bind(&contract_ids)
    .bind(&passed)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&raw_events)
    .bind(&validation_us)
    .bind(&source_ips)
    .execute(pool)
    .await?;

    Ok(())
}

/// Batch-insert quarantine entries in a single round-trip.
pub async fn quarantine_events_batch(
    pool: &PgPool,
    entries: &[QuarantineEventInsert],
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let payloads: Vec<serde_json::Value> =
        entries.iter().map(|e| e.payload.clone()).collect();
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> =
        entries.iter().map(|e| e.violation_details.clone()).collect();
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();

    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, payload, violation_count, violation_details,
             validation_us, source_ip, status, created_at)
        SELECT
            uuid_generate_v4(),
            contract_id, payload, violation_count, violation_details,
            validation_us,
            NULLIF(source_ip, ''),
            'pending',
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::jsonb[], $3::int[], $4::jsonb[],
            $5::bigint[], $6::text[]
        ) AS t(contract_id, payload, violation_count, violation_details,
               validation_us, source_ip)
        "#,
    )
    .bind(&contract_ids)
    .bind(&payloads)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&validation_us)
    .bind(&source_ips)
    .execute(pool)
    .await?;

    Ok(())
}

/// Batch-insert forwarded events in a single round-trip.
///
/// Unlike the other two helpers this one is awaited inline from the ingest
/// handler so the response can mark individual events as `forwarded: true`.
pub async fn forward_events_batch(
    pool: &PgPool,
    entries: &[ForwardEventInsert],
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let payloads: Vec<serde_json::Value> =
        entries.iter().map(|e| e.payload.clone()).collect();

    sqlx::query(
        r#"
        INSERT INTO forwarded_events (id, contract_id, payload, created_at)
        SELECT uuid_generate_v4(), contract_id, payload, NOW()
        FROM UNNEST($1::uuid[], $2::jsonb[]) AS t(contract_id, payload)
        "#,
    )
    .bind(&contract_ids)
    .bind(&payloads)
    .execute(pool)
    .await?;

    Ok(())
}
