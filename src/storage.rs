//! Supabase (PostgreSQL) storage layer for ContractGate.
//!
//! All database access goes through this module.  Uses `sqlx` with **runtime**
//! (non-macro) query execution so the crate builds without requiring a live
//! `DATABASE_URL` at compile time.  To enable compile-time query verification,
//! run `cargo sqlx prepare` against a real database and commit the `.sqlx/`
//! directory, then switch to `query!` / `query_as!` macros.

use crate::contract::{
    Contract, ContractIdentity, ContractSummary, ContractVersion, MultiStableResolution,
    NameHistoryEntry, VersionState, VersionSummary,
};
use crate::error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Internal row helpers — map directly to the `contracts` and
// `contract_versions` table columns.  `_raw` fields are read as strings and
// parsed into enums so we don't need sqlx's Postgres-enum glue.
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ContractIdentityRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    multi_stable_resolution: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl ContractIdentityRow {
    fn into_identity(self) -> AppResult<ContractIdentity> {
        let resolution = MultiStableResolution::parse(&self.multi_stable_resolution)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "invalid multi_stable_resolution in DB: {}",
                    self.multi_stable_resolution
                ))
            })?;
        Ok(ContractIdentity {
            id: self.id,
            name: self.name,
            description: self.description,
            multi_stable_resolution: resolution,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ContractVersionRow {
    id: Uuid,
    contract_id: Uuid,
    version: String,
    state: String,
    yaml_content: String,
    created_at: chrono::DateTime<chrono::Utc>,
    promoted_at: Option<chrono::DateTime<chrono::Utc>>,
    deprecated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ContractVersionRow {
    fn into_version(self) -> AppResult<ContractVersion> {
        let state = VersionState::parse(&self.state).ok_or_else(|| {
            AppError::Internal(format!(
                "invalid contract_versions.state in DB: {}",
                self.state
            ))
        })?;
        Ok(ContractVersion {
            id: self.id,
            contract_id: self.contract_id,
            version: self.version,
            state,
            yaml_content: self.yaml_content,
            created_at: self.created_at,
            promoted_at: self.promoted_at,
            deprecated_at: self.deprecated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ContractSummaryRow {
    id: Uuid,
    name: String,
    multi_stable_resolution: String,
    latest_stable_version: Option<String>,
    version_count: i64,
}

impl ContractSummaryRow {
    fn into_summary(self) -> AppResult<ContractSummary> {
        let resolution = MultiStableResolution::parse(&self.multi_stable_resolution)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "invalid multi_stable_resolution in DB: {}",
                    self.multi_stable_resolution
                ))
            })?;
        Ok(ContractSummary {
            id: self.id,
            name: self.name,
            multi_stable_resolution: resolution,
            latest_stable_version: self.latest_stable_version,
            version_count: self.version_count,
        })
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
// Contract identity CRUD (RFC-002)
//
// A contract identity is the mutable metadata for a logical contract.  The
// immutable validation content lives in `contract_versions`.  These helpers
// operate only on identity rows.
// ---------------------------------------------------------------------------

/// Create a new contract identity AND an initial `v1.0.0` draft from the
/// submitted YAML, all in one transaction.  The YAML is parsed first to
/// reject invalid contracts before any write hits the DB.
///
/// Returns the identity + the freshly created draft version.
pub async fn create_contract(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
    yaml_content: &str,
    resolution: MultiStableResolution,
) -> AppResult<(ContractIdentity, ContractVersion)> {
    // Parse first — reject invalid YAML before touching the DB.
    let _parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let contract_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    let identity_row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        INSERT INTO contracts
            (id, name, description, multi_stable_resolution, created_at, updated_at)
        VALUES ($1, $2, $3, $4, NOW(), NOW())
        RETURNING id, name, description, multi_stable_resolution, created_at, updated_at
        "#,
    )
    .bind(contract_id)
    .bind(name)
    .bind(description)
    .bind(resolution.as_str())
    .fetch_one(&mut *tx)
    .await?;

    let version_row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at)
        VALUES ($1, $2, '1.0.0', 'draft', $3, NOW())
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at
        "#,
    )
    .bind(version_id)
    .bind(contract_id)
    .bind(yaml_content)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((identity_row.into_identity()?, version_row.into_version()?))
}

/// Fetch a contract identity by id.
pub async fn get_contract_identity(
    pool: &PgPool,
    id: Uuid,
) -> AppResult<ContractIdentity> {
    let row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        SELECT id, name, description, multi_stable_resolution, created_at, updated_at
        FROM contracts
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ContractNotFound(id))?;

    row.into_identity()
}

/// List contracts with aggregated version info — suitable for the dashboard
/// list view.
pub async fn list_contracts(pool: &PgPool) -> AppResult<Vec<ContractSummary>> {
    // Subquery picks the most recently promoted stable version per contract.
    let rows = sqlx::query_as::<_, ContractSummaryRow>(
        r#"
        SELECT
            c.id,
            c.name,
            c.multi_stable_resolution,
            (
                SELECT version
                FROM contract_versions cv
                WHERE cv.contract_id = c.id AND cv.state = 'stable'
                ORDER BY cv.promoted_at DESC
                LIMIT 1
            ) AS latest_stable_version,
            (
                SELECT COUNT(*)::bigint
                FROM contract_versions cv
                WHERE cv.contract_id = c.id
            ) AS version_count
        FROM contracts c
        ORDER BY c.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| r.into_summary()).collect()
}

/// Patch identity-level fields.  Passing `Some(value)` updates the field;
/// `None` leaves it alone.  Name changes are mirrored to
/// `contract_name_history` by the DB trigger.
pub async fn patch_contract_identity(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    resolution: Option<MultiStableResolution>,
) -> AppResult<ContractIdentity> {
    // COALESCE keeps the existing value when the bind is NULL.  Resolution
    // is bound as a string with an explicit NULL when not provided.
    let row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        UPDATE contracts
        SET
            name                    = COALESCE($2, name),
            description             = COALESCE($3, description),
            multi_stable_resolution = COALESCE($4, multi_stable_resolution),
            updated_at              = NOW()
        WHERE id = $1
        RETURNING id, name, description, multi_stable_resolution, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(resolution.map(|r| r.as_str()))
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ContractNotFound(id))?;

    row.into_identity()
}

