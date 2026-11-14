//! Contract families — a chain of contracts for one dataset as it changes shape.
//!
//! A family is a single chain. Each link names the contract it is derived from
//! (none on the root) and the pipeline boundary it sits at:
//!
//! | `boundary` | Where it sits |
//! |---|---|
//! | `ingest` | Source → raw (`POST /v1/ingest`) |
//! | `stage` | Between stages (`POST /v1/ingest` on the next shape). `between_stages` is accepted on write and stored as `stage`. |
//! | `egress` | Mart → partner / API (`POST /egress`) |
//!
//! Private families are org-scoped the same way as contract identity: the
//! caller's API key org owns the family and every contract in it. A missing
//! or other-org id is 404. Public catalog families are seeded (migrations 050
//! and 052). `POST /public-contract-families/{id}/fork` copies one into the
//! caller's org.
//!
//! ## Routes
//!
//! | Method | Path | Auth |
//! |---|---|---|
//! | POST | `/contract-families` | API key |
//! | GET | `/contract-families` | API key |
//! | GET | `/contract-families/{id}` | API key |
//! | POST | `/contract-families/{id}/links` | API key |
//! | GET | `/contracts/{id}/family` | API key |
//! | GET | `/public-contract-families` | none |
//! | GET | `/public-contract-families/{id}` | none (uuid or slug) |
//! | POST | `/public-contract-families/{id}/fork` | API key (uuid or slug) |
//! | GET | `/public-contracts/{id}/family` | none (uuid or slug) |

use std::collections::{HashMap, HashSet};

use axum::{
    extract::{FromRequest, Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::AppState;

const MAX_LINKS: usize = 32;
const MAX_NAME_LEN: usize = 200;
const MAX_DESCRIPTION_LEN: usize = 4_000;

// ---------------------------------------------------------------------------
// Chain rules (pure)
// ---------------------------------------------------------------------------

/// Pipeline boundary stored on a link.
///
/// `stage` is the between-stages boundary (raw → staging, staging → mart).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineBoundary {
    Ingest,
    Stage,
    Egress,
}

impl PipelineBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Stage => "stage",
            Self::Egress => "egress",
        }
    }

    /// `between_stages` is the prose name for the between-stages boundary and
    /// is stored as `stage`.
    pub fn parse(raw: &str) -> Result<Self, ()> {
        match raw.trim() {
            "ingest" => Ok(Self::Ingest),
            "stage" | "between_stages" => Ok(Self::Stage),
            "egress" => Ok(Self::Egress),
            _ => Err(()),
        }
    }
}

/// One link as the caller described it, before positions are assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyLinkDraft {
    pub contract_id: Uuid,
    pub derived_from_contract_id: Option<Uuid>,
    pub boundary: PipelineBoundary,
}

/// A link after the chain walk. `position` is 0 on the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedFamilyLink {
    pub contract_id: Uuid,
    pub derived_from_contract_id: Option<Uuid>,
    pub boundary: PipelineBoundary,
    pub position: i32,
}

/// Order `links` into one chain.
///
/// An empty set is a family with no members yet. A non-empty set must be a
/// single path: exactly one root (`derived_from` absent), each contract the
/// parent of at most one other, no contract repeated, and every contract
/// reachable from the root. Positions are assigned along that walk.
pub fn order_family_chain(links: &[FamilyLinkDraft]) -> Result<Vec<OrderedFamilyLink>, String> {
    if links.is_empty() {
        return Ok(Vec::new());
    }
    if links.len() > MAX_LINKS {
        return Err(format!("a family has at most {MAX_LINKS} contracts"));
    }

    let mut seen = HashSet::with_capacity(links.len());
    for link in links {
        if !seen.insert(link.contract_id) {
            return Err(format!(
                "contract {} appears more than once in the family",
                link.contract_id
            ));
        }
        if link.derived_from_contract_id == Some(link.contract_id) {
            return Err(format!(
                "contract {} cannot be derived from itself",
                link.contract_id
            ));
        }
    }

    let ids: HashSet<Uuid> = seen;
    for link in links {
        if let Some(parent) = link.derived_from_contract_id {
            if !ids.contains(&parent) {
                return Err(format!(
                    "derived_from contract {parent} is not a member of this family"
                ));
            }
        }
    }

    let roots: Vec<&FamilyLinkDraft> = links
        .iter()
        .filter(|link| link.derived_from_contract_id.is_none())
        .collect();
    if roots.len() != 1 {
        return Err(format!(
            "a family chain needs exactly one contract with no derived_from (the root); found {}",
            roots.len()
        ));
    }

    // parent id → the child link derived from it
    let mut child_of: HashMap<Uuid, &FamilyLinkDraft> = HashMap::with_capacity(links.len());
    for link in links {
        if let Some(parent) = link.derived_from_contract_id {
            if child_of.insert(parent, link).is_some() {
                return Err(format!(
                    "contract {parent} has more than one contract derived from it; a family is a single chain"
                ));
            }
        }
    }

    let by_id: HashMap<Uuid, &FamilyLinkDraft> =
        links.iter().map(|link| (link.contract_id, link)).collect();
    let mut ordered = Vec::with_capacity(links.len());
    let mut current = roots[0].contract_id;
    while ordered.len() < links.len() {
        let link = by_id[&current];
        ordered.push(OrderedFamilyLink {
            contract_id: link.contract_id,
            derived_from_contract_id: link.derived_from_contract_id,
            boundary: link.boundary,
            position: i32::try_from(ordered.len()).unwrap_or(i32::MAX),
        });
        match child_of.get(&current) {
            Some(child) => current = child.contract_id,
            None => break,
        }
    }
    if ordered.len() != links.len() {
        return Err(
            "every contract in the family must be reachable from the root; the links are not a single chain"
                .into(),
        );
    }
    Ok(ordered)
}

