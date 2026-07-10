//! Contract identity + version CRUD (RFC-002).
//!
//! Split out of the original monolithic `storage.rs` (2026-07-10, RFC/worklist
//! item 3). Kept identity + version together in one file rather than
//! splitting further: `create_contract` and `deploy_contract_version` both
//! need `ContractIdentityRow` *and* `ContractVersionRow` in the same
//! transaction, so separating them would only replace in-file privacy with
//! `pub(super)` cross-file coupling for no real gain. `ContractVersionRow`
//! (struct + `into_version`) is `pub(super)` because `storage::publication`
//! also needs it for the publication-import flow.

use crate::contract::{
    Contract, ContractIdentity, ContractSummary, ContractVersion, EgressLeakageMode,
    ImportSource, MultiStableResolution, NameHistoryEntry, VersionState, VersionSummary,
};
use crate::error::{AppError, AppResult, DbOpContext};
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
pub(super) struct ContractVersionRow {
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
    /// RFC-030: denormalized for SQL-level filtering.  YAML is authoritative;
    /// synced to `Contract::egress_leakage_mode` at INSERT / UPDATE time.
    egress_leakage_mode: String,
    /// Migration 010: ODCS import provenance.
    import_source: String,
    /// Migration 010: blocks promotion until human review clears it.
    requires_review: bool,
}

impl ContractVersionRow {
    pub(super) fn into_version(self) -> AppResult<ContractVersion> {
        let state = self.state.parse::<VersionState>().map_err(|e| {
            AppError::Internal(format!("invalid contract_versions.state in DB: {e}"))
        })?;
        let import_source = self
            .import_source
            .parse::<ImportSource>()
            .map_err(|e| AppError::Internal(format!("invalid import_source in DB: {e}")))?;
        let egress_leakage_mode = self
            .egress_leakage_mode
            .parse::<EgressLeakageMode>()
            .map_err(|e| AppError::Internal(format!("invalid egress_leakage_mode in DB: {e}")))?;
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
            egress_leakage_mode,
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
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();

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
             egress_leakage_mode, import_source, requires_review)
        VALUES ($1, $2, '1.0.0', 'draft', $3, NOW(), $4, $5, 'native', FALSE)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(version_id)
    .bind(contract_id)
    .bind(yaml_content)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
    .fetch_one(&mut *tx)
    .await
    .db_op("create_contract:insert_initial_version")?;

    tx.commit().await.db_op("create_contract:commit")?;

    Ok((identity_row.into_identity()?, version_row.into_version()?))
}

/// Fetch a contract identity by id.  Soft-deleted contracts are not visible
/// (RFC-001 sign-off #6: soft delete everywhere).
///
/// `org_id` — when `Some`, the row must belong to that org or 404 is returned
/// (RFC-047 app-level BOLA guard).  Pass `None` only in dev mode or from
/// internal callers that have already verified ownership.
pub async fn get_contract_identity(
    pool: &PgPool,
    id: Uuid,
    org_id: Option<Uuid>,
) -> AppResult<ContractIdentity> {
    let row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        SELECT id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
        FROM contracts
        WHERE id = $1 AND deleted_at IS NULL AND ($2 IS NULL OR org_id = $2)
        "#,
    )
    .bind(id)
    .bind(org_id)
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
            WHERE c.org_id = $1 AND c.deleted_at IS NULL
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
            WHERE c.deleted_at IS NULL
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
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn patch_contract_identity(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    resolution: Option<MultiStableResolution>,
    org_id: Option<Uuid>,
) -> AppResult<ContractIdentity> {
    // COALESCE keeps the existing value when the bind is NULL.  Resolution
    // is bound as a string with an explicit NULL when not provided.
    // $5 = org_id: row is invisible (fetch_optional → None → 404) when the
    // caller's org doesn't own it — never leaks existence via 403 (RFC-047).
    let row = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        UPDATE contracts
        SET
            name                    = COALESCE($2, name),
            description             = COALESCE($3, description),
            multi_stable_resolution = COALESCE($4, multi_stable_resolution),
            updated_at              = NOW()
        WHERE id = $1 AND ($5 IS NULL OR org_id = $5)
        RETURNING id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(resolution.map(|r| r.as_str()))
    .bind(org_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::ContractNotFound(id))?;

    row.into_identity()
}

