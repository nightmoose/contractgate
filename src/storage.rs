//! Supabase (PostgreSQL) storage layer for ContractGate.
//!
//! All database access goes through this module.  Uses `sqlx` with **runtime**
//! (non-macro) query execution so the crate builds without requiring a live
//! `DATABASE_URL` at compile time.  To enable compile-time query verification,
//! run `cargo sqlx prepare` against a real database and commit the `.sqlx/`
//! directory, then switch to `query!` / `query_as!` macros.

use crate::contract::{
    Contract, ContractIdentity, ContractSummary, ContractVersion, ImportSource,
    MultiStableResolution, NameHistoryEntry, VersionState, VersionSummary,
};
use crate::error::{AppError, AppResult, DbOpContext};
use crate::transform::TransformedPayload;
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
    /// RFC-004: per-contract 32-byte secret salt.  BYTEA in Postgres.
    pii_salt: Vec<u8>,
}

impl ContractIdentityRow {
    fn into_identity(self) -> AppResult<ContractIdentity> {
        let resolution = self
            .multi_stable_resolution
            .parse::<MultiStableResolution>()
            .map_err(|e| {
                AppError::Internal(format!("invalid multi_stable_resolution in DB: {e}"))
            })?;
        Ok(ContractIdentity {
            id: self.id,
            name: self.name,
            description: self.description,
            multi_stable_resolution: resolution,
            created_at: self.created_at,
            updated_at: self.updated_at,
            pii_salt: self.pii_salt,
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
    /// RFC-004: denormalized for SQL-level filtering + indexing.  YAML
    /// remains the single source of truth — this column is synced to
    /// `Contract::compliance_mode` at INSERT / UPDATE time.
    compliance_mode: bool,
    /// Migration 010: ODCS import provenance.
    import_source: String,
    /// Migration 010: blocks promotion until human review clears it.
    requires_review: bool,
}

impl ContractVersionRow {
    fn into_version(self) -> AppResult<ContractVersion> {
        let state = self.state.parse::<VersionState>().map_err(|e| {
            AppError::Internal(format!("invalid contract_versions.state in DB: {e}"))
        })?;
        let import_source = self.import_source.parse::<ImportSource>().map_err(|e| {
            AppError::Internal(format!("invalid import_source in DB: {e}"))
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
            compliance_mode: self.compliance_mode,
            import_source,
            requires_review: self.requires_review,
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
        let resolution = self
            .multi_stable_resolution
            .parse::<MultiStableResolution>()
            .map_err(|e| {
                AppError::Internal(format!("invalid multi_stable_resolution in DB: {e}"))
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
/// `org_id` scopes the contract to a specific org. Pass `None` in dev-mode
/// (no auth configured) to skip org assignment — fine for local testing.
///
/// Returns the identity + the freshly created draft version.
pub async fn create_contract(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
    yaml_content: &str,
    resolution: MultiStableResolution,
    org_id: Option<Uuid>,
) -> AppResult<(ContractIdentity, ContractVersion)> {
    // Parse first — reject invalid YAML before touching the DB.  We also
    // extract `compliance_mode` so the column stays in sync with the YAML
    // (RFC-004: YAML is authoritative; column is a denormalization for
    // SQL-level filtering).
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;

    let contract_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let mut tx = pool.begin().await.db_op("create_contract:begin")?;

    let identity_row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        INSERT INTO contracts
            (id, org_id, name, description, multi_stable_resolution, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
        RETURNING id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(name)
    .bind(description)
    .bind(resolution.as_str())
    .fetch_one(&mut *tx)
    .await
    .db_op("create_contract:insert_identity")?;

    let version_row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at, compliance_mode,
             import_source, requires_review)
        VALUES ($1, $2, '1.0.0', 'draft', $3, NOW(), $4, 'native', FALSE)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
        "#,
    )
    .bind(version_id)
    .bind(contract_id)
    .bind(yaml_content)
    .bind(compliance_mode)
    .fetch_one(&mut *tx)
    .await
    .db_op("create_contract:insert_initial_version")?;

    tx.commit().await.db_op("create_contract:commit")?;

    Ok((identity_row.into_identity()?, version_row.into_version()?))
}

/// Fetch a contract identity by id.
pub async fn get_contract_identity(pool: &PgPool, id: Uuid) -> AppResult<ContractIdentity> {
    let row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        SELECT id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
        FROM contracts
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .db_op("get_contract_identity")?
    .ok_or(AppError::ContractNotFound(id))?;

    row.into_identity()
}

/// List contracts with aggregated version info — suitable for the dashboard
/// list view.  When `org_id` is Some, results are scoped to that org.
/// Pass `None` only in dev-mode (no auth configured).
pub async fn list_contracts(
    pool: &PgPool,
    org_id: Option<Uuid>,
) -> AppResult<Vec<ContractSummary>> {
    // Subquery picks the most recently promoted stable version per contract.
    let rows = if let Some(oid) = org_id {
        sqlx::query_as::<_, ContractSummaryRow>(
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
            WHERE c.org_id = $1
            ORDER BY c.created_at DESC
            "#,
        )
        .bind(oid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ContractSummaryRow>(
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
        .await?
    };

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
        RETURNING id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
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
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;

    // Ensure the contract exists first so we get a clean 404 instead of a
    // foreign-key violation.
    let _ = get_contract_identity(pool, contract_id).await?;

    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at, compliance_mode,
             import_source, requires_review)
        VALUES ($1, $2, $3, 'draft', $4, NOW(), $5, 'native', FALSE)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
        "#,
    )
    .bind(id)
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
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
    // Parse first — reject invalid YAML before touching the DB.  We also
    // extract `compliance_mode` so the column stays in sync on UPDATE.  The
    // DB trigger `contract_versions_compliance_mode_guard` will reject any
    // change to `compliance_mode` once the version leaves draft, so this is
    // safe to always bind (RFC-004).
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;

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
        SET yaml_content = $3,
            compliance_mode = $4
        WHERE contract_id = $1 AND version = $2
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
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
    // D-002: block stripped ODCS imports until a human approves them.
    if current.requires_review {
        return Err(AppError::OdcsReviewRequired {
            contract_id,
            version: version.to_string(),
        });
    }

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        UPDATE contract_versions
        SET state = 'stable', promoted_at = NOW()
        WHERE contract_id = $1 AND version = $2 AND state = 'draft'
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
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
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
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
pub async fn delete_version(pool: &PgPool, contract_id: Uuid, version: &str) -> AppResult<()> {
    let current = get_version(pool, contract_id, version).await?;
    if current.state != VersionState::Draft {
        return Err(AppError::VersionImmutable {
            version: version.to_string(),
            state: current.state.as_str().to_string(),
        });
    }

    sqlx::query(r#"DELETE FROM contract_versions WHERE contract_id = $1 AND version = $2"#)
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
               created_at, promoted_at, deprecated_at, compliance_mode,
               import_source, requires_review
        FROM contract_versions
        WHERE contract_id = $1 AND version = $2
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_optional(pool)
    .await
    .db_op("get_version")?
    .ok_or_else(|| AppError::VersionNotFound {
        contract_id,
        version: version.to_string(),
    })?;

    row.into_version()
}

pub async fn list_versions(pool: &PgPool, contract_id: Uuid) -> AppResult<Vec<VersionSummary>> {
    // Ensure contract exists so callers get a clean 404.
    let _ = get_contract_identity(pool, contract_id).await?;

    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at, compliance_mode,
               import_source, requires_review
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
                import_source: v.import_source,
                requires_review: v.requires_review,
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
               created_at, promoted_at, deprecated_at, compliance_mode,
               import_source, requires_review
        FROM contract_versions
        WHERE contract_id = $1 AND state = 'stable'
        ORDER BY promoted_at DESC
        LIMIT 1
        "#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await
    .db_op("get_latest_stable_version")?;

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
               created_at, promoted_at, deprecated_at, compliance_mode,
               import_source, requires_review
        FROM contract_versions
        WHERE contract_id = $1 AND state = 'stable'
        ORDER BY promoted_at DESC
        "#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await
    .db_op("list_stable_versions")?;

    rows.into_iter().map(|r| r.into_version()).collect()
}

/// Load every contract's `pii_salt` keyed by contract id.  Used at boot
/// alongside [`load_all_non_draft_versions`] so the warm-cache path can
/// compile each version against the right salt without issuing a
/// per-contract round-trip.
pub async fn load_all_pii_salts(
    pool: &PgPool,
) -> AppResult<std::collections::HashMap<Uuid, Vec<u8>>> {
    let rows: Vec<(Uuid, Vec<u8>)> = sqlx::query_as(r#"SELECT id, pii_salt FROM contracts"#)
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().collect())
}

/// Load every stable + deprecated version across all contracts — used at
/// boot to warm the in-memory cache.  Drafts are loaded lazily on pin.
pub async fn load_all_non_draft_versions(pool: &PgPool) -> AppResult<Vec<ContractVersion>> {
    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at, compliance_mode,
               import_source, requires_review
        FROM contract_versions
        WHERE state IN ('stable', 'deprecated')
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| r.into_version()).collect()
}

/// Create a new draft version from an ODCS import.  Functionally identical to
/// [`create_version`] but accepts an explicit `import_source` and sets
/// `requires_review` when the source is `odcs_stripped` (D-002).
pub async fn create_version_from_import(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
    import_source: ImportSource,
) -> AppResult<ContractVersion> {
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;
    let requires_review = import_source == ImportSource::OdcsStripped;

    let _ = get_contract_identity(pool, contract_id).await?;

    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at, compliance_mode,
             import_source, requires_review)
        VALUES ($1, $2, $3, 'draft', $4, NOW(), $5, $6, $7)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
        "#,
    )
    .bind(id)
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
    .bind(import_source.as_str())
    .bind(requires_review)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::VersionConflict {
                contract_id,
                version: version.to_string(),
            }
        }
        _ => AppError::from(e),
    })?;

    row.into_version()
}

/// Clear the `requires_review` flag set by a stripped ODCS import (D-002).
/// Called by the `POST /contracts/:id/versions/:version/approve-import` handler.
/// Only valid on draft versions; stable/deprecated versions are immutable.
pub async fn clear_requires_review(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
) -> AppResult<ContractVersion> {
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
        SET requires_review = FALSE
        WHERE contract_id = $1 AND version = $2
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  import_source, requires_review
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_one(pool)
    .await
    .db_op("clear_requires_review")?;

    row.into_version()
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
///
/// `payload` is [`TransformedPayload`] (RFC-004 §6) — raw PII never reaches
/// `quarantine_events.payload`.  Callers that already hold a stored value
/// (replay re-quarantine, etc.) can wrap it via
/// [`TransformedPayload::from_stored`].
#[allow(clippy::too_many_arguments, dead_code)]
pub async fn quarantine_event(
    pool: &PgPool,
    contract_id: Uuid,
    contract_version: &str,
    payload: TransformedPayload,
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
    .bind(payload.into_inner())
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
///
/// `raw_event` is [`TransformedPayload`] (RFC-004 §6) — raw PII never
/// reaches `audit_log.raw_event`.  Summary-audit callers that carry
/// synthetic bookkeeping JSON (batch-rejected, deprecated-pin) wrap via
/// [`TransformedPayload::from_stored`].
#[allow(clippy::too_many_arguments)]
pub async fn log_audit_entry(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
    contract_version: &str,
    passed: bool,
    violation_count: i32,
    violation_details: serde_json::Value,
    raw_event: TransformedPayload,
    validation_us: i64,
    source_ip: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, org_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(contract_id)
    .bind(org_id)
    .bind(contract_version)
    .bind(passed)
    .bind(violation_count)
    .bind(violation_details)
    .bind(raw_event.into_inner())
    .bind(validation_us)
    .bind(source_ip)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch recent audit entries for the dashboard monitor.
/// `org_id` scopes results to one org when Some; `contract_id` further filters
/// to one contract within that org. Pass `org_id = None` in dev-mode only.
pub async fn recent_audit_entries(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<AuditEntry>> {
    let rows = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                WHERE org_id = $1 AND contract_id = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(oid)
            .bind(cid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, AuditEntry>(
                r#"
                SELECT id, contract_id, contract_version, passed, violation_count,
                       violation_details, raw_event, validation_us, source_ip, created_at
                FROM audit_log
                WHERE org_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(oid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(cid)) => {
            // Dev-mode: no org, but still filter by contract
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
        }
        (None, None) => {
            // Dev-mode: no scoping at all
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
        }
    };

    Ok(rows)
}

/// Fetch summary statistics for the dashboard monitor, including p50/p95/p99 latency.
///
/// `org_id` scopes all aggregates to one org when Some.  Pass `None` only in
/// dev-mode (no auth configured).  `contract_id` further narrows within the org.
pub async fn ingestion_stats(
    pool: &PgPool,
    org_id: Option<Uuid>,
    contract_id: Option<Uuid>,
) -> AppResult<IngestionStats> {
    // -----------------------------------------------------------------------
    // Query 1: aggregate counts + average latency
    // -----------------------------------------------------------------------
    let stats: StatsRow = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE org_id = $1 AND contract_id = $2"#,
            )
            .bind(oid)
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE org_id = $1"#,
            )
            .bind(oid)
            .fetch_one(pool)
            .await?
        }
        (None, Some(cid)) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log WHERE contract_id = $1"#,
            )
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, StatsRow>(
                r#"SELECT COUNT(*)::bigint AS total,
                      COALESCE(SUM(CASE WHEN passed THEN 1 ELSE 0 END), 0)::bigint AS passed,
                      COALESCE(AVG(validation_us::float8), 0.0) AS avg_us
               FROM audit_log"#,
            )
            .fetch_one(pool)
            .await?
        }
    };

    let total = stats.total.unwrap_or(0);
    let passed = stats.passed.unwrap_or(0);
    let avg_us = stats.avg_us.unwrap_or(0.0);

    // -----------------------------------------------------------------------
    // Query 2: percentile latencies (p50 / p95 / p99)
    // -----------------------------------------------------------------------
    let perc: PercRow = match (org_id, contract_id) {
        (Some(oid), Some(cid)) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE org_id = $1 AND contract_id = $2"#,
            )
            .bind(oid)
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (Some(oid), None) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE org_id = $1"#,
            )
            .bind(oid)
            .fetch_one(pool)
            .await?
        }
        (None, Some(cid)) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log WHERE contract_id = $1"#,
            )
            .bind(cid)
            .fetch_one(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, PercRow>(
                r#"SELECT percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50,
                      percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95,
                      percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99
               FROM audit_log"#,
            )
            .fetch_one(pool)
            .await?
        }
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
///
/// `raw_event` is typed as [`TransformedPayload`] (RFC-004 §6) so the
/// compiler enforces "raw PII never reaches `audit_log`".  The only legal
/// ways to produce a value are [`crate::transform::apply_transforms`] at
/// the ingest boundary or [`TransformedPayload::from_stored`] for rows
/// whose payload was already durable-stored in post-transform form
/// (replay, summary audits).
#[derive(Debug, Clone)]
pub struct AuditEntryInsert {
    pub contract_id: Uuid,
    /// Org that owns this contract. None in dev-mode only.
    pub org_id: Option<Uuid>,
    pub contract_version: String,
    pub passed: bool,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub raw_event: TransformedPayload,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    /// Optional app-generated UUID.  Replay uses this so the caller can
    /// link the source quarantine row to the exact audit row it produced
    /// *before* the INSERT round-trip completes.  Fresh ingest leaves
    /// this `None` and lets Postgres generate a UUID via
    /// `uuid_generate_v4()`.
    pub pre_assigned_id: Option<Uuid>,
    /// For replay-pass audit rows: the source quarantine row that was
    /// re-validated.  NULL on fresh ingest.  RFC-003.
    pub replay_of_quarantine_id: Option<Uuid>,
}

/// One row to insert into `quarantine_events`.  Only failed events get one.
///
/// `contract_version` is the version that rejected the event.
///
/// `payload` is [`TransformedPayload`] — same invariant as
/// `AuditEntryInsert::raw_event`, RFC-004 §6.
#[derive(Debug, Clone)]
pub struct QuarantineEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: TransformedPayload,
    pub violation_count: i32,
    pub violation_details: serde_json::Value,
    pub validation_us: i64,
    pub source_ip: Option<String>,
    /// For quarantine rows created by a *failed* replay attempt: the
    /// source quarantine row whose payload we re-validated.  NULL for
    /// ingest-time quarantine rows.  RFC-003.
    pub replay_of_quarantine_id: Option<Uuid>,
}