fn parse_boundary(raw: &str) -> AppResult<PipelineBoundary> {
    PipelineBoundary::parse(raw).map_err(|()| {
        AppError::BadRequest("boundary must be ingest, stage (between stages), or egress".into())
    })
}

fn clean_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::BadRequest(format!(
            "name must be at most {MAX_NAME_LEN} characters"
        )));
    }
    Ok(name.to_string())
}

fn clean_description(description: Option<&str>) -> AppResult<Option<String>> {
    let Some(description) = description else {
        return Ok(None);
    };
    let description = description.trim();
    if description.is_empty() {
        return Ok(None);
    }
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(AppError::BadRequest(format!(
            "description must be at most {MAX_DESCRIPTION_LEN} characters"
        )));
    }
    Ok(Some(description.to_string()))
}

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FamilyLinkResponse {
    pub contract_id: Uuid,
    pub name: String,
    pub derived_from_contract_id: Option<Uuid>,
    pub derived_from_name: Option<String>,
    pub boundary: PipelineBoundary,
    pub position: i32,
}

#[derive(Debug, Serialize)]
pub struct FamilyDetail {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub links: Vec<FamilyLinkResponse>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FamilySummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub link_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateLinkRequest {
    pub contract_id: Uuid,
    #[serde(default)]
    pub derived_from_contract_id: Option<Uuid>,
    pub boundary: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFamilyRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub links: Vec<CreateLinkRequest>,
}

/// A public catalog family, ordered root → leaf.
#[derive(Debug, Clone, Serialize)]
pub struct PublicFamilyLink {
    pub public_contract_id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub derived_from_public_contract_id: Option<Uuid>,
    pub derived_from_name: Option<String>,
    pub derived_from_slug: Option<String>,
    pub boundary: PipelineBoundary,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicFamily {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub links: Vec<PublicFamilyLink>,
}

/// Optional rename for the private family. Omit `name` (send `{}`) to keep
/// the catalog family's name. Each forked contract keeps the catalog contract's
/// name. `fork_filter` is not set.
#[derive(Debug, Deserialize)]
pub struct ForkFamilyRequest {
    #[serde(default)]
    pub name: Option<String>,
}

/// One private contract created by a family fork.
#[derive(Debug, Serialize)]
pub struct ForkedFamilyContract {
    pub contract_id: Uuid,
    pub name: String,
    pub parent_public_contract_id: Uuid,
    pub boundary: PipelineBoundary,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

/// `POST /public-contract-families/{id}/fork`.
#[derive(Debug, Serialize)]
pub struct ForkFamilyResponse {
    pub family: FamilyDetail,
    pub contracts: Vec<ForkedFamilyContract>,
}

// ---------------------------------------------------------------------------
// Storage — private families
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct FamilyRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    description: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct LinkRow {
    contract_id: Uuid,
    name: String,
    derived_from_contract_id: Option<Uuid>,
    derived_from_name: Option<String>,
    boundary: String,
    position: i32,
}

#[derive(sqlx::FromRow)]
struct DraftRow {
    contract_id: Uuid,
    derived_from_contract_id: Option<Uuid>,
    boundary: String,
}

fn db_err(ctx: &str, err: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db) = &err {
        if db.code().as_deref() == Some("23505") {
            return match db.constraint() {
                Some("contract_family_links_contract_unique") => {
                    AppError::Conflict("contract is already in a family".into())
                }
                Some("contract_families_org_name_unique") => {
                    AppError::Conflict("a family with this name already exists in the org".into())
                }
                _ => AppError::Conflict("family link conflicts with an existing row".into()),
            };
        }
    }
    AppError::Internal(format!("{ctx}: {err}"))
}

fn drafts_from_requests(links: &[CreateLinkRequest]) -> AppResult<Vec<FamilyLinkDraft>> {
    links
        .iter()
        .map(|link| {
            Ok(FamilyLinkDraft {
                contract_id: link.contract_id,
                derived_from_contract_id: link.derived_from_contract_id,
                boundary: parse_boundary(&link.boundary)?,
            })
        })
        .collect()
}

async fn assert_contracts_in_org(
    tx: &mut Transaction<'_, Postgres>,
    ids: &[Uuid],
    org_id: Option<Uuid>,
) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let found: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM contracts \
         WHERE id = ANY($1::uuid[]) \
           AND deleted_at IS NULL \
           AND ($2::uuid IS NULL OR org_id = $2)",
    )
    .bind(ids)
    .bind(org_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| db_err("load contracts for family", err))?;