pub async fn delete_contract(pool: &PgPool, id: Uuid) -> AppResult<()> {
    // ON DELETE CASCADE on contract_versions + quarantine + audit handles
    // the rest — but draft-only-delete guard on contract_versions would
    // abort this whenever any version is stable/deprecated.  This is
    // intentional: once any version is non-draft, the contract is part of
    // the audit trail forever.  Callers should deprecate instead.
    sqlx::query("DELETE FROM contracts WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Contract version CRUD (RFC-002)
// ---------------------------------------------------------------------------

/// Create a new draft version on an existing contract.  The YAML is parsed
/// first so invalid contracts never land in the DB.
pub async fn create_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
) -> AppResult<ContractVersion> {
    let _parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    // Ensure the contract exists first so we get a clean 404 instead of a
    // foreign-key violation.
    let _ = get_contract_identity(pool, contract_id).await?;

    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at)
        VALUES ($1, $2, $3, 'draft', $4, NOW())
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at
        "#,
    )
    .bind(id)
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            // unique_violation on (contract_id, version)
            AppError::VersionConflict {
                contract_id,
                version: version.to_string(),
            }
        }
        _ => AppError::from(e),
    })?;

    row.into_version()
}

/// Edit a draft version's YAML.  Illegal (409) on stable/deprecated — the
/// Postgres trigger blocks that at the storage layer too.
pub async fn patch_version_yaml(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
) -> AppResult<ContractVersion> {
    let _parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    // Fetch first so we can emit a specific error (not-found vs. immutable).
    let current = get_version(pool, contract_id, version).await?;
    if current.state != VersionState::Draft {
        return Err(AppError::VersionImmutable {
            version: version.to_string(),
            state: current.state.as_str().to_string(),
        });
    }

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        UPDATE contract_versions
        SET yaml_content = $3
        WHERE contract_id = $1 AND version = $2
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Transition draft → stable.  Rejects any other source state.
pub async fn promote_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
) -> AppResult<ContractVersion> {
    let current = get_version(pool, contract_id, version).await?;
    if current.state != VersionState::Draft {
        return Err(AppError::InvalidStateTransition {
            from: current.state.as_str().to_string(),
            to: "stable".to_string(),
            version: version.to_string(),
        });
    }

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        UPDATE contract_versions
        SET state = 'stable', promoted_at = NOW()
        WHERE contract_id = $1 AND version = $2 AND state = 'draft'
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Transition stable → deprecated.  Rejects any other source state.
pub async fn deprecate_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
) -> AppResult<ContractVersion> {
    let current = get_version(pool, contract_id, version).await?;
    if current.state != VersionState::Stable {
        return Err(AppError::InvalidStateTransition {
            from: current.state.as_str().to_string(),
            to: "deprecated".to_string(),
            version: version.to_string(),
        });
    }

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        UPDATE contract_versions
        SET state = 'deprecated', deprecated_at = NOW()
        WHERE contract_id = $1 AND version = $2 AND state = 'stable'
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Delete a draft version.  Postgres trigger enforces the draft-only rule
/// as well — so even a direct SQL hit cannot remove a stable/deprecated
/// row.
pub async fn delete_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
) -> AppResult<()> {
    let current = get_version(pool, contract_id, version).await?;
    if current.state != VersionState::Draft {
        return Err(AppError::VersionImmutable {
            version: version.to_string(),
            state: current.state.as_str().to_string(),
        });
    }

    sqlx::query(
        r#"DELETE FROM contract_versions WHERE contract_id = $1 AND version = $2"#,
    )
    .bind(contract_id)
    .bind(version)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
) -> AppResult<ContractVersion> {
    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at
        FROM contract_versions
        WHERE contract_id = $1 AND version = $2
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::VersionNotFound {
        contract_id,
        version: version.to_string(),
    })?;

    row.into_version()
}