/// One row to insert into `forwarded_events`.  Only passing events get one.
///
/// `contract_version` is the version that accepted the event.
///
/// `payload` is [`TransformedPayload`] so the forward destination also
/// only ever sees the post-transform form (RFC-004 §6).
#[derive(Debug, Clone)]
pub struct ForwardEventInsert {
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: TransformedPayload,
}

/// Batch-insert audit log entries in a single round-trip.
///
/// Uses `UNNEST` of typed arrays so one SQL statement handles the whole batch
/// regardless of size.  Rows in the input slice keep their order; the
/// database assigns each a fresh UUID via `uuid_generate_v4()`.
pub async fn log_audit_entries_batch(pool: &PgPool, entries: &[AuditEntryInsert]) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    // Split the struct-of-arrays columns for UNNEST.  Columns are aligned
    // positionally — every Vec is the same length as `entries`.
    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    // org_id: None entries use nil UUID as sentinel; NULLIF converts to SQL NULL.
    let org_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.org_id.unwrap_or(Uuid::nil()))
        .collect();
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    let passed: Vec<bool> = entries.iter().map(|e| e.passed).collect();
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.violation_details.clone())
        .collect();
    // RFC-004 §6: extract the underlying JSON only at the SQL bind boundary.
    let raw_events: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.raw_event.as_value().clone())
        .collect();
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();
    let pre_assigned_ids: Vec<Uuid> = entries
        .iter()
        .map(|e| e.pre_assigned_id.unwrap_or(Uuid::nil()))
        .collect();
    let replay_of: Vec<Uuid> = entries
        .iter()
        .map(|e| e.replay_of_quarantine_id.unwrap_or(Uuid::nil()))
        .collect();

    sqlx::query(
        r#"
        INSERT INTO audit_log
            (id, contract_id, org_id, contract_version, passed, violation_count,
             violation_details, raw_event, validation_us, source_ip,
             replay_of_quarantine_id, created_at)
        SELECT
            COALESCE(NULLIF(pre_assigned_id, '00000000-0000-0000-0000-000000000000'::uuid),
                     uuid_generate_v4()),
            contract_id,
            NULLIF(org_id, '00000000-0000-0000-0000-000000000000'::uuid),
            contract_version, passed, violation_count,
            violation_details, raw_event, validation_us,
            NULLIF(source_ip, ''),
            NULLIF(replay_of_quarantine_id, '00000000-0000-0000-0000-000000000000'::uuid),
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::uuid[], $3::text[], $4::bool[], $5::int[], $6::jsonb[],
            $7::jsonb[], $8::bigint[], $9::text[], $10::uuid[], $11::uuid[]
        ) AS t(contract_id, org_id, contract_version, passed, violation_count,
               violation_details, raw_event, validation_us, source_ip,
               pre_assigned_id, replay_of_quarantine_id)
        "#,
    )
    .bind(&contract_ids)
    .bind(&org_ids)
    .bind(&contract_versions)
    .bind(&passed)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&raw_events)
    .bind(&validation_us)
    .bind(&source_ips)
    .bind(&pre_assigned_ids)
    .bind(&replay_of)
    .execute(pool)
    .await
    .db_op("log_audit_entries_batch")?;

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
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    // RFC-004 §6: same pattern as `log_audit_entries_batch` — unwrap to
    // `Value` only at the SQL boundary.
    let payloads: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.payload.as_value().clone())
        .collect();
    let violation_counts: Vec<i32> = entries.iter().map(|e| e.violation_count).collect();
    let violation_details: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.violation_details.clone())
        .collect();
    let validation_us: Vec<i64> = entries.iter().map(|e| e.validation_us).collect();
    let source_ips: Vec<String> = entries
        .iter()
        .map(|e| e.source_ip.clone().unwrap_or_default())
        .collect();
    // Failed-replay rows link back to their source quarantine row; fresh
    // ingest rows pass NULL (sentinel zero UUID → NULLIF).
    let replay_of: Vec<Uuid> = entries
        .iter()
        .map(|e| e.replay_of_quarantine_id.unwrap_or(Uuid::nil()))
        .collect();

    sqlx::query(
        r#"
        INSERT INTO quarantine_events
            (id, contract_id, contract_version, payload, violation_count,
             violation_details, validation_us, source_ip,
             replay_of_quarantine_id, status, created_at)
        SELECT
            uuid_generate_v4(),
            contract_id, contract_version, payload, violation_count,
            violation_details, validation_us,
            NULLIF(source_ip, ''),
            NULLIF(replay_of_quarantine_id, '00000000-0000-0000-0000-000000000000'::uuid),
            'pending',
            NOW()
        FROM UNNEST(
            $1::uuid[], $2::text[], $3::jsonb[], $4::int[], $5::jsonb[],
            $6::bigint[], $7::text[], $8::uuid[]
        ) AS t(contract_id, contract_version, payload, violation_count,
               violation_details, validation_us, source_ip,
               replay_of_quarantine_id)
        "#,
    )
    .bind(&contract_ids)
    .bind(&contract_versions)
    .bind(&payloads)
    .bind(&violation_counts)
    .bind(&violation_details)
    .bind(&validation_us)
    .bind(&source_ips)
    .bind(&replay_of)
    .execute(pool)
    .await
    .db_op("quarantine_events_batch")?;

    Ok(())
}