    // Any miss is 404. A contract in another org is indistinguishable from a
    // missing id, matching the contract routes.
    if found.len() != ids.len() {
        return Err(AppError::NotFound("contract not found".into()));
    }
    Ok(())
}

async fn insert_links(
    tx: &mut Transaction<'_, Postgres>,
    family_id: Uuid,
    links: &[OrderedFamilyLink],
) -> AppResult<()> {
    sqlx::query("DELETE FROM contract_family_links WHERE family_id = $1")
        .bind(family_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| db_err("replace family links", err))?;

    for link in links {
        sqlx::query(
            "INSERT INTO contract_family_links \
                (family_id, contract_id, derived_from_contract_id, boundary, position) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(family_id)
        .bind(link.contract_id)
        .bind(link.derived_from_contract_id)
        .bind(link.boundary.as_str())
        .bind(link.position)
        .execute(&mut **tx)
        .await
        .map_err(|err| db_err("insert family link", err))?;
    }
    Ok(())
}

async fn load_links(pool: &PgPool, family_id: Uuid) -> AppResult<Vec<FamilyLinkResponse>> {
    let rows: Vec<LinkRow> = sqlx::query_as(
        "SELECT l.contract_id, c.name, \
                l.derived_from_contract_id, p.name AS derived_from_name, \
                l.boundary, l.position \
         FROM contract_family_links l \
         JOIN contracts c ON c.id = l.contract_id AND c.deleted_at IS NULL \
         LEFT JOIN contracts p \
             ON p.id = l.derived_from_contract_id AND p.deleted_at IS NULL \
         WHERE l.family_id = $1 \
         ORDER BY l.position ASC",
    )
    .bind(family_id)
    .fetch_all(pool)
    .await
    .map_err(|err| db_err("load family links", err))?;

    rows.into_iter()
        .map(|row| {
            let boundary = PipelineBoundary::parse(&row.boundary).map_err(|()| {
                AppError::Internal(format!(
                    "invalid boundary {:?} on family {family_id}",
                    row.boundary
                ))
            })?;
            Ok(FamilyLinkResponse {
                contract_id: row.contract_id,
                name: row.name,
                derived_from_contract_id: row.derived_from_contract_id,
                derived_from_name: row.derived_from_name,
                boundary,
                position: row.position,
            })
        })
        .collect()
}

fn detail_from(row: FamilyRow, links: Vec<FamilyLinkResponse>) -> FamilyDetail {
    FamilyDetail {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        description: row.description,
        created_at: row.created_at,
        updated_at: row.updated_at,
        links,
    }
}

/// Read one family. `org_id = None` is the self-host / auth-off scope (no
/// org filter). Any other org, or a missing id, is 404.
pub(crate) async fn get_family(
    pool: &PgPool,
    id: Uuid,
    org_id: Option<Uuid>,
) -> AppResult<FamilyDetail> {
    let row: Option<FamilyRow> = sqlx::query_as(
        "SELECT id, org_id, name, description, created_at, updated_at \
         FROM contract_families \
         WHERE id = $1 AND ($2::uuid IS NULL OR org_id = $2)",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| db_err("load family", err))?;

    let Some(row) = row else {
        return Err(AppError::NotFound("contract family not found".into()));
    };
    let links = load_links(pool, row.id).await?;
    Ok(detail_from(row, links))
}

pub(crate) async fn list_families(
    pool: &PgPool,
    org_id: Option<Uuid>,
) -> AppResult<Vec<FamilySummary>> {
    let rows: Vec<FamilySummary> = sqlx::query_as(
        "SELECT f.id, f.org_id, f.name, f.description, f.created_at, f.updated_at, \
                COUNT(c.id) AS link_count \
         FROM contract_families f \
         LEFT JOIN contract_family_links l ON l.family_id = f.id \
         LEFT JOIN contracts c ON c.id = l.contract_id AND c.deleted_at IS NULL \
         WHERE ($1::uuid IS NULL OR f.org_id = $1) \
         GROUP BY f.id \
         ORDER BY f.name ASC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(|err| db_err("list families", err))?;
    Ok(rows)
}

pub(crate) async fn create_family(
    pool: &PgPool,
    org_id: Uuid,
    name: &str,
    description: Option<&str>,
    links: &[CreateLinkRequest],
) -> AppResult<FamilyDetail> {
    let name = clean_name(name)?;
    let description = clean_description(description)?;
    let drafts = drafts_from_requests(links)?;
    let ordered = order_family_chain(&drafts).map_err(AppError::BadRequest)?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| db_err("begin create family", err))?;
    let member_ids: Vec<Uuid> = ordered.iter().map(|link| link.contract_id).collect();
    assert_contracts_in_org(&mut tx, &member_ids, Some(org_id)).await?;

    let row: FamilyRow = sqlx::query_as(
        "INSERT INTO contract_families (org_id, name, description) \
         VALUES ($1, $2, $3) \
         RETURNING id, org_id, name, description, created_at, updated_at",
    )
    .bind(org_id)
    .bind(&name)
    .bind(&description)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| db_err("insert family", err))?;

    insert_links(&mut tx, row.id, &ordered).await?;
    tx.commit()
        .await
        .map_err(|err| db_err("commit create family", err))?;

    get_family(pool, row.id, Some(org_id)).await
}