/// Soft-delete a contract (RFC-001 §6).  `org_id` scope guard (RFC-047):
/// returns `ContractNotFound` when no row matches id + org, so UUID
/// existence is never leaked to the wrong tenant.
pub async fn delete_contract(pool: &PgPool, id: Uuid, org_id: Option<Uuid>) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE contracts SET deleted_at = NOW() \
         WHERE id = $1 AND deleted_at IS NULL AND ($2 IS NULL OR org_id = $2)",
    )
    .bind(id)
    .bind(org_id)
    .execute(pool)
    .await?;

    // 0 rows: either doesn't exist, already deleted, or belongs to another
    // org.  All map to 404 so we never reveal UUID existence (RFC-047).
    if result.rows_affected() == 0 {
        return Err(AppError::ContractNotFound(id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Contract version CRUD (RFC-002)
// ---------------------------------------------------------------------------

/// Create a new draft version on an existing contract.  The YAML is parsed
/// first so invalid contracts never land in the DB.
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn create_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();

    // Ensure the contract exists and belongs to the caller's org (RFC-047).
    let _ = get_contract_identity(pool, contract_id, org_id).await?;

    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at, compliance_mode,
             egress_leakage_mode, import_source, requires_review)
        VALUES ($1, $2, $3, 'draft', $4, NOW(), $5, $6, 'native', FALSE)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(id)
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
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
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn patch_version_yaml(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    // Parse first — reject invalid YAML before touching the DB.  We also
    // extract `compliance_mode` so the column stays in sync on UPDATE.  The
    // DB trigger `contract_versions_compliance_mode_guard` will reject any
    // change to `compliance_mode` once the version leaves draft, so this is
    // safe to always bind (RFC-004).
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();

    // Fetch first so we can emit a specific error (not-found vs. immutable).
    // org_id check happens inside get_version (RFC-047).
    let current = get_version(pool, contract_id, version, org_id).await?;
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
            compliance_mode = $4,
            egress_leakage_mode = $5
        WHERE contract_id = $1 AND version = $2
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Transition draft → stable.  Rejects any other source state.
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn promote_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let current = get_version(pool, contract_id, version, org_id).await?;
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
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Transition stable → deprecated.  Rejects any other source state.
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn deprecate_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let current = get_version(pool, contract_id, version, org_id).await?;
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
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .fetch_one(pool)
    .await?;

    row.into_version()
}

/// Atomically deploy a contract version (RFC-028).
///
/// Steps (in one transaction):
///   1. Find or create the contract identity by name.
///   2. Guard: reject if any pending quarantine events exist for this contract.
///   3. Insert a new `stable` version with `parsed_json`, `source`,
///      `deployed_by`, and `deployed_at` populated.
///   4. Deprecate all other `stable` versions for this contract.
///
/// Returns the new version row and the count of versions deprecated.
///
/// Admin-only at the API layer — this function does not enforce roles itself.
pub async fn deploy_contract_version(
    pool: &PgPool,
    name: &str,
    yaml_content: &str,
    source: Option<&str>,
    deployed_by: Option<&str>,
    org_id: Option<Uuid>,
) -> AppResult<(ContractVersion, i64)> {
    // Parse YAML and extract version — fail fast before touching the DB.
    let parsed: crate::contract::Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let version = parsed.version.clone();
    let compliance_mode = parsed.compliance_mode;
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();
    let parsed_json = serde_json::to_value(&parsed)
        .map_err(|e| AppError::Internal(format!("contract json serialization: {e}")))?;

    let mut tx = pool.begin().await?;

    // ── 1. Find or create contract identity by name ───────────────────────────
    let maybe_identity = sqlx::query_as::<_, ContractIdentityRow>(
        r#"
        SELECT id, name, description, multi_stable_resolution, created_at, updated_at, pii_salt
        FROM contracts
        WHERE name = $1
        "#,
    )
    .bind(name)
    .fetch_optional(&mut *tx)
    .await?;

    let contract_id = match maybe_identity {
        Some(row) => row.id,
        None => {
            // Create a new contract identity.  pii_salt uses DB default gen_random_bytes(32).
            let id = Uuid::new_v4();
            sqlx::query(
                r#"
                INSERT INTO contracts (id, name, org_id)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(id)
            .bind(name)
            .bind(org_id)
            .execute(&mut *tx)
            .await
            .db_op("deploy_contract:create_identity")?;
            id
        }
    };

    // ── 2. Quarantine guard — refuse if pending events exist ──────────────────
    let pending_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM quarantine_events
        WHERE contract_id = $1
          AND status = 'pending'
        "#,
    )
    .bind(contract_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(0);

    if pending_count > 0 {
        return Err(AppError::BadRequest(format!(
            "contract '{}' has {} pending quarantine event(s); resolve them before deploying a new version",
            name, pending_count
        )));
    }

    // ── 3. Insert the new version as stable ───────────────────────────────────
    let version_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at,
             promoted_at, compliance_mode, egress_leakage_mode, import_source,
             requires_review, source, deployed_by, deployed_at, parsed_json)
        VALUES ($1, $2, $3, 'stable', $4, $5,
                $5, $6, $7, 'native', FALSE,
                $8, $9, $5, $10)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(version_id)
    .bind(contract_id)
    .bind(&version)
    .bind(yaml_content)
    .bind(now)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
    .bind(source)
    .bind(deployed_by)
    .bind(parsed_json)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
            AppError::VersionConflict {
                contract_id,
                version: version.clone(),
            }
        }
        _ => AppError::from(e),
    })?;

    // ── 4. Deprecate all other stable versions for this contract ──────────────
    let deprecated_result = sqlx::query(
        r#"
        UPDATE contract_versions
        SET state = 'deprecated', deprecated_at = NOW()
        WHERE contract_id = $1
          AND state = 'stable'
          AND id <> $2
        "#,
    )
    .bind(contract_id)
    .bind(version_id)
    .execute(&mut *tx)
    .await?;
    let deprecated_count = deprecated_result.rows_affected() as i64;

    tx.commit().await?;

    Ok((row.into_version()?, deprecated_count))
}