/// Batch-insert forwarded events in a single round-trip.
///
/// Unlike the other two helpers this one is awaited inline from the ingest
/// handler so the response can mark individual events as `forwarded: true`.
pub async fn forward_events_batch(pool: &PgPool, entries: &[ForwardEventInsert]) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let contract_ids: Vec<Uuid> = entries.iter().map(|e| e.contract_id).collect();
    let contract_versions: Vec<String> =
        entries.iter().map(|e| e.contract_version.clone()).collect();
    // RFC-004 §6: forward destination only ever sees post-transform payloads.
    let payloads: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| e.payload.as_value().clone())
        .collect();

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
    .await
    .db_op("forward_events_batch")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Replay (RFC-003) — manual Replay Quarantine
// ---------------------------------------------------------------------------

/// A quarantine row as loaded by the replay handler.  Carries just enough to
/// categorize the row (not_found / wrong_contract / purged / already_replayed
/// / eligible) and re-validate the payload under a target version.
///
/// `contract_version`, `replayed_into_audit_id`, and `created_at` are read
/// back for future use (dashboard drawer, audit export) even though the
/// current replay handler doesn't dispatch on them.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct QuarantineRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub contract_version: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub replayed_into_audit_id: Option<Uuid>,
    pub source_ip: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct QuarantineRowRaw {
    id: Uuid,
    contract_id: Uuid,
    contract_version: String,
    payload: serde_json::Value,
    status: String,
    replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    replayed_into_audit_id: Option<Uuid>,
    source_ip: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<QuarantineRowRaw> for QuarantineRow {
    fn from(r: QuarantineRowRaw) -> Self {
        Self {
            id: r.id,
            contract_id: r.contract_id,
            contract_version: r.contract_version,
            payload: r.payload,
            status: r.status,
            replayed_at: r.replayed_at,
            replayed_into_audit_id: r.replayed_into_audit_id,
            source_ip: r.source_ip,
            created_at: r.created_at,
        }
    }
}

/// Load quarantine rows by ID.  Rows missing from the result set are
/// surfaced by the caller as `not_found` — this helper simply returns the
/// subset that exists.
///
/// Preserves no particular order; the handler re-keys by ID.
pub async fn list_quarantine_by_ids(pool: &PgPool, ids: &[Uuid]) -> AppResult<Vec<QuarantineRow>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<QuarantineRowRaw> = sqlx::query_as(
        r#"
        SELECT
            id, contract_id, contract_version, payload, status,
            replayed_at, replayed_into_audit_id, source_ip, created_at
        FROM quarantine_events
        WHERE id = ANY($1::uuid[])
        "#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(QuarantineRow::from).collect())
}

/// Mark a batch of quarantine rows as replayed, each linked to the specific
/// audit_log row its payload produced on success.
///
/// The UPDATE is **conditional**: it only stamps rows whose `replayed_at` is
/// still NULL and whose status is `pending` or `reviewed`.  This is the race
/// guard — if two concurrent replay calls target the same source row, at
/// most one UPDATE touches it; the other sees the stamp on a re-read and
/// surfaces `already_replayed` to its caller.
///
/// Returns the set of source IDs that were successfully marked.  Any IDs in
/// `pairs` missing from the returned vec lost the race and should be
/// re-categorized by the caller.
pub async fn mark_quarantine_replayed_batch(
    pool: &PgPool,
    pairs: &[(Uuid, Uuid)], // (source_quarantine_id, new_audit_id)
    replayed_at: chrono::DateTime<chrono::Utc>,
) -> AppResult<Vec<Uuid>> {
    if pairs.is_empty() {
        return Ok(vec![]);
    }

    let source_ids: Vec<Uuid> = pairs.iter().map(|(s, _)| *s).collect();
    let new_audit_ids: Vec<Uuid> = pairs.iter().map(|(_, a)| *a).collect();

    let updated: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE quarantine_events qe
        SET status = 'replayed',
            replayed_at = $3,
            replayed_into_audit_id = t.new_audit_id
        FROM UNNEST($1::uuid[], $2::uuid[]) AS t(source_id, new_audit_id)
        WHERE qe.id = t.source_id
          AND qe.status IN ('pending', 'reviewed')
          AND qe.replayed_at IS NULL
        RETURNING qe.id
        "#,
    )
    .bind(&source_ids)
    .bind(&new_audit_ids)
    .bind(replayed_at)
    .fetch_all(pool)
    .await?;

    Ok(updated.into_iter().map(|(id,)| id).collect())
}