pub(crate) async fn add_family_link(
    pool: &PgPool,
    family_id: Uuid,
    org_id: Option<Uuid>,
    link: &CreateLinkRequest,
) -> AppResult<FamilyDetail> {
    let draft = FamilyLinkDraft {
        contract_id: link.contract_id,
        derived_from_contract_id: link.derived_from_contract_id,
        boundary: parse_boundary(&link.boundary)?,
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| db_err("begin add family link", err))?;

    // Lock the family row so two adds cannot each rebuild the chain.
    let family: Option<FamilyRow> = sqlx::query_as(
        "SELECT id, org_id, name, description, created_at, updated_at \
         FROM contract_families \
         WHERE id = $1 AND ($2::uuid IS NULL OR org_id = $2) \
         FOR UPDATE",
    )
    .bind(family_id)
    .bind(org_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| db_err("lock family", err))?;
    let Some(family) = family else {
        return Err(AppError::NotFound("contract family not found".into()));
    };

    assert_contracts_in_org(&mut tx, &[draft.contract_id], Some(family.org_id)).await?;

    let existing: Vec<DraftRow> = sqlx::query_as(
        "SELECT contract_id, derived_from_contract_id, boundary \
         FROM contract_family_links \
         WHERE family_id = $1",
    )
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|err| db_err("load family links", err))?;

    let mut drafts = Vec::with_capacity(existing.len() + 1);
    for row in &existing {
        if row.contract_id == draft.contract_id {
            return Err(AppError::Conflict(
                "contract is already in this family".into(),
            ));
        }
        drafts.push(FamilyLinkDraft {
            contract_id: row.contract_id,
            derived_from_contract_id: row.derived_from_contract_id,
            boundary: PipelineBoundary::parse(&row.boundary).map_err(|()| {
                AppError::Internal(format!(
                    "invalid boundary {:?} on family {family_id}",
                    row.boundary
                ))
            })?,
        });
    }
    drafts.push(draft);
    let ordered = order_family_chain(&drafts).map_err(AppError::BadRequest)?;

    insert_links(&mut tx, family_id, &ordered).await?;
    sqlx::query("UPDATE contract_families SET updated_at = now() WHERE id = $1")
        .bind(family_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| db_err("touch family", err))?;
    tx.commit()
        .await
        .map_err(|err| db_err("commit add family link", err))?;

    // Scope the read to the family we just locked, not the caller's maybe-None
    // dev org, so auth-off still returns the row we wrote.
    get_family(pool, family.id, Some(family.org_id)).await
}

/// Family for one private contract, if the caller can see that contract.
pub(crate) async fn family_for_contract(
    pool: &PgPool,
    contract_id: Uuid,
    org_id: Option<Uuid>,
) -> AppResult<FamilyDetail> {
    let found: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT c.id, l.family_id \
         FROM contracts c \
         LEFT JOIN contract_family_links l ON l.contract_id = c.id \
         WHERE c.id = $1 AND c.deleted_at IS NULL \
           AND ($2::uuid IS NULL OR c.org_id = $2)",
    )
    .bind(contract_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| db_err("load contract family", err))?;

    let Some((_contract_id, family_id)) = found else {
        return Err(AppError::ContractNotFound(contract_id));
    };
    let Some(family_id) = family_id else {
        return Err(AppError::NotFound("contract is not in a family".into()));
    };
    get_family(pool, family_id, org_id).await
}