pub async fn list_versions(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Vec<VersionSummary>> {
    // Ensure contract exists so callers get a clean 404.
    let _ = get_contract_identity(pool, contract_id).await?;

    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at
        FROM contract_versions
        WHERE contract_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            let v = r.into_version()?;
            Ok(VersionSummary {
                version: v.version,
                state: v.state,
                created_at: v.created_at,
                promoted_at: v.promoted_at,
                deprecated_at: v.deprecated_at,
            })
        })
        .collect()
}

/// Return the most-recently-promoted stable version for a contract, if any.
pub async fn get_latest_stable_version(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Option<ContractVersion>> {
    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at
        FROM contract_versions
        WHERE contract_id = $1 AND state = 'stable'
        ORDER BY promoted_at DESC
        LIMIT 1
        "#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;

    row.map(|r| r.into_version()).transpose()
}

/// Return all `stable` versions for a contract, newest-promoted first.
/// Used by `fallback`-mode resolution to iterate remaining candidates.
pub async fn list_stable_versions(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Vec<ContractVersion>> {
    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at
        FROM contract_versions
        WHERE contract_id = $1 AND state = 'stable'
        ORDER BY promoted_at DESC
        "#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| r.into_version()).collect()
}

/// Load every stable + deprecated version across all contracts — used at
/// boot to warm the in-memory cache.  Drafts are loaded lazily on pin.
pub async fn load_all_non_draft_versions(
    pool: &PgPool,
) -> AppResult<Vec<ContractVersion>> {
    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at
        FROM contract_versions
        WHERE state IN ('stable', 'deprecated')
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| r.into_version()).collect()
}

// ---------------------------------------------------------------------------
// Name history
// ---------------------------------------------------------------------------

pub async fn list_name_history(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<Vec<NameHistoryEntry>> {
    let rows = sqlx::query_as::<_, NameHistoryEntry>(
        r#"
        SELECT id, contract_id, old_name, new_name, changed_at
        FROM contract_name_history
        WHERE contract_id = $1
        ORDER BY changed_at DESC
        "#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Quarantine events
// ---------------------------------------------------------------------------

/// Write a failed event to the quarantine table.
///
/// Kept as a single-event helper for ad-hoc use (scripts, manual replays).
/// The live ingest handler uses [`quarantine_events_batch`] — see RFC-001.
///
/// `contract_version` must be the exact version that rejected the event
/// (audit honesty — see `feedback_audit_honesty` memory).
#[allow(clippy::too_many_arguments, dead_code)]
pub async fn quarantine_event(
    pool: &PgPool,
    contract_id: Uuid,
    contract_version: &str,
    payload: serde_json::Value,
    violation_count: i32,
    violation_details: serde_json::Value,
    validation_us: i64,
    source_ip: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, contract_version, payload, violation_count,
             violation_details, validation_us, source_ip, status, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(contract_version)
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
///
/// `contract_version` must reflect the version that actually produced the
/// decision — never a default or guess (audit honesty).
#[allow(clippy::too_many_arguments)]
pub async fn log_audit_entry(
    pool: &PgPool,
    contract_id: Uuid,
    contract_version: &str,
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
            (id, contract_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(contract_version)
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
            SELECT id, contract_id, contract_version, passed, violation_count,
                   violation_details, raw_event, validation_us, source_ip, created_at
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
            SELECT id, contract_id, contract_version, passed, violation_count,
                   violation_details, raw_event, validation_us, source_ip, created_at
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
    /// The exact contract version that accepted or rejected this event.
    /// Optional at the schema level (legacy rows pre-RFC-002 may be NULL),
    /// but RFC-002 writes always populate it.
    pub contract_version: Option<String>,
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
///
/// `contract_version` must be the exact version that produced the decision
/// (audit honesty).  Under fallback resolution, this is the version that
/// actually accepted (or, for the final fail, rejected) the event.
#[derive(Debug, Clone)]
pub struct AuditEntryInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
}

/// One row to insert into `quarantine_events`.  Only failed events get one.
///
/// `contract_version` is the version that rejected the event.
#[derive(Debug, Clone)]
pub struct QuarantineEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: serde_json::Value,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
}

/// One row to insert into `forwarded_events`.  Only passing events get one.
///
/// `contract_version` is the version that accepted the event.
#[derive(Debug, Clone)]
pub struct ForwardEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
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
    let contract_versions: Vec<String> = entries
        .iter()
        .map(|e| e.contract_version.clone())
        .collect();
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
            (id, contract_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip, created_at)
        SELECT
            uuid_generate_v4(),
            contract_id, contract_version, passed, violation_count,
            violation_details, raw_event, validation_us,
            NULLIF(source_ip, ''),
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::text[], $3::bool[], $4::int[], $5::jsonb[],
            $6::jsonb[], $7::bigint[], $8::text[]
        ) AS t(contract_id, contract_version, passed, violation_count,
               violation_details, raw_event, validation_us, source_ip)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
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
    let contract_versions: Vec<String> = entries
        .iter()
        .map(|e| e.contract_version.clone())
        .collect();
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
            (id, contract_id, contract_version, payload, violation_count,
             violation_details, validation_us, source_ip, status, created_at)
        SELECT
            uuid_generate_v4(),
            contract_id, contract_version, payload, violation_count,
            violation_details, validation_us,
            NULLIF(source_ip, ''),
            'pending',
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::text[], $3::jsonb[], $4::int[], $5::jsonb[],
            $6::bigint[], $7::text[]
        ) AS t(contract_id, contract_version, payload, violation_count,
               violation_details, validation_us, source_ip)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
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
    let contract_versions: Vec<String> = entries
        .iter()
        .map(|e| e.contract_version.clone())
        .collect();
    let payloads: Vec<serde_json::Value> =
        entries.iter().map(|e| e.payload.clone()).collect();

    sqlx::query(
        r#"
        INSERT INTO forwarded_events
            (id, contract_id, contract_version, payload, created_at)
        SELECT uuid_generate_v4(), contract_id, contract_version, payload, NOW()
        FROM UNNEST($1::uuid[], $2::text[], $3::jsonb[])
            AS t(contract_id, contract_version, payload)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
    .bind(&payloads)
    .execute(pool)
    .await?;

    Ok(())
}
