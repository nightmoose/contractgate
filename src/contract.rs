//! Contract types — the schema that defines what a valid data event looks like.
//!
//! A ContractGate contract is composed of three sections:
//!   - `ontology`  — field-level type/constraint definitions
//!   - `glossary`  — human-readable term definitions (business context)
//!   - `metrics`   — numeric KPI / measure definitions with range bounds
//!
//! Contracts are stored as YAML and versioned in Supabase.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

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
    /// Data quality rules — enforced at ingest time.
    /// Completeness, validity, freshness, and uniqueness checks declared here
    /// are evaluated per-event (or per-batch for uniqueness) and produce
    /// structured violations just like ontology checks.
    #[serde(default)]
    pub quality: Vec<QualityRule>,
    /// RFC-030: controls what happens to fields in the outbound payload that
    /// are not declared in `ontology.entities`.
    ///   - `off`   — pass through untouched (backwards-compatible default).
    ///   - `strip` — remove from response; record in per-record outcome.
    ///   - `fail`  — treat as a violation, subject to the RFC-029 disposition
    ///               (`block` / `fail` / `tag`).  Field is stripped even on `tag`.
    #[serde(default)]
    pub egress_leakage_mode: EgressLeakageMode,
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

impl TransformKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransformKind::Mask => "mask",
            TransformKind::Hash => "hash",
            TransformKind::Drop => "drop",
            TransformKind::Redact => "redact",
        }
    }
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

impl MaskStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            MaskStyle::Opaque => "opaque",
            MaskStyle::FormatPreserving => "format_preserving",
        }
    }
}

// ---------------------------------------------------------------------------
// Egress leakage mode (RFC-030)
// ---------------------------------------------------------------------------

/// What the egress guard does when an outbound payload contains a field that
/// is not declared in `ontology.entities`.
///
/// The mode is version-level: one contract version may want strict egress
/// (`fail`) while another stays permissive (`off`) during migration.  The
/// three-way split is intentional — ingest `compliance_mode` is a boolean
/// (strip-for-storage), but egress needs to distinguish "silently strip"
/// from "surface as a violation" because silent stripping can hide real
/// producer bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressLeakageMode {
    /// Pass undeclared fields through untouched.  Backwards-compatible default.
    #[default]
    Off,
    /// Remove undeclared fields from the response.  Records the stripped field
    /// names in the per-record egress outcome.
    Strip,
    /// Treat each undeclared field as a `LeakageViolation`.  The record is
    /// subject to the RFC-029 disposition (`block` / `fail` / `tag`).
    /// Undeclared fields are stripped from the response even when the record
    /// passes through (e.g. under `tag` disposition).
    Fail,
}

impl EgressLeakageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            EgressLeakageMode::Off => "off",
            EgressLeakageMode::Strip => "strip",
            EgressLeakageMode::Fail => "fail",
        }
    }
}

impl FromStr for EgressLeakageMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(Self::Off),
            "strip" => Ok(Self::Strip),
            "fail" => Ok(Self::Fail),
            _ => Err(format!("invalid EgressLeakageMode: {s:?}")),
        }
    }
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
// Data quality rules
// ---------------------------------------------------------------------------

