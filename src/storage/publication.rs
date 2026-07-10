//! Contract publication (RFC-028) + public catalog fork/import (RFC-034).
//!
//! Split out of the original monolithic `storage.rs` (2026-07-10, RFC/worklist
//! item 3). `import_published_contract` spans into both `storage::contracts`
//! (needs `ContractVersionRow`/`get_contract_identity`/`get_version`) and
//! `storage::collaboration` (`ensure_viewer_collaborator`) — referenced via
//! explicit sibling paths rather than glob imports to keep the cross-module
//! dependency visible at a glance.

use super::collaboration::ensure_viewer_collaborator;
use super::contracts::{get_contract_identity, get_version, ContractVersionRow};
use crate::contract::{Contract, ContractVersion, ImportMode, PublicationVisibility};
use crate::error::{AppError, AppResult, DbOpContext};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Publication storage (RFC-032)
// ---------------------------------------------------------------------------

/// Row type for `contract_publications`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct PublicationRow {
    pub publication_ref: String,
    pub contract_id: Uuid,
    pub version_id: Uuid,
    pub contract_name: String,
    pub contract_version: String,
    pub yaml_content: String,
    pub visibility: String,
    pub link_token: Option<String>,
    pub org_id: Option<Uuid>,
    pub published_by: Option<String>,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl PublicationRow {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn visibility_parsed(&self) -> Option<PublicationVisibility> {
        self.visibility.parse().ok()
    }
}

/// Create a new publication for a contract version.
///
/// `link_token` must be `Some` when `visibility == PublicationVisibility::Link`.
pub async fn publish_contract_version(
    pool: &PgPool,
    contract_id: Uuid,
    version_str: &str,
    visibility: PublicationVisibility,
    link_token: Option<&str>,
    org_id: Option<Uuid>,
    published_by: Option<&str>,
) -> AppResult<PublicationRow> {
    // Resolve the version row (org-scoped — RFC-047).
    let cv = get_version(pool, contract_id, version_str, org_id).await?;

    // Fetch the contract name for denormalization (org already verified above).
    let identity = get_contract_identity(pool, contract_id, org_id).await?;

    let row = sqlx::query_as::<_, PublicationRow>(
        r#"
        INSERT INTO contract_publications
            (ref, contract_id, version_id, contract_name, contract_version,
             yaml_content, visibility, link_token, org_id, published_by, published_at)
        VALUES (
            encode(gen_random_bytes(12), 'hex'),
            $1, $2, $3, $4, $5, $6, $7, $8, $9, NOW()
        )
        RETURNING
            ref      AS publication_ref,
            contract_id,
            version_id,
            contract_name,
            contract_version,
            yaml_content,
            visibility,
            link_token,
            org_id,
            published_by,
            published_at,
            revoked_at
        "#,
    )
    .bind(contract_id)
    .bind(cv.id)
    .bind(&identity.name)
    .bind(version_str)
    .bind(&cv.yaml_content)
    .bind(visibility.as_str())
    .bind(link_token)
    .bind(org_id)
    .bind(published_by)
    .fetch_one(pool)
    .await
    .db_op("publish_contract_version")?;

    Ok(row)
}

/// Soft-delete (revoke) a publication.  Only the org that published it may revoke it.
pub async fn revoke_publication(
    pool: &PgPool,
    publication_ref: &str,
    org_id: Option<Uuid>,
) -> AppResult<PublicationRow> {
    let row = sqlx::query_as::<_, PublicationRow>(
        r#"
        UPDATE contract_publications
        SET revoked_at = NOW()
        WHERE ref = $1
          AND ($2::uuid IS NULL OR org_id = $2)
          AND revoked_at IS NULL
        RETURNING
            ref      AS publication_ref,
            contract_id,
            version_id,
            contract_name,
            contract_version,
            yaml_content,
            visibility,
            link_token,
            org_id,
            published_by,
            published_at,
            revoked_at
        "#,
    )
    .bind(publication_ref)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .db_op("revoke_publication")?
    .ok_or_else(|| {
        AppError::BadRequest(format!(
            "publication '{}' not found, already revoked, or not owned by this org",
            publication_ref
        ))
    })?;

    Ok(row)
}

