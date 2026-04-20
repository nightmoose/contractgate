//! Contract types — the schema that defines what a valid data event looks like.
//!
//! A ContractGate contract is composed of three sections:
//!   - `ontology`  — field-level type/constraint definitions
//!   - `glossary`  — human-readable term definitions (business context)
//!   - `metrics`   — numeric KPI / measure definitions with range bounds
//!
//! Contracts are stored as YAML and versioned in Supabase.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level contract
// ---------------------------------------------------------------------------

/// A versioned semantic contract describing the shape and rules of a data event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    /// Semver-style version string, e.g. "1.0"
    pub version: String,
    /// Human-readable name for the contract
    pub name: String,
    /// Optional description of what data this contract covers
    #[serde(default)]
    pub description: Option<String>,
    /// RFC-004: when true, events containing fields not declared in
    /// `ontology.entities` are rejected as per-event `UNDECLARED_FIELD`
    /// violations.  Default `false` preserves the pre-RFC-004 behavior
    /// (undeclared fields pass through untouched).
    #[serde(default)]
    pub compliance_mode: bool,
    /// Field-level ontology (structure + constraints)
    pub ontology: Ontology,
    /// Business glossary entries
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    /// Metric / measure definitions
    #[serde(default)]
    pub metrics: Vec<MetricDefinition>,
}

// ---------------------------------------------------------------------------
// Ontology (field definitions)
// ---------------------------------------------------------------------------

/// The structural schema — a list of field (entity) definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ontology {
    pub entities: Vec<FieldDefinition>,
}

/// Defines a single field (entity attribute) inside a data event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinition {
    /// Field name as it appears in the JSON event
    pub name: String,
    /// Expected value type
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether this field must be present in every event
    #[serde(default = "default_true")]
    pub required: bool,
    /// Optional regex pattern the string value must match
    #[serde(default)]
    pub pattern: Option<String>,
    /// Optional allowed value set (for strings / integers)
    #[serde(rename = "enum", default)]
    pub allowed_values: Option<Vec<serde_json::Value>>,
    /// Minimum numeric value (inclusive); applies to integer / float
    #[serde(default)]
    pub min: Option<f64>,
    /// Maximum numeric value (inclusive); applies to integer / float
    #[serde(default)]
    pub max: Option<f64>,
    /// Minimum string length (for string fields)
    #[serde(default)]
    pub min_length: Option<usize>,
    /// Maximum string length (for string fields)
    #[serde(default)]
    pub max_length: Option<usize>,
    /// Nested field definitions when field_type == Object
    #[serde(default)]
    pub properties: Option<Vec<FieldDefinition>>,
    /// For Array fields — element type constraints
    #[serde(default)]
    pub items: Option<Box<FieldDefinition>>,
    /// RFC-004: optional PII transform applied AFTER validation.  Only
    /// supported on `FieldType::String` — a non-string field with a
    /// transform declared is a compile-time error at contract load.
    #[serde(default)]
    pub transform: Option<Transform>,
}

/// Supported field types inside a contract ontology.
///
/// `"number"` is accepted as an alias for `"float"` to match common data-contract
/// conventions (the canonical CLAUDE.md example uses `type: number`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    Integer,
    /// `"float"` or `"number"` — both accepted during deserialization.
    #[serde(alias = "number")]
    Float,
    Boolean,
    Object,
    Array,
    /// Field may hold any JSON value (use sparingly — weakens contract)
    Any,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// PII transforms (RFC-004)
// ---------------------------------------------------------------------------

/// A PII transform declaration attached to a single string-typed field.
///
/// Runs in the ingest pipeline AFTER validation and BEFORE storage/forward.
/// Raw values reach the validator; the transformed form is what lands on
/// disk and in downstream systems.  See RFC-004 for the full pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    /// Which transform to apply.
    pub kind: TransformKind,
    /// Mask style — required for `kind: mask`, ignored otherwise.  Defaults
    /// to `Opaque` when omitted on a mask transform.
    #[serde(default)]
    pub style: Option<MaskStyle>,
}

/// The four transform kinds supported in v1.  No stacking: each field gets
/// at most one transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformKind {
    /// Replace with a sentinel.  See `MaskStyle` for exact behavior.
    Mask,
    /// Deterministic HMAC-SHA256 keyed on the per-contract `pii_salt`.
    /// Output is `"hmac-sha256:<hex>"`.  Same input → same output on the
    /// same contract, so analytics joins on hashed keys work forever.
    Hash,
    /// Remove the field from the payload entirely.
    Drop,
    /// Replace with the literal sentinel string `"<REDACTED>"`.
    Redact,
}