/// One entry in the replay-history chain returned by
/// `GET /contracts/:id/quarantine/:quar_id/replay-history`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayHistoryEntry {
    /// The original quarantine row the replay chain starts from.
    Source {
        id: Uuid,
        contract_version: String,
        status: String,
        violation_count: i32,
        replayed_at: Option<chrono::DateTime<chrono::Utc>>,
        replayed_into_audit_id: Option<Uuid>,
        created_at: chrono::DateTime<chrono::Utc>,
    },
    /// A quarantine row created by a failed replay attempt.
    FailedReplay {
        id: Uuid,
        contract_version: String,
        violation_count: i32,
        created_at: chrono::DateTime<chrono::Utc>,
    },
    /// The audit_log row a successful replay attempt produced.
    PassedReplay {
        id: Uuid,
        contract_version: String,
        created_at: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(sqlx::FromRow)]
struct SourceHistoryRow {
    id: Uuid,
    contract_version: String,
    status: String,
    violation_count: i32,
    replayed_at: Option<chrono::DateTime<chrono::Utc>>,
    replayed_into_audit_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct FailedReplayRow {
    id: Uuid,
    contract_version: String,
    violation_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct PassedReplayRow {
    id: Uuid,
    contract_version: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Return the full replay history for a given quarantine row: the source row,
/// every failed-replay child linking back to it, and the terminal audit_log
/// row if a replay eventually passed.
///
/// The caller is expected to have already verified that `source_id` belongs
/// to `contract_id` (or to return 404 if not).
pub async fn replay_history_for(
    pool: &PgPool,
    contract_id: Uuid,
    source_id: Uuid,
) -> AppResult<Vec<ReplayHistoryEntry>> {
    let source: Option<SourceHistoryRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, status, violation_count, replayed_at,
               replayed_into_audit_id, created_at
        FROM quarantine_events
        WHERE id = $1 AND contract_id = $2
        "#,
    )
    .bind(source_id)
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;

    let source = match source {
        Some(r) => r,
        None => {
            return Err(AppError::BadRequest(format!(
                "quarantine row {source_id} not found on contract {contract_id}"
            )));
        }
    };

    // Failed-replay children.
    let failed: Vec<FailedReplayRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, violation_count, created_at
        FROM quarantine_events
        WHERE replay_of_quarantine_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    // Passed-replay audit row (if any).
    let passed: Vec<PassedReplayRow> = sqlx::query_as(
        r#"
        SELECT id, contract_version, created_at
        FROM audit_log
        WHERE replay_of_quarantine_id = $1 AND passed = true
        ORDER BY created_at ASC
        "#,
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    let mut out: Vec<ReplayHistoryEntry> = Vec::with_capacity(1 + failed.len() + passed.len());
    out.push(ReplayHistoryEntry::Source {
        id: source.id,
        contract_version: source.contract_version,
        status: source.status,
        violation_count: source.violation_count,
        replayed_at: source.replayed_at,
        replayed_into_audit_id: source.replayed_into_audit_id,
        created_at: source.created_at,
    });
    for r in failed {
        out.push(ReplayHistoryEntry::FailedReplay {
            id: r.id,
            contract_version: r.contract_version,
            violation_count: r.violation_count,
            created_at: r.created_at,
        });
    }
    for r in passed {
        out.push(ReplayHistoryEntry::PassedReplay {
            id: r.id,
            contract_version: r.contract_version,
            created_at: r.created_at,
        });
    }
    Ok(out)
}