/// A single data-quality rule evaluated at ingest time.
///
/// Quality rules are a superset of ontology-level constraints: they add
/// *semantic* checks (freshness, uniqueness) and make *explicit* the
/// completeness and validity expectations that ontology constraints only
/// imply.  Violations are reported with the same `Violation` struct as
/// ontology checks and included in the overall `ValidationResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRule {
    /// Field path to check (dot-notation, e.g. `"user.id"`).
    pub field: String,
    /// Which check to apply.
    #[serde(rename = "type")]
    pub rule_type: QualityRuleType,
    /// Human-readable description (informational; included in ODCS export).
    #[serde(default)]
    pub description: Option<String>,
    // ── Freshness params ───────────────────────────────────────────────────
    /// Maximum age of a timestamp value, in seconds.  The field must hold
    /// a Unix epoch (integer seconds or milliseconds — detected by magnitude).
    /// Only used when `rule_type == Freshness`.
    #[serde(default)]
    pub max_age_seconds: Option<u64>,
    // ── Uniqueness params ──────────────────────────────────────────────────
    /// Scope for uniqueness deduplication.  Only `"batch"` is supported today.
    /// Only used when `rule_type == Uniqueness`.
    #[serde(default)]
    pub scope: Option<UniqueScope>,
    // ── Validity threshold ─────────────────────────────────────────────────
    /// Fraction of events in a batch that must pass this rule (0.0 – 1.0).
    /// Useful for validity and completeness rules where a small tail of
    /// missing/malformed values is acceptable in practice.  Default `1.0`
    /// (all events must pass).  Per-event violations are still reported
    /// even when the threshold is not exceeded — this controls whether the
    /// *batch* is considered failed.  `null` means strict (= 1.0).
    #[serde(default)]
    pub threshold: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityRuleType {
    /// Field must be present, non-null, and (for strings) non-empty.
    Completeness,
    /// Field value must satisfy the ontology-declared constraints (pattern,
    /// enum, min/max range, length).  This is an explicit overlay on top of
    /// ontology validation — violations are also reported via the standard
    /// ontology check path, but this rule makes the *intent* explicit and
    /// participates in the quality-coverage conformance score.
    Validity,
    /// Field must hold a Unix epoch timestamp no older than `max_age_seconds`
    /// relative to the ingest wall-clock time.  Detects stale / replayed events.
    Freshness,
    /// Field value must be unique across all events in the same ingest batch.
    /// Detects duplicate events before they land in downstream sinks.
    Uniqueness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UniqueScope {
    /// Deduplicate within the current ingest batch only.  Cross-batch
    /// deduplication requires an external store (not yet supported).
    #[default]
    Batch,
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
}

/// Parsing is intentionally case-sensitive: DB enum values and YAML are always
/// lowercase, and accepting "DRAFT" would mask producer bugs that we'd rather
/// surface loudly.  See the `version_state_parse_*` tests in `src/tests.rs`.
impl FromStr for VersionState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "stable" => Ok(Self::Stable),
            "deprecated" => Ok(Self::Deprecated),
            _ => Err(format!("invalid VersionState: {s:?}")),
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
}

impl FromStr for MultiStableResolution {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "strict" => Ok(Self::Strict),
            "fallback" => Ok(Self::Fallback),
            _ => Err(format!("invalid MultiStableResolution: {s:?}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Import provenance (ODCS import tracking)
// ---------------------------------------------------------------------------

/// Where a contract version originated.
///
/// Used to track ODCS import fidelity and to gate promotion on human review
/// when a stripped ODCS document (no `x-contractgate-*` extensions) was
/// imported — validation constraints and PII transforms may not have been
/// fully recoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    /// Created natively in ContractGate (default).
    #[default]
    Native,
    /// Imported from an ODCS document with `x-contractgate-*` extensions —
    /// full round-trip fidelity preserved.
    Odcs,
    /// Imported from an ODCS document without `x-contractgate-*` extensions.
    /// Validation constraints, PII transforms, glossary, and metrics may be
    /// partially or fully unrecoverable.  `requires_review` is set to `true`
    /// and promotion to stable is blocked until explicitly cleared.
    OdcsStripped,
    /// Imported from a published contract reference (RFC-032).  Provenance
    /// (publication ref, import mode, imported_at) is recorded on the
    /// `contracts` row.
    Publication,
}

impl ImportSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportSource::Native => "native",
            ImportSource::Odcs => "odcs",
            ImportSource::OdcsStripped => "odcs_stripped",
            ImportSource::Publication => "publication",
        }
    }
}

