//! ODCS v3.1.0 conformance scoring for ContractGate contract versions.
//!
//! Produces a `ConformanceReport` with four independent score dimensions so
//! operators can see at a glance how well a contract version aligns with the
//! ODCS v3.1.0 specification and ContractGate extension conventions.
//!
//! ## Dimensions
//!
//! | Dimension | Weight | Description |
//! |---|---|---|
//! | `mandatory_fields` | 0.30 | Presence of all ODCS mandatory top-level fields |
//! | `extensions` | 0.25 | Presence of required `x-contractgate-*` extensions |
//! | `round_trip_fidelity` | 0.25 | Document survives export → import as Mode A (lossless) |
//! | `quality_coverage` | 0.20 | % of ontology fields covered by ≥1 quality rule |
//!
//! All dimension scores are in [0.0, 1.0].  The `overall_score` is the
//! weighted average.  A score of 1.0 means fully conformant.

use crate::contract::{Contract, ContractIdentity, ContractVersion, ImportSource};
use crate::odcs::{self, OdcsExportInput};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Report type
// ---------------------------------------------------------------------------

/// Full conformance report for a single contract version.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConformanceReport {
    /// Version string the report covers.
    pub version: String,
    /// Presence of ODCS v3.1.0 mandatory fields: apiVersion, kind, id, version, status.
    /// Score = (fields present) / 5.
    pub mandatory_fields_score: f64,
    /// Which mandatory fields are present / missing.
    pub mandatory_fields_detail: MandatoryFieldsDetail,
    /// Presence of required CG extensions: x-contractgate-version, x-contractgate-ontology.
    /// Score = (extensions present) / 2.
    pub extensions_score: f64,
    /// Which extensions are present / missing.
    pub extensions_detail: ExtensionsDetail,
    /// Whether the exported document can be re-imported losslessly (Mode A).
    /// 1.0 = round-trip succeeded; 0.0 = failed.
    pub round_trip_fidelity_score: f64,
    /// Human-readable note about the round-trip check (error message if failed).
    pub round_trip_note: String,
    /// Fraction of ontology leaf fields that have ≥1 quality rule.
    /// 0.0 when the contract has no fields.
    pub quality_coverage_pct: f64,
    /// Number of fields covered by ≥1 quality rule.
    pub quality_covered_fields: usize,
    /// Total number of leaf fields in the ontology.
    pub total_fields: usize,
    /// Weighted overall score in [0.0, 1.0].
    pub overall_score: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MandatoryFieldsDetail {
    pub api_version: bool,
    pub kind: bool,
    pub id: bool,
    pub version: bool,
    pub status: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionsDetail {
    pub x_contractgate_version: bool,
    pub x_contractgate_ontology: bool,
}

// ---------------------------------------------------------------------------
// Score weights
// ---------------------------------------------------------------------------

const W_MANDATORY: f64 = 0.30;
const W_EXTENSIONS: f64 = 0.25;
const W_ROUND_TRIP: f64 = 0.25;
const W_QUALITY: f64 = 0.20;

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Compute conformance scores for a single contract version.
///
/// `identity`, `cv`, and `contract` are the three DB rows for the version.
pub fn compute_conformance(
    identity: &ContractIdentity,
    cv: &ContractVersion,
    contract: &Contract,
) -> ConformanceReport {
    // ── 1. Mandatory fields ──────────────────────────────────────────────────
    // We derive these by exporting to ODCS and checking the resulting document.
    // This tests the exporter too — if the exporter is broken, the score drops.
    let export_result = odcs::export_odcs(OdcsExportInput {
        identity,
        version: cv,
        contract,
    });

    let (mandatory_detail, extensions_detail, round_trip_score, round_trip_note) =
        match export_result {
            Err(e) => {
                // Export failed — mandatory and extension scores are 0.
                let mandatory = MandatoryFieldsDetail {
                    api_version: false,
                    kind: false,
                    id: false,
                    version: false,
                    status: false,
                };
                let extensions = ExtensionsDetail {
                    x_contractgate_version: false,
                    x_contractgate_ontology: false,
                };
                (
                    mandatory,
                    extensions,
                    0.0_f64,
                    format!("export failed: {e}"),
                )
            }
            Ok(odcs_yaml) => {
                // Parse the exported YAML to inspect fields.
                let (mandatory, extensions) = score_document(&odcs_yaml);

                // Round-trip fidelity: import the exported YAML and verify
                // it comes back as Mode A (lossless).
                let (rt_score, rt_note) = check_round_trip(&odcs_yaml);

                (mandatory, extensions, rt_score, rt_note)
            }
        };

    // ── 2. Compute scalar scores ─────────────────────────────────────────────
    let mandatory_score = [
        mandatory_detail.api_version,
        mandatory_detail.kind,
        mandatory_detail.id,
        mandatory_detail.version,
        mandatory_detail.status,
    ]
    .iter()
    .filter(|&&v| v)
    .count() as f64
        / 5.0;

    let extensions_score = [
        extensions_detail.x_contractgate_version,
        extensions_detail.x_contractgate_ontology,
    ]
    .iter()
    .filter(|&&v| v)
    .count() as f64
        / 2.0;

    // ── 3. Quality coverage ──────────────────────────────────────────────────
    let leaf_fields = collect_leaf_field_names(&contract.ontology.entities, "");
    let total_fields = leaf_fields.len();
    let quality_covered_fields = leaf_fields
        .iter()
        .filter(|name| contract.quality.iter().any(|r| &r.field == *name))
        .count();
    let quality_coverage_pct = if total_fields == 0 {
        // No fields → no coverage to report; treat as full coverage so the
        // score doesn't artificially penalise empty-schema contracts.
        1.0
    } else {
        quality_covered_fields as f64 / total_fields as f64
    };

    // ── 4. Overall weighted score ────────────────────────────────────────────
    let overall_score = mandatory_score * W_MANDATORY
        + extensions_score * W_EXTENSIONS
        + round_trip_score * W_ROUND_TRIP
        + quality_coverage_pct * W_QUALITY;

    ConformanceReport {
        version: cv.version.clone(),
        mandatory_fields_score: mandatory_score,
        mandatory_fields_detail: mandatory_detail,
        extensions_score,
        extensions_detail,
        round_trip_fidelity_score: round_trip_score,
        round_trip_note,
        quality_coverage_pct,
        quality_covered_fields,
        total_fields,
        overall_score,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse an ODCS YAML document and check which mandatory / extension fields
/// are present.
fn score_document(odcs_yaml: &str) -> (MandatoryFieldsDetail, ExtensionsDetail) {
    let doc: serde_yaml::Value = match serde_yaml::from_str(odcs_yaml) {
        Ok(v) => v,
        Err(_) => {
            return (
                MandatoryFieldsDetail {
                    api_version: false,
                    kind: false,
                    id: false,
                    version: false,
                    status: false,
                },
                ExtensionsDetail {
                    x_contractgate_version: false,
                    x_contractgate_ontology: false,
                },
            );
        }
    };
    let m = match doc.as_mapping() {
        Some(m) => m,
        None => {
            return (
                MandatoryFieldsDetail {
                    api_version: false,
                    kind: false,
                    id: false,
                    version: false,
                    status: false,
                },
                ExtensionsDetail {
                    x_contractgate_version: false,
                    x_contractgate_ontology: false,
                },
            );
        }
    };

    let mandatory = MandatoryFieldsDetail {
        api_version: m
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        kind: m
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        id: m
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        version: m
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        status: m
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
    };

    let extensions = ExtensionsDetail {
        x_contractgate_version: m.contains_key("x-contractgate-version"),
        x_contractgate_ontology: m.contains_key("x-contractgate-ontology"),
    };

    (mandatory, extensions)
}

/// Export → import round-trip check.  Returns (1.0, "ok") on success or
/// (0.0, error_message) on failure.
fn check_round_trip(odcs_yaml: &str) -> (f64, String) {
    match odcs::import_odcs(odcs_yaml) {
        Ok(result) if result.import_source == ImportSource::Odcs => {
            (1.0, "round-trip succeeded (Mode A lossless)".into())
        }
        Ok(result) => (
            0.5,
            format!(
                "import succeeded but as {:?} (expected Odcs / Mode A lossless)",
                result.import_source
            ),
        ),
        Err(e) => (0.0, format!("round-trip import failed: {e}")),
    }
}

/// Recursively collect dot-notation paths of all leaf fields in the ontology.
fn collect_leaf_field_names(fields: &[crate::contract::FieldDefinition], prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    for f in fields {
        let path = if prefix.is_empty() {
            f.name.clone()
        } else {
            format!("{}.{}", prefix, f.name)
        };
        if f.field_type == crate::contract::FieldType::Object {
            if let Some(props) = &f.properties {
                names.extend(collect_leaf_field_names(props, &path));
                continue;
            }
        }
        names.push(path);
    }
    names
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ContractIdentity, ContractVersion, FieldDefinition, FieldType, ImportSource,
        MultiStableResolution, Ontology, QualityRule, QualityRuleType, VersionState,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn make_identity() -> ContractIdentity {
        ContractIdentity {
            id: Uuid::new_v4(),
            name: "test_contract".into(),
            description: Some("conformance test".into()),
            multi_stable_resolution: MultiStableResolution::Strict,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            pii_salt: vec![],
        }
    }

    fn make_version(id: Uuid, yaml: &str) -> ContractVersion {
        ContractVersion {
            id: Uuid::new_v4(),
            contract_id: id,
            version: "1.0.0".into(),
            state: VersionState::Stable,
            yaml_content: yaml.into(),
            created_at: Utc::now(),
            promoted_at: Some(Utc::now()),
            deprecated_at: None,
            compliance_mode: false,
            import_source: ImportSource::Native,
            requires_review: false,
        }
    }

    fn simple_contract(with_quality: bool) -> Contract {
        let quality = if with_quality {
            vec![QualityRule {
                field: "user_id".into(),
                rule_type: QualityRuleType::Completeness,
                description: None,
                max_age_seconds: None,
                scope: None,
                threshold: None,
            }]
        } else {
            vec![]
        };
        Contract {
            version: "1.0".into(),
            name: "test_contract".into(),
            description: None,
            compliance_mode: false,
            ontology: Ontology {
                entities: vec![
                    FieldDefinition {
                        name: "user_id".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: None,
                        allowed_values: None,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                        transform: None,
                    },
                    FieldDefinition {
                        name: "amount".into(),
                        field_type: FieldType::Float,
                        required: false,
                        pattern: None,
                        allowed_values: None,
                        min: Some(0.0),
                        max: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                        transform: None,
                    },
                ],
            },
            glossary: vec![],
            metrics: vec![],
            quality,
        }
    }

    #[test]
    fn fully_conformant_native_contract_scores_high() {
        let identity = make_identity();
        let contract = simple_contract(true);
        let yaml = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, &yaml);

        let report = compute_conformance(&identity, &cv, &contract);

        assert_eq!(report.mandatory_fields_score, 1.0, "all mandatory present");
        assert_eq!(report.extensions_score, 1.0, "all extensions present");
        assert_eq!(report.round_trip_fidelity_score, 1.0, "round-trip lossless");
        // quality_coverage: 1 of 2 fields covered → 0.5
        assert!((report.quality_coverage_pct - 0.5).abs() < 0.001);
        // overall must be > 0.8
        assert!(
            report.overall_score > 0.80,
            "expected overall > 0.80, got {}",
            report.overall_score
        );
    }

    #[test]
    fn zero_quality_rules_gives_zero_coverage() {
        let identity = make_identity();
        let contract = simple_contract(false);
        let yaml = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, &yaml);

        let report = compute_conformance(&identity, &cv, &contract);

        assert_eq!(report.quality_coverage_pct, 0.0);
        assert_eq!(report.quality_covered_fields, 0);
        assert_eq!(report.total_fields, 2);
    }

    #[test]
    fn round_trip_fidelity_passes_for_native_export() {
        let identity = make_identity();
        let contract = simple_contract(false);
        let yaml = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, &yaml);

        let report = compute_conformance(&identity, &cv, &contract);

        assert_eq!(
            report.round_trip_fidelity_score, 1.0,
            "native export must round-trip as Mode A"
        );
        assert!(
            report.round_trip_note.contains("lossless"),
            "note: {}",
            report.round_trip_note
        );
    }
}