/// Fetch a publication by ref.  Does NOT filter on revoked — callers check.
pub async fn get_publication(pool: &PgPool, publication_ref: &str) -> AppResult<PublicationRow> {
    let row = sqlx::query_as::<_, PublicationRow>(
        r#"
        SELECT
            ref      AS publication_ref,
            contract_id,
            version_id,
            contract_name,
            contract_version,
            yaml_content,
            visibility,
            link_token,
            org_id,
            published_by,
            published_at,
            revoked_at
        FROM contract_publications
        WHERE ref = $1
        "#,
    )
    .bind(publication_ref)
    .fetch_optional(pool)
    .await
    .db_op("get_publication")?
    .ok_or_else(|| AppError::BadRequest(format!("publication '{}' not found", publication_ref)))?;

    Ok(row)
}

/// Lightweight row for public catalog listings — omits yaml_content for efficiency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct CatalogRow {
    pub publication_ref: String,
    pub contract_name: String,
    pub contract_version: String,
    pub published_by: Option<String>,
    pub published_at: chrono::DateTime<chrono::Utc>,
}

/// Return up to `limit` public (non-revoked) publications, newest first.
pub async fn list_public_catalog(pool: &PgPool, limit: i64) -> AppResult<Vec<CatalogRow>> {
    let rows = sqlx::query_as::<_, CatalogRow>(
        r#"
        SELECT
            ref      AS publication_ref,
            contract_name,
            contract_version,
            published_by,
            published_at
        FROM contract_publications
        WHERE visibility = 'public'
          AND revoked_at IS NULL
        ORDER BY published_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .db_op("list_public_catalog")?;

    Ok(rows)
}

/// Import a published contract into the caller's org.
///
/// Creates a new contract identity + a draft version from the publication's
/// YAML, recording provenance (`imported_from_ref`, `import_mode`,
/// `imported_at`) on the `contracts` row.
pub async fn import_published_contract(
    pool: &PgPool,
    pub_row: &PublicationRow,
    import_mode: ImportMode,
    org_id: Option<Uuid>,
) -> AppResult<ContractVersion> {
    let parsed: Contract = serde_yaml::from_str(&pub_row.yaml_content)
        .map_err(|e| AppError::InvalidContractYaml(e.to_string()))?;

    let compliance_mode = parsed.compliance_mode;
    let egress_leakage_mode = parsed.egress_leakage_mode.as_str();

    let contract_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let description = parsed.description.as_deref();

    let mut tx = pool.begin().await.db_op("import_published:begin")?;

    // Insert identity with provenance.
    sqlx::query(
        r#"
        INSERT INTO contracts
            (id, org_id, name, description, multi_stable_resolution,
             imported_from_ref, import_mode, imported_at,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, 'strict', $5, $6, NOW(), NOW(), NOW())
        "#,
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(&pub_row.contract_name)
    .bind(description)
    .bind(&pub_row.publication_ref)
    .bind(import_mode.as_str())
    .execute(&mut *tx)
    .await
    .db_op("import_published:insert_identity")?;

    // Insert draft version from the publication YAML.
    let version_row = sqlx::query_as::<_, ContractVersionRow>(
        r#"
        INSERT INTO contract_versions
            (id, contract_id, version, state, yaml_content, created_at,
             compliance_mode, egress_leakage_mode, import_source, requires_review)
        VALUES ($1, $2, $3, 'draft', $4, NOW(), $5, $6, 'publication', FALSE)
        RETURNING id, contract_id, version, state, yaml_content,
                  created_at, promoted_at, deprecated_at, compliance_mode,
                  egress_leakage_mode, import_source, requires_review
        "#,
    )
    .bind(version_id)
    .bind(contract_id)
    .bind(&pub_row.contract_version)
    .bind(&pub_row.yaml_content)
    .bind(compliance_mode)
    .bind(egress_leakage_mode)
    .fetch_one(&mut *tx)
    .await
    .db_op("import_published:insert_version")?;

    tx.commit().await.db_op("import_published:commit")?;

    // RFC-033: if the publication has `org` visibility, grant the importing org
    // a viewer collaborator role on the contract so it can access the
    // collaboration surface without a separate invite.
    if pub_row.visibility == "org" {
        if let Some(importer_org) = org_id {
            // granted_by = the publication's owning org (or the importer if unknown).
            let granting_org = pub_row.org_id.unwrap_or(importer_org);
            // Best-effort — don't fail the import if this insert errors.
            let _ = ensure_viewer_collaborator(
                pool,
                &pub_row.contract_name,
                importer_org,
                granting_org,
            )
            .await;
        }
    }

    version_row.into_version()
}

/// For a `subscribe` import: check whether the source publication has a newer
/// version available than the one the consumer imported.
///
/// Returns `(current_published_version, update_available, source_revoked)`.
pub async fn check_import_status(
    pool: &PgPool,
    contract_id: Uuid,
) -> AppResult<ImportStatusResult> {
    // Load the contract identity to get provenance fields.
    #[derive(sqlx::FromRow)]
    struct ProvenanceRow {
        imported_from_ref: Option<String>,
        import_mode: Option<String>,
    }

    let prov = sqlx::query_as::<_, ProvenanceRow>(
        r#"
        SELECT imported_from_ref, import_mode
        FROM contracts
        WHERE id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await
    .db_op("check_import_status:load_provenance")?
    .ok_or(AppError::ContractNotFound(contract_id))?;

    let publication_ref = match prov.imported_from_ref {
        Some(r) => r,
        None => {
            return Ok(ImportStatusResult {
                import_mode: None,
                publication_ref: None,
                source_revoked: false,
                update_available: false,
                latest_published_version: None,
                imported_version: None,
            });
        }
    };

    let import_mode = prov
        .import_mode
        .as_deref()
        .and_then(|s| s.parse::<ImportMode>().ok());

    // Load the publication to check its state.
    let pub_row = get_publication(pool, &publication_ref).await?;

    // The "imported" version is the version stored on the consumer's draft.
    let imported_version = get_latest_draft_version(pool, contract_id).await?;

    let update_available = if pub_row.is_revoked() {
        false
    } else {
        // A newer version is available when the publication's version differs
        // from what we imported.
        imported_version
            .as_deref()
            .map(|iv| iv != pub_row.contract_version)
            .unwrap_or(false)
    };

    Ok(ImportStatusResult {
        import_mode,
        publication_ref: Some(publication_ref),
        source_revoked: pub_row.is_revoked(),
        update_available,
        latest_published_version: Some(pub_row.contract_version),
        imported_version,
    })
}

/// Result of `check_import_status`.
#[derive(Debug, serde::Serialize)]
pub struct ImportStatusResult {
    pub import_mode: Option<ImportMode>,
    pub publication_ref: Option<String>,
    pub source_revoked: bool,
    pub update_available: bool,
    pub latest_published_version: Option<String>,
    pub imported_version: Option<String>,
}

// ---------------------------------------------------------------------------

/// Return the version string of the latest (most recently created) draft for
/// the contract, if any.
async fn get_latest_draft_version(pool: &PgPool, contract_id: Uuid) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT version FROM contract_versions
        WHERE contract_id = $1 AND state = 'draft'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await
    .db_op("get_latest_draft_version")?;

    Ok(row.map(|(v,)| v))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // RFC-069: PublicationRow::is_revoked reflects revoked_at presence.
    fn publication_row(revoked: bool) -> PublicationRow {
        PublicationRow {
            publication_ref: "pub_test".into(),
            contract_id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
            contract_name: "user_events".into(),
            contract_version: "1.0".into(),
            yaml_content: String::new(),
            visibility: "public".into(),
            link_token: None,
            org_id: None,
            published_by: None,
            published_at: chrono::Utc::now(),
            revoked_at: revoked.then(chrono::Utc::now),
        }
    }

    #[test]
    fn is_revoked_true_when_revoked_at_set() {
        assert!(publication_row(true).is_revoked());
    }

    #[test]
    fn is_revoked_false_when_revoked_at_none() {
        assert!(!publication_row(false).is_revoked());
    }
}