impl FromStr for ImportSource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "native" => Ok(Self::Native),
            "odcs" => Ok(Self::Odcs),
            "odcs_stripped" => Ok(Self::OdcsStripped),
            "publication" => Ok(Self::Publication),
            _ => Err(format!("invalid ImportSource: {s:?}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Publication types (RFC-032)
// ---------------------------------------------------------------------------

/// Visibility level for a published contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublicationVisibility {
    /// Anyone with the publication ref can fetch it.
    Public,
    /// Requires both the ref and an unguessable link token.
    Link,
    /// Only orgs explicitly granted access (wired up by RFC-033).
    Org,
}

impl PublicationVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Link => "link",
            Self::Org => "org",
        }
    }
}

impl FromStr for PublicationVisibility {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "link" => Ok(Self::Link),
            "org" => Ok(Self::Org),
            _ => Err(format!("invalid PublicationVisibility: {s:?}")),
        }
    }
}

/// How an imported contract stays linked to its source publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// One-time copy.  Provenance is recorded but the contract never auto-updates.
    Snapshot,
    /// Copy with a live link.  When the provider publishes a newer version,
    /// `import-status` surfaces an "update available" signal — never auto-applies.
    Subscribe,
}

impl ImportMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Subscribe => "subscribe",
        }
    }
}

impl FromStr for ImportMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "snapshot" => Ok(Self::Snapshot),
            "subscribe" => Ok(Self::Subscribe),
            _ => Err(format!("invalid ImportMode: {s:?}")),
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
    /// RFC-030: mirrors `contract_versions.egress_leakage_mode`.
    /// Controls handling of undeclared fields in outbound payloads.
    /// Default `off` (pass-through) for backwards compatibility.
    #[serde(default)]
    pub egress_leakage_mode: EgressLeakageMode,
    /// Where this version originated (native / odcs / odcs_stripped).
    #[serde(default)]
    pub import_source: ImportSource,
    /// When `true`, the version was imported from a stripped ODCS document
    /// (no `x-contractgate-*` extensions) and requires human review before
    /// it may be promoted to stable.
    #[serde(default)]
    pub requires_review: bool,
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

/// Request body for `POST /contracts/deploy` (RFC-028).
///
/// Atomically: find-or-create the contract identity by name, insert the
/// version as `stable` (parsed_json generated at call time), and deprecate
/// all prior stable versions — unless pending quarantine events exist, in
/// which case the call is rejected to preserve the causal audit chain.
///
/// Admin-only: the endpoint requires a service-role API key.
#[derive(Debug, Deserialize)]
pub struct DeployContractRequest {
    /// Contract name — matches `Contract.name` in the YAML.
    pub name: String,
    /// Raw YAML.  Parsed server-side; version extracted from the YAML itself.
    pub yaml_content: String,
    /// PMS vendor or logical feed name (e.g. "yardi", "realpage", "entrata").
    #[serde(default)]
    pub source: Option<String>,
    /// CI job ID or username that triggered the deploy.
    #[serde(default)]
    pub deployed_by: Option<String>,
}

/// Response for `POST /contracts/deploy` — the newly stable version.
#[derive(Debug, Serialize)]
pub struct DeployContractResponse {
    pub contract_id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub deployed_by: Option<String>,
    pub deployed_at: chrono::DateTime<chrono::Utc>,
    /// How many prior stable versions were deprecated.
    pub deprecated_count: i64,
}

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
    /// RFC-030 egress leakage mode — exposed so the dashboard can render
    /// the selector without re-parsing YAML.
    pub egress_leakage_mode: EgressLeakageMode,
    /// Import provenance.
    pub import_source: ImportSource,
    /// When `true`, human review is required before promotion to stable.
    pub requires_review: bool,
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
            egress_leakage_mode: v.egress_leakage_mode,
            import_source: v.import_source,
            requires_review: v.requires_review,
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
    pub import_source: ImportSource,
    pub requires_review: bool,
}