/// Sub-setting for `TransformKind::Mask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskStyle {
    /// Replace entire value with the fixed sentinel `"****"` — length
    /// doesn't leak.  Default when `style` is omitted on a mask transform.
    #[default]
    Opaque,
    /// Preserve length + character class per position (digit → digit,
    /// letter → letter of same case, symbols pass through), shuffled
    /// deterministically per (contract salt, field name).  Not reversible,
    /// not a formal FPE scheme — see RFC-004 non-goals.
    FormatPreserving,
}

// ---------------------------------------------------------------------------
// Glossary
// ---------------------------------------------------------------------------

/// A single term definition in the business glossary.
///
/// Accepts two naming conventions for maximum YAML compatibility:
///   - Canonical:  `field` / `description` / `constraints`
///   - Legacy/alt: `term`  / `definition`  / `constraints`
///
/// Both forms are accepted during deserialization; serialization always
/// uses the canonical names (`field`, `description`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryEntry {
    /// The event field name this entry documents.
    /// Also accepted as `"term"` in YAML for compatibility with common contract editors.
    #[serde(alias = "term")]
    pub field: String,
    /// Human-readable description of the field's meaning.
    /// Also accepted as `"definition"` in YAML.
    #[serde(alias = "definition")]
    pub description: String,
    /// Optional natural-language constraint statement (informational only)
    #[serde(default)]
    pub constraints: Option<String>,
    /// Optional list of alternate names / synonyms for documentation (informational only).
    /// Not used for validation — stored for reference.
    #[serde(default)]
    pub synonyms: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Metric definitions
// ---------------------------------------------------------------------------

/// Defines a numeric KPI / measure associated with a contract.
///
/// Two styles are supported:
///
/// 1. **Field-bound metric** — validates a single event field stays within
///    optional `min`/`max` bounds:
///    ```yaml
///    - name: latency_ms
///      field: latency
///      type: float
///      max: 500
///    ```
///
/// 2. **Formula metric** — records the formula string for documentation /
///    downstream aggregation systems. ContractGate does not evaluate the
///    formula at ingestion time; it is stored for reference:
///    ```yaml
///    - name: total_revenue
///      formula: "sum(amount) where event_type = 'purchase'"
///    ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDefinition {
    /// Metric name (used in violation messages and dashboards)
    pub name: String,
    /// JSON field path for field-bound metrics (dot-separated, e.g. "user.score")
    #[serde(default)]
    pub field: Option<String>,
    /// Expected numeric type for field-bound metrics
    #[serde(rename = "type", default)]
    pub metric_type: Option<MetricType>,
    /// Formula string for aggregate / formula-style metrics (informational)
    #[serde(default)]
    pub formula: Option<String>,
    /// Inclusive lower bound — only applies when `field` is set
    #[serde(default)]
    pub min: Option<f64>,
    /// Inclusive upper bound — only applies when `field` is set
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Integer,
    Float,
}

// ---------------------------------------------------------------------------
// DB row types — identity vs version split (RFC-002)
// ---------------------------------------------------------------------------

/// State machine for `contract_versions.state`.
///
/// Strict, forward-only transitions: `draft → stable → deprecated`.  No
/// other moves are legal, in either direction, ever.  The Postgres trigger
/// `contract_versions_frozen` enforces this at the storage layer as a
/// belt-and-braces check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionState {
    Draft,
    Stable,
    Deprecated,
}

impl VersionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            VersionState::Draft => "draft",
            VersionState::Stable => "stable",
            VersionState::Deprecated => "deprecated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "stable" => Some(Self::Stable),
            "deprecated" => Some(Self::Deprecated),
            _ => None,
        }
    }
}

/// Resolution policy for unpinned ingest traffic on a given contract.
///
/// See RFC-002 §2b.
///   - `Strict`   — validate against latest-stable only; fail-closed.
///   - `Fallback` — on failure, retry against other stables in `promoted_at
///     DESC` order, first-pass-wins.  `contract_version` on the resulting
///     audit row always records the version that actually matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MultiStableResolution {
    #[default]
    Strict,
    Fallback,
}

impl MultiStableResolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            MultiStableResolution::Strict => "strict",
            MultiStableResolution::Fallback => "fallback",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "strict" => Some(Self::Strict),
            "fallback" => Some(Self::Fallback),
            _ => None,
        }
    }
}