// ---------------------------------------------------------------------------
// Storage — public catalog families
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct PublicLinkRow {
    family_id: Uuid,
    family_name: String,
    family_slug: String,
    family_description: Option<String>,
    public_contract_id: Option<Uuid>,
    name: Option<String>,
    slug: Option<String>,
    derived_from_public_contract_id: Option<Uuid>,
    derived_from_name: Option<String>,
    derived_from_slug: Option<String>,
    boundary: Option<String>,
    position: Option<i32>,
}

const PUBLIC_FAMILY_SELECT: &str = "\
    SELECT f.id AS family_id, f.name AS family_name, f.slug AS family_slug, \
           f.description AS family_description, \
           c.id AS public_contract_id, c.name, c.slug, \
           p.id AS derived_from_public_contract_id, \
           p.name AS derived_from_name, p.slug AS derived_from_slug, \
           l.boundary, l.position \
    FROM public_contract_families f \
    LEFT JOIN public_contract_family_links l ON l.family_id = f.id \
    LEFT JOIN public_contracts c ON c.id = l.public_contract_id \
    LEFT JOIN public_contracts p ON p.id = l.derived_from_public_contract_id";

fn group_public_families(rows: Vec<PublicLinkRow>) -> AppResult<Vec<PublicFamily>> {
    let mut order = Vec::new();
    let mut by_id: HashMap<Uuid, PublicFamily> = HashMap::new();
    for row in rows {
        if let std::collections::hash_map::Entry::Vacant(entry) = by_id.entry(row.family_id) {
            order.push(row.family_id);
            entry.insert(PublicFamily {
                id: row.family_id,
                name: row.family_name.clone(),
                slug: row.family_slug.clone(),
                description: row.family_description.clone(),
                links: Vec::new(),
            });
        }
        let Some(public_contract_id) = row.public_contract_id else {
            continue;
        };
        let boundary =
            PipelineBoundary::parse(row.boundary.as_deref().unwrap_or("")).map_err(|()| {
                AppError::Internal(format!(
                    "invalid boundary {:?} on public family {}",
                    row.boundary, row.family_id
                ))
            })?;
        let family = by_id.get_mut(&row.family_id).expect("just inserted");
        family.links.push(PublicFamilyLink {
            public_contract_id,
            name: row.name.unwrap_or_default(),
            slug: row.slug,
            derived_from_public_contract_id: row.derived_from_public_contract_id,
            derived_from_name: row.derived_from_name,
            derived_from_slug: row.derived_from_slug,
            boundary,
            position: row.position.unwrap_or(0),
        });
    }
    Ok(order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect())
}

pub(crate) async fn list_public_families(pool: &PgPool) -> AppResult<Vec<PublicFamily>> {
    let sql = format!("{PUBLIC_FAMILY_SELECT} ORDER BY f.name ASC, l.position ASC NULLS LAST");
    let rows = sqlx::query_as::<_, PublicLinkRow>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|err| db_err("list public families", err))?;
    group_public_families(rows)
}

pub(crate) async fn get_public_family(pool: &PgPool, id_or_slug: &str) -> AppResult<PublicFamily> {
    let (sql, bind_uuid): (String, Option<Uuid>) = if let Ok(id) = Uuid::parse_str(id_or_slug) {
        (
            format!("{PUBLIC_FAMILY_SELECT} WHERE f.id = $1 ORDER BY l.position ASC NULLS LAST"),
            Some(id),
        )
    } else {
        (
            format!("{PUBLIC_FAMILY_SELECT} WHERE f.slug = $1 ORDER BY l.position ASC NULLS LAST"),
            None,
        )
    };
    let mut query = sqlx::query_as::<_, PublicLinkRow>(&sql);
    if let Some(id) = bind_uuid {
        query = query.bind(id);
    } else {
        query = query.bind(id_or_slug);
    }
    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| db_err("load public family", err))?;
    group_public_families(rows)?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("public contract family not found".into()))
}

/// Family that contains this public contract, if any.
pub(crate) async fn public_family_for_contract(
    pool: &PgPool,
    public_contract_id: Uuid,
) -> AppResult<Option<PublicFamily>> {
    let sql = format!(
        "{PUBLIC_FAMILY_SELECT} \
         WHERE f.id = ( \
             SELECT family_id FROM public_contract_family_links \
             WHERE public_contract_id = $1 \
         ) \
         ORDER BY l.position ASC NULLS LAST"
    );
    let rows = sqlx::query_as::<_, PublicLinkRow>(&sql)
        .bind(public_contract_id)
        .fetch_all(pool)
        .await
        .map_err(|err| db_err("load public family for contract", err))?;
    Ok(group_public_families(rows)?.into_iter().next())
}

