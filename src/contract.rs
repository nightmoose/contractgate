//! Contract types — the schema that defines what a valid data event looks like.
//!
//! A ContractGate contract is composed of three sections:
//!   - `ontology`  — field-level type/constraint definitions
//!   - `glossary`  — human-readable term definitions (business context)
//!   - `metrics`   — numeric KPI / measure definitions with range bounds
//!
//! Contracts are stored as YAML and versioned in Supabase.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

/// Supported field types inside a contract ontology.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    Integer,
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
// Glossary
// ---------------------------------------------------------------------------

/// A single term definition in the business glossary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub term: String,
    pub definition: String,
    #[serde(default)]
    pub synonyms: Vec<String>,
}

// ---------------------------------------------------------------------------
// Metric definitions
// ---------------------------------------------------------------------------

/// Defines a numeric KPI / measure that must pass range checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDefinition {
    /// Metric name (used in violation messages)
    pub name: String,
    /// JSON field path containing the metric value (dot-separated)
    pub field: String,
    /// Expected numeric type
    #[serde(rename = "type")]
    pub metric_type: MetricType,
    /// Inclusive lower bound (optional)
    #[serde(default)]
    pub min: Option<f64>,
    /// Inclusive upper bound (optional)
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
// Stored contract (includes DB metadata)
// ---------------------------------------------------------------------------

/// A contract as stored in Supabase — wraps the YAML definition with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContract {
    pub id: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Human-readable name (mirrors `Contract::name`)
    pub name: String,
    /// Semver version string
    pub version: String,
    /// Whether this contract is active / accepting ingestion
    pub active: bool,
    /// Raw YAML content of the contract
    pub yaml_content: String,
    /// Parsed contract (not stored, derived on load)
    #[serde(skip)]
    pub parsed: Option<Contract>,
}

impl StoredContract {
    /// Parse the stored YAML into a `Contract` value.
    pub fn parse(&mut self) -> anyhow::Result<()> {
        let contract: Contract = serde_yaml::from_str(&self.yaml_content)?;
        self.parsed = Some(contract);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// API request/response models
// ---------------------------------------------------------------------------

/// Request body for creating a new contract.
#[derive(Debug, Deserialize)]
pub struct CreateContractRequest {
    pub yaml_content: String,
}

/// Response returned after creating or fetching a contract.
#[derive(Debug, Serialize)]
pub struct ContractResponse {
    pub id: uuid::Uuid,
    pub name: String,
    pub version: String,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<&StoredContract> for ContractResponse {
    fn from(sc: &StoredContract) -> Self {
        ContractResponse {
            id: sc.id,
            name: sc.name.clone(),
            version: sc.version.clone(),
            active: sc.active,
            created_at: sc.created_at,
            updated_at: sc.updated_at,
        }
    }
}

/// Summary info for contract listing (lightweight).
#[derive(Debug, Serialize)]
pub struct ContractSummary {
    pub id: uuid::Uuid,
    pub name: String,
    pub version: String,
    pub active: bool,
}

/// Request to toggle contract active state.
#[derive(Debug, Deserialize)]
pub struct UpdateContractRequest {
    pub active: Option<bool>,
    pub yaml_content: Option<String>,
}