/// Identity row — one per `contract_id`.  Mutable: `name`, `description`,
/// `multi_stable_resolution`.  Renames are mirrored to
/// `contract_name_history` via the `contracts_record_rename` trigger.
///
/// RFC-004: carries `pii_salt` loaded from the DB — a 32-byte secret
/// key used by the hash + format-preserving-mask transforms.  This
/// struct is an internal/storage type; it must NEVER be serialized to
/// an HTTP response directly.  The `#[serde(skip_serializing)]` on
/// `pii_salt` is defence-in-depth; the public-facing types
/// (`ContractResponse`, `ContractSummary`) don't carry the field at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractIdentity {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub multi_stable_resolution: MultiStableResolution,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Per-contract 32-byte salt for PII transforms.  NEVER serialize.
    #[serde(skip_serializing)]
    pub pii_salt: Vec<u8>,
}

/// Version row — one per `(contract_id, version)` pair.  Frozen once state
/// leaves `draft`.  `compliance_mode` is per-version (RFC-004): it is part
/// of the semantic contract and cannot be toggled after promotion; the
/// migration-005 trigger enforces the freeze.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractVersion {
    pub id: uuid::Uuid,
    pub contract_id: uuid::Uuid,
    pub version: String,
    pub state: VersionState,
    pub yaml_content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub promoted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deprecated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Mirrors `contract_versions.compliance_mode`.  When `true`, the
    /// validator raises `UNDECLARED_FIELD` on any inbound field not in
    /// the ontology (RFC-004).  Default `false`.
    #[serde(default)]
    pub compliance_mode: bool,
}

/// One row of `contract_name_history`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NameHistoryEntry {
    pub id: uuid::Uuid,
    pub contract_id: uuid::Uuid,
    pub old_name: String,
    pub new_name: String,
    pub changed_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// API request / response models
// ---------------------------------------------------------------------------

/// Request body for `POST /contracts` — identity + initial v1.0.0 draft in
/// a single transactional call.
#[derive(Debug, Deserialize)]
pub struct CreateContractRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// YAML for the auto-created v1.0.0 draft.  Must parse as a valid
    /// `Contract`.
    pub yaml_content: String,
    /// Defaults to `strict` when omitted.
    #[serde(default)]
    pub multi_stable_resolution: Option<MultiStableResolution>,
}

/// Request body for `PATCH /contracts/{id}` — identity-level metadata
/// only.  Does not touch any version's YAML; that's immutable after draft.
#[derive(Debug, Deserialize)]
pub struct PatchContractRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub multi_stable_resolution: Option<MultiStableResolution>,
}

/// Request body for `POST /contracts/{id}/versions` — create a new draft.
#[derive(Debug, Deserialize)]
pub struct CreateVersionRequest {
    /// Semver string, e.g. "1.1.0".  Must be unique per contract.
    pub version: String,
    pub yaml_content: String,
}

/// Request body for `PATCH /contracts/{id}/versions/{version}` — only
/// legal when the version is still in `draft` state.
#[derive(Debug, Deserialize)]
pub struct PatchVersionRequest {
    pub yaml_content: String,
}

/// Full contract response — identity + aggregated version summary.
///
/// `latest_stable_version` is `None` if no stable version exists yet (the
/// contract has only drafts so far, or all versions have been deprecated).
#[derive(Debug, Serialize)]
pub struct ContractResponse {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub multi_stable_resolution: MultiStableResolution,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub version_count: i64,
    pub latest_stable_version: Option<String>,
}

/// Lightweight listing row.
#[derive(Debug, Serialize)]
pub struct ContractSummary {
    pub id: uuid::Uuid,
    pub name: String,
    pub multi_stable_resolution: MultiStableResolution,
    pub latest_stable_version: Option<String>,
    pub version_count: i64,
}

/// Full response for a single version — includes YAML so the dashboard can
/// render/edit without a second fetch.
#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub id: uuid::Uuid,
    pub contract_id: uuid::Uuid,
    pub version: String,
    pub state: VersionState,
    pub yaml_content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub promoted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deprecated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// RFC-004 compliance mode flag — exposed so the dashboard can render
    /// the toggle state without re-parsing YAML.
    pub compliance_mode: bool,
}

impl From<&ContractVersion> for VersionResponse {
    fn from(v: &ContractVersion) -> Self {
        Self {
            id: v.id,
            contract_id: v.contract_id,
            version: v.version.clone(),
            state: v.state,
            yaml_content: v.yaml_content.clone(),
            created_at: v.created_at,
            promoted_at: v.promoted_at,
            deprecated_at: v.deprecated_at,
            compliance_mode: v.compliance_mode,
        }
    }
}

/// Lightweight version listing row (no YAML — saves bandwidth on list
/// views).
#[derive(Debug, Serialize)]
pub struct VersionSummary {
    pub version: String,
    pub state: VersionState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub promoted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deprecated_at: Option<chrono::DateTime<chrono::Utc>>,
}