pub(crate) async fn public_family_for_ref(
    pool: &PgPool,
    id_or_slug: &str,
) -> AppResult<PublicFamily> {
    let contract_id: Option<Uuid> = if let Ok(id) = Uuid::parse_str(id_or_slug) {
        sqlx::query_scalar("SELECT id FROM public_contracts WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|err| db_err("load public contract", err))?
    } else {
        sqlx::query_scalar("SELECT id FROM public_contracts WHERE slug = $1")
            .bind(id_or_slug)
            .fetch_optional(pool)
            .await
            .map_err(|err| db_err("load public contract", err))?
    };
    let Some(contract_id) = contract_id else {
        return Err(AppError::NotFound("public contract not found".into()));
    };
    public_family_for_contract(pool, contract_id)
        .await?
        .ok_or_else(|| AppError::NotFound("public contract is not in a family".into()))
}

// ---------------------------------------------------------------------------
// Fork a public family into the caller's org
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct PublicContractCopy {
    id: Uuid,
    name: String,
    description: Option<String>,
    contract_yaml: String,
}

fn family_fork_cap_error(needed: i64, count: i64, limit: i64) -> AppError {
    AppError::PlanResourceLimit {
        resource: "contracts",
        limit,
        current: count,
        message: format!(
            "Forking this pipeline creates {needed} contracts. The Free plan allows {limit} contracts and this org already has {count}. Upgrade at {}",
            crate::plan::PLAN_LIMIT_UPGRADE_URL
        ),
    }
}

async fn insert_forked_contract(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    description: Option<&str>,
    yaml: &str,
    org_id: Uuid,
    public_id: Uuid,
) -> AppResult<(Uuid, DateTime<Utc>)> {
    let salt: Vec<u8> = {
        use rand::RngCore;
        let mut salt = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        salt
    };
    // Same row shape as a single public-contract fork: identity plus a
    // draft 1.0.0 whose YAML is the catalog copy. parent_public_contract_id
    // records the source. fork_filter stays null.
    let row = sqlx::query(
        r#"WITH ins_contract AS (
               INSERT INTO contracts (name, description, pii_salt, org_id,
                                      parent_public_contract_id, fork_filter)
               VALUES ($1, $2, $3, $4, $5, NULL)
               RETURNING id, created_at
           ),
           ins_version AS (
               INSERT INTO contract_versions
                   (contract_id, version, state, yaml_content,
                    compliance_mode, egress_leakage_mode)
               SELECT id, '1.0.0', 'draft', $6, false, 'off'
               FROM ins_contract
               RETURNING contract_id
           )
           SELECT id, created_at FROM ins_contract"#,
    )
    .bind(name)
    .bind(description)
    .bind(&salt)
    .bind(org_id)
    .bind(public_id)
    .bind(yaml)
    .fetch_one(&mut **tx)
    .await
    .map_err(AppError::from)?;
    let contract_id: Uuid = row
        .try_get("id")
        .map_err(|err| AppError::Internal(format!("missing id from family fork insert: {err}")))?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|err| {
        AppError::Internal(format!("missing created_at from family fork insert: {err}"))
    })?;
    Ok((contract_id, created_at))
}