/// Delete a draft version.  Postgres trigger enforces the draft-only rule
/// as well — so even a direct SQL hit cannot remove a stable/deprecated row.
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode or
/// from internal callers that have already verified ownership.
pub async fn delete_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    org_id: Option<Uuid>,
) -> AppResult<()> {
    let current = get_version(pool, contract_id, version, org_id).await?;
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

/// Fetch a specific contract version.
///
/// `org_id` — when `Some`, scopes through the parent contract row so a
/// version that exists for another org is invisible (RFC-047).  Returns
/// `VersionNotFound` in all miss cases — never leaks UUID existence.
/// Pass `None` only from ingest/egress hot paths or after the contract
/// identity has already been org-verified.
pub async fn get_version(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at, compliance_mode,
               egress_leakage_mode, import_source, requires_review
        FROM contract_versions
        WHERE contract_id = $1 AND version = $2
          AND contract_id IN (
              SELECT id FROM contracts
              WHERE id = $1 AND ($3 IS NULL OR org_id = $3)
          )
        "#,
    )
    .bind(contract_id)
    .bind(version)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("get_version")?
    .ok_or_else(|| AppError::VersionNotFound {
        contract_id,
        version: version.to_string(),
    })?;

    row.into_version()
}

/// `org_id` — org-scope guard (RFC-047).  Pass `None` from internal helpers
/// that have already verified ownership (e.g. `identity_to_response`).
pub async fn list_versions(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
) -> AppResult<Vec<VersionSummary>> {
    // Ensure contract exists (and belongs to caller's org) so callers get a clean 404.
    let _ = get_contract_identity(pool, contract_id, org_id).await?;

    let rows = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        SELECT id, contract_id, version, state, yaml_content,
               created_at, promoted_at, deprecated_at, compliance_mode,
               egress_leakage_mode, import_source, requires_review
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
               egress_leakage_mode, import_source, requires_review
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
               egress_leakage_mode, import_source, requires_review
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
    let rows: Vec<(Uuid, Vec<u8>)> =
        sqlx::query_as(r#"SELECT id, pii_salt FROM contracts WHERE deleted_at IS NULL"#)
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
               egress_leakage_mode, import_source, requires_review
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
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn create_version_from_import(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    yaml_content: &str,
    import_source: ImportSource,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let parsed: Contract = serde_yaml::from_str(yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;
    let compliance_mode = parsed.compliance_mode;
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();
    let requires_review = import_source == ImportSource::OdcsStripped;

    let _ = get_contract_identity(pool, contract_id, org_id).await?;

    let id = Uuid::new_v4();

    let row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at, compliance_mode,
             egress_leakage_mode, import_source, requires_review)
        VALUES ($1, $2, $3, 'draft', $4, NOW(), $5, $6, $7, $8)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(id)
    .bind(contract_id)
    .bind(version)
    .bind(yaml_content)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
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
///
/// `org_id` — org-scope guard (RFC-047).  Pass `None` only in dev mode.
pub async fn clear_requires_review(
    pool: &PgPool,
    contract_id: Uuid,
    version: &str,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let current = get_version(pool, contract_id, version, org_id).await?;
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
                  egress_leakage_mode, import_source, requires_review
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

/// `org_id` — org-scope guard (RFC-047).  Verifies ownership before
/// returning history rows so cross-org UUID enumeration is blocked.
pub async fn list_name_history(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
) -> AppResult<Vec<NameHistoryEntry>> {
    // Ownership check — returns ContractNotFound if wrong org.
    let _ = get_contract_identity(pool, contract_id, org_id).await?;

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