/// Copy every contract in a public family into `org_id` and create a private
/// family with the same chain, in one transaction.
///
/// Hosted Free is refused before any insert when `existing + links` would
/// pass [`crate::plan::FREE_CONTRACT_LIMIT`]. The org row lock is held until
/// commit, so a refused fork writes nothing.
pub(crate) async fn fork_public_family(
    pool: &PgPool,
    id_or_slug: &str,
    org_id: Uuid,
    name_override: Option<&str>,
    self_hosted: bool,
) -> AppResult<ForkFamilyResponse> {
    let public_family = get_public_family(pool, id_or_slug).await?;
    let mut links = public_family.links.clone();
    links.sort_by_key(|link| link.position);
    if links.is_empty() {
        return Err(AppError::BadRequest(
            "this catalog family has no contracts to fork".into(),
        ));
    }
    if links.len() > MAX_LINKS {
        return Err(AppError::BadRequest(format!(
            "a family has at most {MAX_LINKS} contracts"
        )));
    }

    let family_name = match name_override {
        Some(name) => clean_name(name)?,
        None => clean_name(&public_family.name)?,
    };
    let family_description = clean_description(public_family.description.as_deref())?;

    let ids: Vec<Uuid> = links.iter().map(|link| link.public_contract_id).collect();
    let copies: Vec<PublicContractCopy> = sqlx::query_as(
        "SELECT id, name, description, contract_yaml \
         FROM public_contracts WHERE id = ANY($1::uuid[])",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(|err| db_err("load public contracts for family fork", err))?;
    if copies.len() != links.len() {
        return Err(AppError::Internal(
            "public family is missing a catalog contract".into(),
        ));
    }
    let by_id: HashMap<Uuid, PublicContractCopy> =
        copies.into_iter().map(|row| (row.id, row)).collect();

    let mut prepared = Vec::with_capacity(links.len());
    for link in &links {
        let source = by_id.get(&link.public_contract_id).ok_or_else(|| {
            AppError::Internal("public family is missing a catalog contract".into())
        })?;
        prepared.push((
            link.clone(),
            clean_name(&source.name)?,
            clean_description(source.description.as_deref())?,
            source.contract_yaml.clone(),
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| db_err("begin family fork", err))?;

    if let Some((count, limit)) =
        crate::storage::contract_quota_snapshot(&mut tx, Some(org_id), self_hosted).await?
    {
        let needed = i64::try_from(prepared.len()).unwrap_or(i64::MAX);
        if count.saturating_add(needed) > limit {
            return Err(family_fork_cap_error(needed, count, limit));
        }
    }

    let mut created = Vec::with_capacity(prepared.len());
    let mut id_map: HashMap<Uuid, Uuid> = HashMap::with_capacity(prepared.len());
    for (link, name, description, yaml) in &prepared {
        let (contract_id, created_at) = insert_forked_contract(
            &mut tx,
            name,
            description.as_deref(),
            yaml,
            org_id,
            link.public_contract_id,
        )
        .await?;
        id_map.insert(link.public_contract_id, contract_id);
        created.push(ForkedFamilyContract {
            contract_id,
            name: name.clone(),
            parent_public_contract_id: link.public_contract_id,
            boundary: link.boundary,
            position: link.position,
            created_at,
        });
    }

    let ordered = prepared
        .iter()
        .map(|(link, _, _, _)| {
            let contract_id = id_map[&link.public_contract_id];
            let derived_from_contract_id = link
                .derived_from_public_contract_id
                .map(|parent| {
                    id_map.get(&parent).copied().ok_or_else(|| {
                        AppError::Internal(
                            "public family derived_from is not a member of the chain".into(),
                        )
                    })
                })
                .transpose()?;
            Ok(OrderedFamilyLink {
                contract_id,
                derived_from_contract_id,
                boundary: link.boundary,
                position: link.position,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let family_row: FamilyRow = sqlx::query_as(
        "INSERT INTO contract_families (org_id, name, description) \
         VALUES ($1, $2, $3) \
         RETURNING id, org_id, name, description, created_at, updated_at",
    )
    .bind(org_id)
    .bind(&family_name)
    .bind(&family_description)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| db_err("insert forked family", err))?;
    insert_links(&mut tx, family_row.id, &ordered).await?;
    let family_id = family_row.id;
    tx.commit()
        .await
        .map_err(|err| db_err("commit family fork", err))?;

    let family = get_family(pool, family_id, Some(org_id)).await?;
    Ok(ForkFamilyResponse {
        family,
        contracts: created,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn require_caller_org(state: &AppState, org_id: Option<Uuid>) -> AppResult<Option<Uuid>> {
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }
    Ok(org_id)
}

pub(crate) async fn create_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<FamilyDetail>)> {
    let org_id = crate::org_id_from_req(&req);
    let org_id = crate::dev_org_fallback(&state, &req, org_id)?;
    let org_id = require_caller_org(&state, org_id)?
        .ok_or_else(|| AppError::BadRequest("no org context".into()))?;

    let Json(body): Json<CreateFamilyRequest> = Json::from_request(req, &state)
        .await
        .map_err(|err| AppError::BadRequest(err.to_string()))?;

    let family = create_family(
        &state.db,
        org_id,
        &body.name,
        body.description.as_deref(),
        &body.links,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(family)))
}

pub(crate) async fn list_families_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
) -> AppResult<Json<Vec<FamilySummary>>> {
    let org_id = require_caller_org(&state, org_id)?;
    Ok(Json(list_families(&state.db, org_id).await?))
}

pub(crate) async fn get_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    Path(id): Path<Uuid>,
) -> AppResult<Json<FamilyDetail>> {
    let org_id = require_caller_org(&state, org_id)?;
    Ok(Json(get_family(&state.db, id, org_id).await?))
}

pub(crate) async fn add_family_link_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateLinkRequest>,
) -> AppResult<(StatusCode, Json<FamilyDetail>)> {
    let org_id = require_caller_org(&state, org_id)?;
    let family = add_family_link(&state.db, id, org_id, &body).await?;
    Ok((StatusCode::CREATED, Json(family)))
}

pub(crate) async fn get_contract_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    crate::OrgId(org_id): crate::OrgId,
    Path(id): Path<Uuid>,
) -> AppResult<Json<FamilyDetail>> {
    let org_id = require_caller_org(&state, org_id)?;
    Ok(Json(family_for_contract(&state.db, id, org_id).await?))
}

pub(crate) async fn list_public_families_handler(
    State(state): State<std::sync::Arc<AppState>>,
) -> AppResult<Json<Vec<PublicFamily>>> {
    Ok(Json(list_public_families(&state.db).await?))
}

pub(crate) async fn get_public_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<PublicFamily>> {
    Ok(Json(get_public_family(&state.db, &id_or_slug).await?))
}

/// `POST /public-contract-families/{id}/fork` — copy the chain into the caller's org.
pub(crate) async fn fork_public_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    req: axum::extract::Request,
) -> AppResult<(StatusCode, Json<ForkFamilyResponse>)> {
    let org_id = crate::org_id_from_req(&req);
    let org_id = crate::dev_org_fallback(&state, &req, org_id)?;
    let org_id = require_caller_org(&state, org_id)?
        .ok_or_else(|| AppError::BadRequest("no org context".into()))?;

    let Json(body): Json<ForkFamilyRequest> = Json::from_request(req, &state)
        .await
        .map_err(|err| AppError::BadRequest(err.to_string()))?;

    let forked = fork_public_family(
        &state.db,
        &id_or_slug,
        org_id,
        body.name.as_deref(),
        state.self_hosted,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(forked)))
}

pub(crate) async fn get_public_contract_family_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<PublicFamily>> {
    Ok(Json(public_family_for_ref(&state.db, &id_or_slug).await?))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: u128, parent: Option<u128>, boundary: PipelineBoundary) -> FamilyLinkDraft {
        FamilyLinkDraft {
            contract_id: Uuid::from_u128(id),
            derived_from_contract_id: parent.map(Uuid::from_u128),
            boundary,
        }
    }

    #[test]
    fn stripe_shaped_chain_orders_root_to_egress() {
        let drafts = vec![
            link(4, Some(3), PipelineBoundary::Egress),
            link(1, None, PipelineBoundary::Ingest),
            link(3, Some(2), PipelineBoundary::Stage),
            link(2, Some(1), PipelineBoundary::Stage),
        ];
        let ordered = order_family_chain(&drafts).unwrap();
        let ids: Vec<u128> = ordered
            .iter()
            .map(|link| link.contract_id.as_u128())
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        assert_eq!(
            ordered.iter().map(|link| link.position).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(ordered[0].derived_from_contract_id, None);
        assert_eq!(
            ordered[1].derived_from_contract_id,
            Some(Uuid::from_u128(1))
        );
        assert_eq!(ordered[3].boundary, PipelineBoundary::Egress);
    }

    #[test]
    fn between_stages_alias_is_stage() {
        assert_eq!(
            PipelineBoundary::parse("between_stages").unwrap(),
            PipelineBoundary::Stage
        );
        assert_eq!(PipelineBoundary::Stage.as_str(), "stage");
        assert!(PipelineBoundary::parse("mart").is_err());
    }

    #[test]
    fn empty_family_is_a_valid_chain() {
        assert!(order_family_chain(&[]).unwrap().is_empty());
    }

    #[test]
    fn rejects_two_roots_a_branch_a_cycle_and_an_outside_parent() {
        let two_roots = vec![
            link(1, None, PipelineBoundary::Ingest),
            link(2, None, PipelineBoundary::Stage),
        ];
        assert!(order_family_chain(&two_roots)
            .unwrap_err()
            .contains("exactly one"));

        let branch = vec![
            link(1, None, PipelineBoundary::Ingest),
            link(2, Some(1), PipelineBoundary::Stage),
            link(3, Some(1), PipelineBoundary::Egress),
        ];
        assert!(order_family_chain(&branch)
            .unwrap_err()
            .contains("more than one"));

        let cycle = vec![
            link(1, None, PipelineBoundary::Ingest),
            link(2, Some(3), PipelineBoundary::Stage),
            link(3, Some(2), PipelineBoundary::Stage),
        ];
        assert!(order_family_chain(&cycle)
            .unwrap_err()
            .contains("single chain"));

        let outside = vec![
            link(1, None, PipelineBoundary::Ingest),
            link(2, Some(9), PipelineBoundary::Stage),
        ];
        assert!(order_family_chain(&outside)
            .unwrap_err()
            .contains("not a member"));

        let self_parent = vec![link(1, Some(1), PipelineBoundary::Ingest)];
        assert!(order_family_chain(&self_parent)
            .unwrap_err()
            .contains("itself"));
    }
}
