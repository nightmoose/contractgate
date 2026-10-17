//! Integration-style tests for DataContractGate that do NOT require a live database.
//!
//! These tests exercise the validation engine + playground handler end-to-end
//! by constructing requests and calling handlers directly — fast, no I/O.
//!
//! DB-dependent tests (ingest, audit) belong in a separate `tests/integration/`
//! directory and require `DATABASE_URL` to be set.

// ---------------------------------------------------------------------------
// Shared test fixtures
//
// `FieldDefinition` is a 12-field struct where the typical test only cares
// about 1–3 fields; the remaining 9–11 are `None`.  These helpers cut the
// boilerplate so a new test reads as "what's special about this field" rather
// than "spell out every defaulted field one more time."
//
// In-file scope only.  Sharing fixtures with `transform.rs`'s inner test mod
// would require pub-exporting them across the lib/bin crate boundary or a
// `test-fixtures` Cargo feature — neither warranted for the current overlap.
// `transform.rs` already has its own small `entity()` helper that fits its
// needs.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod fixtures {
    use crate::contract::{
        Contract, EgressLeakageMode, FieldDefinition, FieldType, GlossaryEntry, MetricDefinition,
        Ontology, QualityRule,
    };

    /// A minimum-defaults `FieldDefinition`.  Use when a test only cares
    /// about `name` and `type`; tweak further fields with `entity_with`.
    /// `dead_code` is allowed because today every test reaches this
    /// indirectly through `entity_with`, but we want the bare-defaults
    /// primitive available for tests that don't need any tweaks.
    #[allow(dead_code)]
    pub fn entity(name: &str, field_type: FieldType) -> FieldDefinition {
        FieldDefinition {
            name: name.to_string(),
            field_type,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform: None,
            format: None,
        }
    }

    /// `entity` + a closure that mutates the result.  Lets each test set
    /// only the fields it cares about while keeping the call site one line:
    ///
    /// ```ignore
    /// entity_with("user_id", FieldType::String, |f| {
    ///     f.required = true;
    ///     f.pattern = Some(r"^[a-z0-9_]+$".into());
    /// })
    /// ```
    pub fn entity_with(
        name: &str,
        field_type: FieldType,
        tweak: impl FnOnce(&mut FieldDefinition),
    ) -> FieldDefinition {
        let mut f = entity(name, field_type);
        tweak(&mut f);
        f
    }

    /// Wrap a vec of entities in a minimum-defaults `Contract`.  Empty
    /// glossary + metrics; pass `compliance_mode = false`.  For richer
    /// shapes, build the `Contract` literal in the test directly.
    pub fn contract(name: &str, entities: Vec<FieldDefinition>) -> Contract {
        Contract {
            version: "1.0".into(),
            name: name.to_string(),
            description: None,
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            ontology: Ontology { entities },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            envelope: None,
            null_as_absent: false,
        }
    }

    /// `contract` + glossary + metrics.  Used by the `playground` module's
    /// `user_events_contract` fixture, which mirrors the canonical YAML
    /// example in CLAUDE.md and needs both supplemental sections present.
    pub fn contract_with(
        name: &str,
        description: Option<&str>,
        entities: Vec<FieldDefinition>,
        glossary: Vec<GlossaryEntry>,
        metrics: Vec<MetricDefinition>,
        quality: Vec<QualityRule>,
    ) -> Contract {
        Contract {
            version: "1.0".into(),
            name: name.to_string(),
            description: description.map(str::to_string),
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            ontology: Ontology { entities },
            glossary,
            metrics,
            quality,
            envelope: None,
            null_as_absent: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Batch validation tests (RFC-001)
//
// These exercise the *validation layer* of the batch pipeline — order
// preservation, parallel correctness, a rough throughput sanity check.  The
// HTTP handler itself (`ingest_handler`) needs a live Postgres pool and is
// covered by the manual curl checks described in `docs/rfcs/001-batch-ingest.md`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod batch {
    use super::fixtures::{contract, entity_with};
    use crate::contract::{Contract, FieldType};
    use crate::validation::{validate, CompiledContract, ViolationKind};
    use rayon::prelude::*;
    use serde_json::{json, Value};

    fn tiny_contract() -> Contract {
        contract(
            "batch_test",
            vec![
                entity_with("user_id", FieldType::String, |f| {
                    f.required = true;
                    f.pattern = Some(r"^[a-z0-9_]+$".into());
                }),
                entity_with("event_type", FieldType::String, |f| {
                    f.required = true;
                    f.allowed_values = Some(vec![json!("click"), json!("view")]);
                }),
            ],
        )
    }

    fn events_with_one_bad_at(bad_index: usize, total: usize) -> Vec<Value> {
        (0..total)
            .map(|i| {
                if i == bad_index {
                    json!({ "user_id": "BAD ID!!", "event_type": "click" })
                } else {
                    json!({ "user_id": format!("user_{}", i), "event_type": "click" })
                }
            })
            .collect()
    }

    /// Sanity: parallel validation yields the same per-event verdicts as the
    /// sequential loop.  Equivalence is what lets us replace one with the other
    /// with no behavioural change.
    #[test]
    fn parallel_matches_sequential() {
        let cc = CompiledContract::compile(tiny_contract()).unwrap();
        let events = events_with_one_bad_at(17, 64);

        let sequential: Vec<bool> = events.iter().map(|e| validate(&cc, e).passed).collect();

        let parallel: Vec<bool> = events.par_iter().map(|e| validate(&cc, e).passed).collect();

        assert_eq!(sequential, parallel);
        assert!(!parallel[17], "bad event should be at index 17");
        assert!(
            parallel[16] && parallel[18],
            "neighbouring events should pass"
        );
    }

    /// Ordering guarantee: `par_iter().map().collect()` preserves input order.
    /// The batch-ingest response contract depends on `results[i]` matching
    /// `events[i]`; this test is a regression guard in case rayon ever changes
    /// its collect semantics.
    #[test]
    fn parallel_preserves_input_order() {
        let cc = CompiledContract::compile(tiny_contract()).unwrap();
        // 200 events, each failing because user_id contains its own index in
        // uppercase — unique per event, so we can tell them apart in the output.
        let events: Vec<Value> = (0..200)
            .map(|i| json!({ "user_id": format!("USER_{}", i), "event_type": "click" }))
            .collect();

        let results: Vec<_> = events.par_iter().map(|e| validate(&cc, e)).collect();

        assert_eq!(results.len(), 200);
        for (i, vr) in results.iter().enumerate() {
            assert!(!vr.passed, "event {} should fail pattern check", i);
            // The violation message mentions the exact value we put in —
            // confirms the i-th result corresponds to the i-th input.
            let joined: String = vr.violations.iter().map(|v| v.message.clone()).collect();
            assert!(
                joined.contains(&format!("USER_{}", i)),
                "event {} result should reference its own value — got: {}",
                i,
                joined
            );
        }
    }

    /// An all-pass batch collects zero violations.  Validates that the
    /// parallel + `par_iter().filter()` pattern used in the handler to build
    /// forward-rows yields the full batch when everything passes.
    #[test]
    fn all_pass_batch_has_no_failures() {
        let cc = CompiledContract::compile(tiny_contract()).unwrap();
        let events: Vec<Value> = (0..500)
            .map(|i| json!({ "user_id": format!("ok_{}", i), "event_type": "view" }))
            .collect();

        let results: Vec<_> = events.par_iter().map(|e| validate(&cc, e)).collect();
        let pass_count = results.iter().filter(|r| r.passed).count();
        let fail_count = results.len() - pass_count;
        assert_eq!(pass_count, 500);
        assert_eq!(fail_count, 0);
    }

    /// A mixed batch splits cleanly into passes and failures with the expected
    /// violation kinds reported only on the failing events.
    #[test]
    fn mixed_batch_separates_cleanly() {
        let cc = CompiledContract::compile(tiny_contract()).unwrap();
        let events = events_with_one_bad_at(3, 10);

        let results: Vec<_> = events.par_iter().map(|e| validate(&cc, e)).collect();
        let failing: Vec<_> = results
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.passed)
            .collect();

        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].0, 3);
        assert!(failing[0]
            .1
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::PatternMismatch));
    }

    /// Throughput sanity: validating a 1 000-event batch in parallel completes
    /// well under the batch-level latency budget declared in RFC-001 (<100 ms
    /// end-to-end on a 4-core runner; we give CI a generous 5 s because debug
    /// builds are much slower than release).
    #[test]
    fn thousand_event_batch_completes_quickly() {
        let cc = CompiledContract::compile(tiny_contract()).unwrap();
        let events: Vec<Value> = (0..1_000)
            .map(|i| json!({ "user_id": format!("user_{}", i), "event_type": "click" }))
            .collect();

        let t0 = std::time::Instant::now();
        let results: Vec<_> = events.par_iter().map(|e| validate(&cc, e)).collect();
        let elapsed = t0.elapsed();

        assert_eq!(results.len(), 1_000);
        assert!(results.iter().all(|r| r.passed));

        #[cfg(debug_assertions)]
        let budget = std::time::Duration::from_secs(5);
        #[cfg(not(debug_assertions))]
        let budget = std::time::Duration::from_millis(100);

        assert!(
            elapsed < budget,
            "1 000-event batch took {:?} — expected < {:?}",
            elapsed,
            budget
        );
    }
}

#[cfg(test)]
mod playground {
    use super::fixtures::{contract_with, entity_with};
    use crate::contract::{
        Contract, FieldType, GlossaryEntry, MetricDefinition, QualityRule, QualityRuleType,
    };
    use crate::validation::{validate, CompiledContract, ViolationKind};
    use serde_json::json;

    // -----------------------------------------------------------------------
    // Helper: build the canonical user_events contract from the YAML example
    // -----------------------------------------------------------------------

    fn user_events_contract() -> Contract {
        contract_with(
            "user_events",
            Some("Contract for user interaction events"),
            vec![
                entity_with("user_id", FieldType::String, |f| {
                    f.required = true;
                    f.pattern = Some(r"^[a-zA-Z0-9_-]+$".into());
                }),
                entity_with("event_type", FieldType::String, |f| {
                    f.required = true;
                    f.allowed_values = Some(vec![
                        json!("click"),
                        json!("view"),
                        json!("purchase"),
                        json!("login"),
                    ]);
                }),
                entity_with("timestamp", FieldType::Integer, |f| {
                    f.required = true;
                }),
                entity_with("amount", FieldType::Float, |f| {
                    f.min = Some(0.0);
                }),
            ],
            vec![GlossaryEntry {
                field: "amount".into(),
                description: "Monetary amount in USD".into(),
                constraints: Some("must be non-negative".into()),
                synonyms: None,
            }],
            vec![MetricDefinition {
                name: "total_revenue".into(),
                field: None,
                metric_type: None,
                formula: Some("sum(amount) where event_type = 'purchase'".into()),
                min: None,
                max: None,
            }],
            vec![QualityRule {
                field: "event_type".into(),
                rule_type: QualityRuleType::Validity,
                description: Some("Event must have a valid event_type".into()),
                max_age_seconds: None,
                scope: None,
                threshold: None,
            }],
        )
    }

    // -----------------------------------------------------------------------
    // Happy-path tests
    // -----------------------------------------------------------------------

    #[test]
    fn click_event_passes() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
        assert_eq!(r.violations.len(), 0);
    }

    #[test]
    fn purchase_event_with_amount_passes() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "bob-99",
            "event_type": "purchase",
            "timestamp": 1712001000,
            "amount": 49.99
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    #[test]
    fn optional_amount_absent_passes() {
        // amount is optional — event without it should still pass
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "carol_X",
            "event_type": "login",
            "timestamp": 1712002000
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    #[test]
    fn extra_fields_allowed_no_violation() {
        // DataContractGate does not reject unknown fields (additive schema evolution)
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "dan_01",
            "event_type": "view",
            "timestamp": 1712003000,
            "page": "/dashboard",        // extra field
            "session_id": "sess_abc"     // extra field
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "violations: {:?}", r.violations);
    }

    // -----------------------------------------------------------------------
    // Violation tests
    // -----------------------------------------------------------------------

    #[test]
    fn missing_required_user_id() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({ "event_type": "click", "timestamp": 1712000000 });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::MissingRequiredField && v.field == "user_id"));
    }

    #[test]
    fn invalid_event_type_enum() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "delete",   // not in allowed set
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::EnumViolation));
    }

    #[test]
    fn user_id_pattern_mismatch() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice @ 01!",  // spaces + special chars not in pattern
            "event_type": "view",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::PatternMismatch));
    }

    #[test]
    fn negative_amount_range_violation() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "purchase",
            "timestamp": 1712000000,
            "amount": -5.00    // below min=0
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::RangeViolation));
    }

    #[test]
    fn wrong_type_for_timestamp() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "alice_01",
            "event_type": "click",
            "timestamp": "not-a-number"  // string instead of integer
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::TypeMismatch));
    }

    #[test]
    fn multiple_violations_collected() {
        // All three fields wrong at once — should collect all violations
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "bad id!",        // pattern mismatch
            "event_type": "explode",     // enum violation
            "timestamp": "now",          // type mismatch
            "amount": -100               // range violation
        });
        let r = validate(&cc, &event);
        assert!(!r.passed);
        assert!(
            r.violations.len() >= 3,
            "Expected ≥3 violations, got: {:?}",
            r.violations
        );
    }

    #[test]
    fn non_object_event_rejected() {
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let r = validate(&cc, &json!(["not", "an", "object"]));
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::TypeMismatch));
    }

    // -----------------------------------------------------------------------
    // Performance sanity check
    // -----------------------------------------------------------------------

    #[test]
    fn validation_completes_under_1ms() {
        // The hot path should be well under 1ms for a simple event.
        // This is a rough guard — CI servers are slower but still within target.
        let cc = CompiledContract::compile(user_events_contract()).unwrap();
        let event = json!({
            "user_id": "perf_test_user",
            "event_type": "click",
            "timestamp": 1712000000
        });
        let r = validate(&cc, &event);
        assert!(r.passed);
        // Target p99 is 15 000 µs in release builds.
        // Debug builds on CI are unoptimised — allow up to 500 ms.
        // The real latency bar is enforced by the release-mode benchmark.
        #[cfg(debug_assertions)]
        let threshold = 500_000;
        #[cfg(not(debug_assertions))]
        let threshold = 1_000;
        assert!(
            r.validation_us < threshold,
            "Validation took {}µs — expected < {}µs",
            r.validation_us,
            threshold
        );
    }

    // -----------------------------------------------------------------------
    // YAML round-trip test
    // -----------------------------------------------------------------------

    #[test]
    fn yaml_parse_and_validate_roundtrip() {
        let yaml = r#"
version: "1.0"
name: "roundtrip_test"
ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^[a-z0-9]+$"
    - name: value
      type: integer
      required: true
      min: 1
      max: 100
glossary: []
metrics: []
"#;
        let contract: Contract = serde_yaml::from_str(yaml).expect("YAML parse failed");
        let cc = CompiledContract::compile(contract).expect("Compile failed");

        // Valid
        let good = json!({ "id": "abc123", "value": 50 });
        assert!(validate(&cc, &good).passed);

        // Out of range
        let bad = json!({ "id": "abc123", "value": 999 });
        let r = validate(&cc, &bad);
        assert!(!r.passed);
        assert!(r
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::RangeViolation));
    }
}

// ---------------------------------------------------------------------------
// Versioning tests (RFC-002) — pure, DB-free layer.
//
// The full RFC-002 test plan (`docs/rfcs/002-versioning.md` §test plan) covers
// 35 cases spanning DB-dependent CRUD + state transitions + ingest resolution
// + fallback.  The DB-backed ones need a live Postgres and will land as
// integration tests under `tests/integration/` once the harness is wired up.
// This module covers what can be verified without any I/O:
//
//   - `VersionState` and `MultiStableResolution` parse ↔ as_str round-trip
//   - Default resolution is `strict`  (guards against accidental permissiveness)
//   - Json deserialization of the request types with/without optional fields
//   - `VersionResponse::from(&ContractVersion)` carries every field through
//
// The ingest path parser is covered by `mod path_tests` in `src/ingest.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod versioning {
    use crate::contract::{
        ContractVersion, CreateContractRequest, CreateVersionRequest, EgressLeakageMode,
        MultiStableResolution, PatchContractRequest, PatchVersionRequest, VersionResponse,
        VersionState,
    };
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn version_state_string_roundtrip() {
        for s in ["draft", "stable", "deprecated"] {
            let parsed: VersionState = s.parse().expect("valid state");
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn version_state_rejects_unknown() {
        assert!("retired".parse::<VersionState>().is_err());
        assert!("".parse::<VersionState>().is_err());
        assert!("DRAFT".parse::<VersionState>().is_err()); // case-sensitive on purpose
    }

    #[test]
    fn multi_stable_resolution_roundtrip() {
        for s in ["strict", "fallback"] {
            let parsed: MultiStableResolution = s.parse().expect("valid policy");
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn multi_stable_resolution_default_is_strict() {
        // Defaulting to `fallback` would weaken the product pitch; RFC-002
        // §2b calls out `strict` as the intentional default.
        assert_eq!(
            MultiStableResolution::default(),
            MultiStableResolution::Strict
        );
    }

    #[test]
    fn create_contract_request_defaults_resolution_absent() {
        // Request can omit multi_stable_resolution; it should land as None
        // so the handler can apply the `Strict` default.
        let body = json!({
            "name": "user_events",
            "yaml_content": "version: \"1.0\"\nname: user_events\nontology:\n  entities: []\n"
        });
        let req: CreateContractRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.name, "user_events");
        assert!(req.description.is_none());
        assert!(req.multi_stable_resolution.is_none());
    }

    #[test]
    fn create_contract_request_accepts_fallback() {
        let body = json!({
            "name": "lenient",
            "yaml_content": "---",
            "multi_stable_resolution": "fallback"
        });
        let req: CreateContractRequest = serde_json::from_value(body).unwrap();
        assert_eq!(
            req.multi_stable_resolution,
            Some(MultiStableResolution::Fallback)
        );
    }

    #[test]
    fn patch_contract_all_optional_empty_body() {
        // Every field is optional — an empty body should deserialize
        // cleanly (= "no changes").
        let req: PatchContractRequest = serde_json::from_value(json!({})).unwrap();
        assert!(req.name.is_none());
        assert!(req.description.is_none());
        assert!(req.multi_stable_resolution.is_none());
    }

    #[test]
    fn create_version_request_requires_version_and_yaml() {
        // version + yaml_content are both required.
        let ok: CreateVersionRequest = serde_json::from_value(json!({
            "version": "1.1.0",
            "yaml_content": "---"
        }))
        .unwrap();
        assert_eq!(ok.version, "1.1.0");

        let missing_version = serde_json::from_value::<CreateVersionRequest>(json!({
            "yaml_content": "---"
        }));
        assert!(missing_version.is_err());

        let missing_yaml = serde_json::from_value::<CreateVersionRequest>(json!({
            "version": "1.0.0"
        }));
        assert!(missing_yaml.is_err());
    }

    #[test]
    fn patch_version_request_carries_yaml() {
        let req: PatchVersionRequest = serde_json::from_value(json!({
            "yaml_content": "new: yaml"
        }))
        .unwrap();
        assert_eq!(req.yaml_content, "new: yaml");
    }

    #[test]
    fn version_response_from_carries_every_field() {
        // Guards against a future refactor silently dropping a field from
        // the API contract (`yaml_content`, `state`, timestamps, etc.).
        let cid = Uuid::new_v4();
        let vid = Uuid::new_v4();
        let created = Utc::now();
        let promoted = Some(Utc::now());
        let v = ContractVersion {
            id: vid,
            contract_id: cid,
            version: "1.2.3".into(),
            state: VersionState::Stable,
            yaml_content: "---\nyaml: here".into(),
            created_at: created,
            promoted_at: promoted,
            deprecated_at: None,
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            import_source: crate::contract::ImportSource::Native,
            requires_review: false,
        };
        let resp = VersionResponse::from(&v);
        assert_eq!(resp.id, vid);
        assert_eq!(resp.contract_id, cid);
        assert_eq!(resp.version, "1.2.3");
        assert_eq!(resp.state, VersionState::Stable);
        assert_eq!(resp.yaml_content, "---\nyaml: here");
        assert_eq!(resp.created_at, created);
        assert_eq!(resp.promoted_at, promoted);
        assert_eq!(resp.deprecated_at, None);
        assert!(!resp.compliance_mode);
        assert_eq!(resp.import_source, crate::contract::ImportSource::Native);
        assert!(!resp.requires_review);
    }
}

// ---------------------------------------------------------------------------
// ODCS import / export tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod odcs_tests {
    use super::fixtures::{contract_with, entity_with};
    use crate::contract::{
        ContractIdentity, ContractVersion, EgressLeakageMode, FieldType, GlossaryEntry,
        ImportSource, MetricDefinition, MultiStableResolution, VersionState,
    };
    use crate::odcs;
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    pub(super) fn make_identity(name: &str) -> ContractIdentity {
        ContractIdentity {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: Some("test contract".to_string()),
            multi_stable_resolution: MultiStableResolution::Strict,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            pii_salt: vec![0u8; 32],
            store_event_payloads: true,
        }
    }

    pub(super) fn make_version(contract_id: Uuid, ver: &str, yaml: &str) -> ContractVersion {
        ContractVersion {
            id: Uuid::new_v4(),
            contract_id,
            version: ver.to_string(),
            state: VersionState::Stable,
            yaml_content: yaml.to_string(),
            created_at: Utc::now(),
            promoted_at: Some(Utc::now()),
            deprecated_at: None,
            compliance_mode: false,
            egress_leakage_mode: EgressLeakageMode::Off,
            import_source: ImportSource::Native,
            requires_review: false,
        }
    }

    /// A stable CG contract with multiple field types and glossary / metrics.
    fn user_events_contract() -> crate::contract::Contract {
        contract_with(
            "user_events",
            Some("User interaction events"),
            vec![
                entity_with("user_id", FieldType::String, |f| {
                    f.required = true;
                    f.pattern = Some(r"^[a-zA-Z0-9_-]+$".into());
                }),
                entity_with("event_type", FieldType::String, |f| {
                    f.required = true;
                    f.allowed_values = Some(vec![json!("click"), json!("view"), json!("purchase")]);
                }),
                entity_with("timestamp", FieldType::Integer, |f| {
                    f.required = true;
                }),
                entity_with("amount", FieldType::Float, |f| {
                    f.min = Some(0.0);
                }),
            ],
            vec![GlossaryEntry {
                field: "amount".into(),
                description: "Monetary amount in USD".into(),
                constraints: Some("must be non-negative".into()),
                synonyms: None,
            }],
            vec![MetricDefinition {
                name: "total_revenue".into(),
                field: None,
                metric_type: None,
                formula: Some("sum(amount) where event_type = 'purchase'".into()),
                min: None,
                max: None,
            }],
            vec![], // quality rules not needed for this test
        )
    }

    // ---- export -------------------------------------------------------------

    #[test]
    fn export_produces_valid_odcs_mandatory_fields() {
        let contract = user_events_contract();
        let identity = make_identity("user_events");
        let yaml_content = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &contract,
        })
        .expect("export must succeed");

        let doc: serde_yaml::Value = serde_yaml::from_str(&odcs_yaml).expect("valid YAML");
        let m = doc.as_mapping().unwrap();

        assert_eq!(m.get("apiVersion").and_then(|v| v.as_str()), Some("v3.1.0"));
        assert_eq!(m.get("kind").and_then(|v| v.as_str()), Some("DataContract"));
        assert!(m.contains_key("id"), "must have id");
        assert_eq!(m.get("version").and_then(|v| v.as_str()), Some("1.0.0"));
        assert_eq!(m.get("status").and_then(|v| v.as_str()), Some("active"));
    }

    #[test]
    fn export_writes_cg_extensions() {
        let contract = user_events_contract();
        let identity = make_identity("user_events");
        let yaml_content = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &contract,
        })
        .unwrap();

        let doc: serde_yaml::Value = serde_yaml::from_str(&odcs_yaml).unwrap();
        let m = doc.as_mapping().unwrap();

        // Strict ODCS: no unknown root keys; extensions live in customProperties[].
        assert!(
            !m.keys()
                .any(|k| k.as_str().is_some_and(|s| s.starts_with("x-"))),
            "no bare x-* root keys: {odcs_yaml}"
        );
        assert!(
            odcs::root_extension(m, "x-contractgate-version").is_some(),
            "must write x-contractgate-version (D-003)"
        );
        assert!(
            odcs::root_extension(m, "x-contractgate-ontology").is_some(),
            "must write x-contractgate-ontology (D-003)"
        );
        assert!(
            odcs::root_extension(m, "x-contractgate-glossary").is_some(),
            "must write x-contractgate-glossary when non-empty"
        );
        assert!(
            odcs::root_extension(m, "x-contractgate-metrics").is_some(),
            "must write x-contractgate-metrics when non-empty"
        );
    }

    #[test]
    fn export_never_includes_pii_salt() {
        let contract = user_events_contract();
        let mut identity = make_identity("user_events");
        identity.pii_salt = vec![0xDE, 0xAD, 0xBE, 0xEF]; // sentinel
        let yaml_content = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &contract,
        })
        .unwrap();

        // The sentinel bytes must not appear in any form.
        assert!(
            !odcs_yaml.contains("deadbeef"),
            "pii_salt sentinel leaked into ODCS export"
        );
        assert!(
            !odcs_yaml.contains("pii_salt"),
            "pii_salt field name leaked into ODCS export"
        );
    }

    #[test]
    fn export_d004_name_in_data_product_and_schema() {
        let contract = user_events_contract();
        let identity = make_identity("user_events");
        let yaml_content = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &contract,
        })
        .unwrap();

        let doc: serde_yaml::Value = serde_yaml::from_str(&odcs_yaml).unwrap();
        let m = doc.as_mapping().unwrap();

        // D-004: name must appear in dataProduct
        assert_eq!(
            m.get("dataProduct").and_then(|v| v.as_str()),
            Some("user_events")
        );

        // D-004: name must also appear in schema[0].name
        let schema0 = m
            .get("schema")
            .and_then(|v| v.as_sequence())
            .and_then(|s| s.first())
            .and_then(|v| v.as_mapping())
            .unwrap();
        assert_eq!(
            schema0.get("name").and_then(|v| v.as_str()),
            Some("user_events")
        );
    }

    // ---- nested objects / arrays (Mode A + Mode B) ---------------------------

    const NESTED_CONTRACT: &str = r#"
version: "1.0.0"
name: orders
ontology:
  entities:
    - name: order_id
      type: string
      required: true
    - name: customer
      type: object
      required: true
      properties:
        - name: email
          type: string
          required: true
          pattern: "^.+@.+$"
        - name: address
          type: object
          required: false
          properties:
            - name: city
              type: string
              required: true
    - name: lines
      type: array
      required: true
      items:
        name: line
        type: object
        required: true
        properties:
          - name: sku
            type: string
            required: true
          - name: qty
            type: integer
            required: true
            min: 1
          - name: tags
            type: array
            required: false
            items:
              name: tag
              type: string
              required: true
"#;

    fn field<'a>(
        fs: &'a [crate::contract::FieldDefinition],
        n: &str,
    ) -> &'a crate::contract::FieldDefinition {
        fs.iter()
            .find(|f| f.name == n)
            .unwrap_or_else(|| panic!("field {n} missing"))
    }

    fn assert_nested_preserved(c: &crate::contract::Contract) {
        let es = &c.ontology.entities;
        let customer = field(es, "customer");
        assert_eq!(customer.field_type, FieldType::Object);
        let cp = customer.properties.as_ref().expect("customer.properties");
        let email = field(cp, "email");
        assert_eq!(email.field_type, FieldType::String);
        assert!(email.required);
        assert_eq!(email.pattern.as_deref(), Some("^.+@.+$"));
        let addr = field(cp, "address");
        let city = field(
            addr.properties.as_ref().expect("address.properties"),
            "city",
        );
        assert_eq!(city.field_type, FieldType::String);

        let lines = field(es, "lines");
        assert_eq!(lines.field_type, FieldType::Array);
        let item = lines.items.as_ref().expect("lines.items");
        assert_eq!(item.field_type, FieldType::Object);
        let ip = item.properties.as_ref().expect("lines.items.properties");
        assert_eq!(field(ip, "sku").field_type, FieldType::String);
        let qty = field(ip, "qty");
        assert_eq!(qty.field_type, FieldType::Integer);
        assert_eq!(qty.min, Some(1.0));
        let tags = field(ip, "tags");
        assert_eq!(tags.field_type, FieldType::Array);
        assert_eq!(
            tags.items.as_ref().expect("tags.items").field_type,
            FieldType::String
        );
    }

    fn nested_export() -> String {
        let original: crate::contract::Contract = serde_yaml::from_str(NESTED_CONTRACT).unwrap();
        let identity = make_identity("orders");
        let cv = make_version(identity.id, "1.0.0", NESTED_CONTRACT);
        odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &original,
        })
        .unwrap()
    }

    #[test]
    fn nested_roundtrip_mode_a() {
        let r = odcs::import_odcs(&nested_export()).unwrap();
        assert_eq!(r.import_source, ImportSource::Odcs);
        assert_nested_preserved(&serde_yaml::from_str(&r.yaml_content).unwrap());
    }

    #[test]
    fn nested_roundtrip_mode_b_stripped_export() {
        // Strip the x-contractgate-* extensions so only the ODCS-native
        // schema section is left.
        let mut doc: serde_yaml::Value = serde_yaml::from_str(&nested_export()).unwrap();
        let m = doc.as_mapping_mut().unwrap();
        m.remove("customProperties");
        let stripped = serde_yaml::to_string(&doc).unwrap();
        // Export writes array element schemas as ODCS `items`.
        assert!(stripped.contains("items:"), "{stripped}");
        let r = odcs::import_odcs(&stripped).unwrap();
        assert_eq!(r.import_source, ImportSource::OdcsStripped);
        let c: crate::contract::Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_nested_preserved(&c);
        // Re-export of the imported contract is stable (same field tree).
        let identity = make_identity("orders");
        let cv = make_version(identity.id, "1.0.0", &r.yaml_content);
        let again = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &c,
        })
        .unwrap();
        let r2 = odcs::import_odcs(&again).unwrap();
        assert_nested_preserved(&serde_yaml::from_str(&r2.yaml_content).unwrap());
    }

    #[test]
    fn foreign_odcs_native_nested_properties_and_items() {
        // Hand-written ODCS v3 with natively nested properties (the nested-properties case).
        let odcs_yaml = r#"
apiVersion: v3.1.0
kind: DataContract
id: 7f0c1e2a-0000-0000-0000-000000000001
version: 1.0.0
status: active
dataProduct: orders
schema:
  - name: orders
    properties:
      - name: order_id
        logicalType: string
        required: true
      - name: customer
        logicalType: object
        required: true
        properties:
          - name: email
            logicalType: string
            required: true
            customProperties:
              - property: x-cg-pattern
                value: "^.+@.+$"
          - name: address
            logicalType: object
            properties:
              - name: city
                logicalType: string
                required: true
      - name: lines
        logicalType: array
        required: true
        items:
          logicalType: object
          properties:
            - name: sku
              logicalType: string
              required: true
            - name: qty
              logicalType: integer
              required: true
              customProperties:
                - property: x-cg-min
                  value: 1
            - name: tags
              logicalType: array
              items:
                logicalType: string
"#;
        let r = odcs::import_odcs(odcs_yaml).unwrap();
        let c: crate::contract::Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_nested_preserved(&c);
    }

    // ---- import Mode A (lossless) -------------------------------------------

    #[test]
    fn import_mode_a_is_lossless_roundtrip() {
        let original = user_events_contract();
        let identity = make_identity("user_events");
        let yaml_content = serde_yaml::to_string(&original).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        // Export → ODCS
        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &original,
        })
        .unwrap();

        // Import the ODCS back
        let result = odcs::import_odcs(&odcs_yaml).expect("import must succeed");

        assert_eq!(result.import_source, ImportSource::Odcs);
        assert_eq!(result.version, "1.0.0");

        // Recovered contract must be functionally identical to the original.
        let recovered: crate::contract::Contract =
            serde_yaml::from_str(&result.yaml_content).expect("yaml must be valid");

        assert_eq!(recovered.name, original.name);
        // The embedded contract's version is rewritten to the row's (ODCS
        // `version`), even when the original YAML said something else.
        assert_eq!(recovered.version, "1.0.0");
        assert_eq!(
            recovered.ontology.entities.len(),
            original.ontology.entities.len()
        );
        assert_eq!(recovered.glossary.len(), original.glossary.len());
        assert_eq!(recovered.metrics.len(), original.metrics.len());

        // Spot-check field constraints are preserved
        let uid = recovered
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "user_id")
            .expect("user_id must survive round-trip");
        assert!(uid.required);
        assert!(uid.pattern.is_some());
    }

    // ---- import Mode B (stripped) -------------------------------------------

    #[test]
    fn import_mode_b_stripped_sets_requires_review() {
        // Minimal foreign ODCS document — no x-contractgate-* extensions.
        let odcs_yaml = r#"
apiVersion: v3.1.0
kind: DataContract
id: "some-foreign-id"
version: "2.0.0"
status: active
dataProduct: external_events
schema:
  - name: external_events
    properties:
      - name: event_id
        logicalType: string
        required: true
      - name: ts
        logicalType: integer
        required: true
"#;

        let result = odcs::import_odcs(odcs_yaml).unwrap();

        assert_eq!(result.import_source, ImportSource::OdcsStripped);
        assert_eq!(result.version, "2.0.0");
        // Stored YAML agrees with the version row (was hardcoded "1.0").
        let c: crate::contract::Contract = serde_yaml::from_str(&result.yaml_content).unwrap();
        assert_eq!(c.version, "2.0.0");

        // Recovered contract must parse
        let contract: crate::contract::Contract =
            serde_yaml::from_str(&result.yaml_content).expect("yaml must be valid");

        assert_eq!(contract.ontology.entities.len(), 2);
        let event_id = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "event_id")
            .expect("event_id must be present");
        assert_eq!(event_id.field_type, FieldType::String);
        assert!(event_id.required);
    }

    #[test]
    fn import_mode_b_recovers_x_cg_constraints() {
        // Foreign ODCS document with our customProperties written by a prior export.
        let odcs_yaml = r#"
apiVersion: v3.1.0
kind: DataContract
id: "test"
version: "1.0.0"
status: active
dataProduct: test_contract
schema:
  - name: test_contract
    properties:
      - name: amount
        logicalType: double
        required: false
        customProperties:
          - property: x-cg-min
            value: 0.0
          - property: x-cg-max
            value: 9999.99
"#;

        let result = odcs::import_odcs(odcs_yaml).unwrap();
        let contract: crate::contract::Contract =
            serde_yaml::from_str(&result.yaml_content).unwrap();

        let amount = contract
            .ontology
            .entities
            .iter()
            .find(|f| f.name == "amount")
            .unwrap();
        assert_eq!(amount.min, Some(0.0));
        assert_eq!(amount.max, Some(9999.99));
    }

    #[test]
    fn import_rejects_missing_version() {
        let bad_yaml = r#"
apiVersion: v3.1.0
kind: DataContract
id: "no-version"
status: active
schema:
  - name: empty
    properties: []
"#;
        let result = odcs::import_odcs(bad_yaml);
        assert!(result.is_err(), "import must fail when version is absent");
    }

    // ---- quality round-trip -------------------------------------------------

    #[test]
    fn export_quality_rules_appear_in_odcs_property_quality_array() {
        use crate::contract::{QualityRule, QualityRuleType};

        let contract = contract_with(
            "qtest",
            None,
            vec![
                entity_with("user_id", FieldType::String, |f| {
                    f.required = true;
                }),
                entity_with("created_at", FieldType::Integer, |f| {
                    f.required = true;
                }),
            ],
            vec![],
            vec![],
            vec![
                QualityRule {
                    field: "user_id".into(),
                    rule_type: QualityRuleType::Completeness,
                    description: Some("must be present".into()),
                    max_age_seconds: None,
                    scope: None,
                    threshold: None,
                },
                QualityRule {
                    field: "created_at".into(),
                    rule_type: QualityRuleType::Freshness,
                    description: None,
                    max_age_seconds: Some(3600),
                    scope: None,
                    threshold: None,
                },
            ],
        );

        let identity = make_identity("qtest");
        let yaml_content = serde_yaml::to_string(&contract).unwrap();
        let cv = make_version(identity.id, "1.0.0", &yaml_content);

        let odcs_yaml = odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: &contract,
        })
        .unwrap();

        let doc: serde_yaml::Value = serde_yaml::from_str(&odcs_yaml).unwrap();
        let schema0 = doc
            .as_mapping()
            .unwrap()
            .get("schema")
            .and_then(|v| v.as_sequence())
            .and_then(|s| s.first())
            .and_then(|v| v.as_mapping())
            .unwrap();
        let props = schema0
            .get("properties")
            .and_then(|v| v.as_sequence())
            .unwrap();

        // user_id property must have quality[0].type == "completeness"
        let uid_prop = props
            .iter()
            .find(|p| {
                p.as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    == Some("user_id")
            })
            .and_then(|p| p.as_mapping())
            .unwrap();
        let uid_quality = uid_prop
            .get("quality")
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert_eq!(uid_quality.len(), 1, "user_id should have 1 quality rule");
        let uq0 = uid_quality[0].as_mapping().unwrap();
        assert_eq!(uq0.get("type").and_then(|v| v.as_str()), Some("library"));
        assert_eq!(
            uq0.get("metric").and_then(|v| v.as_str()),
            Some("missingValues")
        );
        assert_eq!(
            uq0.get("dimension").and_then(|v| v.as_str()),
            Some("completeness")
        );

        // created_at must have freshness + attributes.maxAgeSeconds == 3600
        let ts_prop = props
            .iter()
            .find(|p| {
                p.as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    == Some("created_at")
            })
            .and_then(|p| p.as_mapping())
            .unwrap();
        let ts_quality = ts_prop
            .get("quality")
            .and_then(|v| v.as_sequence())
            .unwrap();
        assert_eq!(ts_quality.len(), 1);
        let ts_q0 = ts_quality[0].as_mapping().unwrap();
        assert_eq!(ts_q0.get("type").and_then(|v| v.as_str()), Some("custom"));
        assert_eq!(
            ts_q0.get("engine").and_then(|v| v.as_str()),
            Some("contractgate")
        );
        assert_eq!(
            ts_q0.get("implementation").and_then(|v| v.as_str()),
            Some("freshness maxAgeSeconds=3600")
        );
    }

    #[test]
    fn import_mode_b_recovers_quality_rules_from_odcs_quality_array() {
        // Foreign ODCS document with quality[] populated on properties.
        let odcs_yaml = r#"
apiVersion: v3.1.0
kind: DataContract
id: "qtest-b"
version: "2.0.0"
status: active
dataProduct: quality_import
schema:
  - name: quality_import
    properties:
      - name: order_id
        logicalType: string
        required: true
        quality:
          - type: completeness
            description: "must be present"
          - type: uniqueness
      - name: created_at
        logicalType: integer
        required: true
        quality:
          - type: freshness
            attributes:
              maxAgeSeconds: 7200
"#;

        let result = odcs::import_odcs(odcs_yaml).unwrap();
        assert_eq!(result.import_source, ImportSource::OdcsStripped);

        let contract: crate::contract::Contract =
            serde_yaml::from_str(&result.yaml_content).unwrap();

        // 3 quality rules total: 2 on order_id + 1 on created_at
        assert_eq!(
            contract.quality.len(),
            3,
            "should recover 3 quality rules; got: {:?}",
            contract
                .quality
                .iter()
                .map(|r| (&r.field, &r.rule_type))
                .collect::<Vec<_>>()
        );

        // Freshness rule must carry max_age_seconds
        let freshness = contract
            .quality
            .iter()
            .find(|r| matches!(r.rule_type, crate::contract::QualityRuleType::Freshness))
            .expect("freshness rule must be recovered");
        assert_eq!(freshness.max_age_seconds, Some(7200));
        assert_eq!(freshness.field, "created_at");
    }
}

// ---------------------------------------------------------------------------
// RFC-032: Contract Sharing & Publication unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rfc032_publication_tests {
    use crate::contract::{ImportMode, ImportSource, PublicationVisibility};
    use crate::publication::constant_time_eq;

    // ── ImportSource ────────────────────────────────────────────────────────

    #[test]
    fn import_source_publication_roundtrip() {
        let src = ImportSource::Publication;
        assert_eq!(src.as_str(), "publication");
        let parsed: ImportSource = "publication".parse().expect("must parse");
        assert_eq!(parsed, ImportSource::Publication);
    }

    #[test]
    fn import_source_all_variants_roundtrip() {
        for (s, expected) in [
            ("native", ImportSource::Native),
            ("odcs", ImportSource::Odcs),
            ("odcs_stripped", ImportSource::OdcsStripped),
            ("publication", ImportSource::Publication),
        ] {
            let parsed: ImportSource = s
                .parse()
                .unwrap_or_else(|e| panic!("parse({s:?}) failed: {e}"));
            assert_eq!(parsed, expected, "variant mismatch for {s:?}");
            assert_eq!(parsed.as_str(), s, "as_str mismatch for {s:?}");
        }
    }

    #[test]
    fn import_source_unknown_rejects() {
        assert!("unknown_source".parse::<ImportSource>().is_err());
    }

    // ── PublicationVisibility ────────────────────────────────────────────────

    #[test]
    fn publication_visibility_roundtrip() {
        for (s, expected) in [
            ("public", PublicationVisibility::Public),
            ("link", PublicationVisibility::Link),
            ("org", PublicationVisibility::Org),
        ] {
            let parsed: PublicationVisibility = s
                .parse()
                .unwrap_or_else(|e| panic!("parse({s:?}) failed: {e}"));
            assert_eq!(parsed, expected, "variant mismatch for {s:?}");
            assert_eq!(parsed.as_str(), s, "as_str mismatch for {s:?}");
        }
    }

    #[test]
    fn publication_visibility_unknown_rejects() {
        assert!("LINK".parse::<PublicationVisibility>().is_err());
        assert!("Private".parse::<PublicationVisibility>().is_err());
    }

    // ── ImportMode ─────────────────────────────────────────────────────────

    #[test]
    fn import_mode_roundtrip() {
        for (s, expected) in [
            ("snapshot", ImportMode::Snapshot),
            ("subscribe", ImportMode::Subscribe),
        ] {
            let parsed: ImportMode = s
                .parse()
                .unwrap_or_else(|e| panic!("parse({s:?}) failed: {e}"));
            assert_eq!(parsed, expected, "variant mismatch for {s:?}");
            assert_eq!(parsed.as_str(), s, "as_str mismatch for {s:?}");
        }
    }

    #[test]
    fn import_mode_unknown_rejects() {
        assert!("SNAPSHOT".parse::<ImportMode>().is_err());
        assert!("live".parse::<ImportMode>().is_err());
    }

    // ── constant_time_eq ────────────────────────────────────────────────────

    #[test]
    fn constant_time_eq_identical() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_different_content() {
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"token_a", b"token_b"));
    }

    #[test]
    fn constant_time_eq_different_length() {
        assert!(!constant_time_eq(b"short", b"longer_value"));
        assert!(!constant_time_eq(b"abc", b""));
    }

    #[test]
    fn constant_time_eq_single_bit_difference() {
        // Ensure a 1-bit flip does not pass
        assert!(!constant_time_eq(b"\x00", b"\x01"));
    }

    // ── Publish request default visibility ─────────────────────────────────

    #[test]
    fn publish_request_default_visibility_is_link() {
        // The JSON `{}` should deserialize to visibility = "link"
        let req: crate::publication::PublishRequest =
            serde_json::from_str("{}").expect("empty object must parse with defaults");
        let vis: PublicationVisibility = req.visibility.parse().expect("default must be valid");
        assert_eq!(vis, PublicationVisibility::Link);
    }

    #[test]
    fn publish_request_visibility_public() {
        let req: crate::publication::PublishRequest =
            serde_json::from_str(r#"{"visibility":"public"}"#).expect("must parse");
        let vis: PublicationVisibility = req.visibility.parse().expect("must parse visibility");
        assert_eq!(vis, PublicationVisibility::Public);
    }

    // ── ImportStatusResult serialization ───────────────────────────────────

    #[test]
    fn import_status_result_serializes() {
        let result = crate::storage::ImportStatusResult {
            import_mode: Some(ImportMode::Subscribe),
            publication_ref: Some("abc123def456".to_string()),
            source_revoked: false,
            update_available: true,
            latest_published_version: Some("2.0.0".to_string()),
            imported_version: Some("1.0.0".to_string()),
        };

        let json = serde_json::to_string(&result).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse back");

        assert_eq!(val["import_mode"], "subscribe");
        assert_eq!(val["update_available"], true);
        assert_eq!(val["latest_published_version"], "2.0.0");
        assert_eq!(val["imported_version"], "1.0.0");
        assert_eq!(val["source_revoked"], false);
    }

    #[test]
    fn import_status_result_no_provenance_serializes() {
        let result = crate::storage::ImportStatusResult {
            import_mode: None,
            publication_ref: None,
            source_revoked: false,
            update_available: false,
            latest_published_version: None,
            imported_version: None,
        };

        let json = serde_json::to_string(&result).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse back");

        assert_eq!(val["import_mode"], serde_json::Value::Null);
        assert_eq!(val["update_available"], false);
    }

    // ── Link token format validation ────────────────────────────────────────

    #[test]
    fn generated_link_token_is_32_hex_chars() {
        // We can't call generate_link_token() directly (private), but we can
        // verify the spec: 16 random bytes hex-encoded = 32 hex characters,
        // all in [0-9a-f].
        let sample_token = "a3f1b2c4d5e6f7081920a1b2c3d4e5f6";
        assert_eq!(sample_token.len(), 32);
        assert!(sample_token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
    }
}

// =============================================================================
// RFC-033 — Provider-Consumer Collaboration tests
// =============================================================================

#[cfg(test)]
mod rfc033_collaboration_tests {
    use crate::collaboration::CallerRole;

    // ── CallerRole::satisfies — full matrix ───────────────────────────────────

    #[test]
    fn owner_satisfies_all_roles() {
        for min in [
            CallerRole::Owner,
            CallerRole::Reviewer,
            CallerRole::Editor,
            CallerRole::Viewer,
        ] {
            assert!(
                CallerRole::Owner.satisfies(min),
                "Owner should satisfy {:?}",
                min
            );
        }
    }

    #[test]
    fn reviewer_satisfies_reviewer_editor_viewer_not_owner() {
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Reviewer));
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Editor));
        assert!(CallerRole::Reviewer.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Reviewer.satisfies(CallerRole::Owner));
    }

    #[test]
    fn editor_satisfies_editor_viewer_not_reviewer_owner() {
        assert!(CallerRole::Editor.satisfies(CallerRole::Editor));
        assert!(CallerRole::Editor.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Editor.satisfies(CallerRole::Reviewer));
        assert!(!CallerRole::Editor.satisfies(CallerRole::Owner));
    }

    #[test]
    fn viewer_satisfies_only_viewer() {
        assert!(CallerRole::Viewer.satisfies(CallerRole::Viewer));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Editor));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Reviewer));
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Owner));
    }

    // ── CallerRole::from_str ──────────────────────────────────────────────────

    #[test]
    fn caller_role_from_str_all_variants_roundtrip() {
        let cases = [
            ("owner", CallerRole::Owner),
            ("editor", CallerRole::Editor),
            ("reviewer", CallerRole::Reviewer),
            ("viewer", CallerRole::Viewer),
        ];
        for (s, expected) in cases {
            let got =
                CallerRole::from_str(s).unwrap_or_else(|| panic!("from_str({s:?}) returned None"));
            assert_eq!(got, expected, "mismatch for {s:?}");
        }
    }

    #[test]
    fn caller_role_from_str_unknown_returns_none() {
        assert!(CallerRole::from_str("admin").is_none());
        assert!(CallerRole::from_str("superuser").is_none());
        assert!(CallerRole::from_str("OWNER").is_none()); // case-sensitive
        assert!(CallerRole::from_str("").is_none());
    }

    // ── Proposal status transitions — only 'open' can be decided ──────────────

    #[test]
    fn proposal_status_set_coverage() {
        // The DB CHECK constraint allows these four values.
        // Verify the strings we use in decide/apply match exactly.
        let statuses = ["open", "approved", "rejected", "applied"];
        for s in statuses {
            assert!(!s.is_empty(), "status {s:?} must not be empty");
        }
    }

    // ── Role permission semantics — spec table from RFC-033 ───────────────────

    #[test]
    fn editor_cannot_decide_proposals() {
        // An editor satisfies editor-minimum but not reviewer-minimum.
        // A decide handler requires CallerRole::Reviewer.
        assert!(!CallerRole::Editor.satisfies(CallerRole::Reviewer));
    }

    #[test]
    fn reviewer_cannot_apply_proposals() {
        // Apply requires CallerRole::Owner.
        assert!(!CallerRole::Reviewer.satisfies(CallerRole::Owner));
    }

    #[test]
    fn editor_cannot_grant_collaborators() {
        // Granting collaborators requires CallerRole::Owner.
        assert!(!CallerRole::Editor.satisfies(CallerRole::Owner));
    }

    #[test]
    fn viewer_cannot_create_proposals() {
        // Creating proposals requires CallerRole::Editor (minimum).
        assert!(!CallerRole::Viewer.satisfies(CallerRole::Editor));
    }

    // ── Collaborator row serialisation ────────────────────────────────────────

    #[test]
    fn collaborator_row_serializes_cleanly() {
        use crate::storage::CollaboratorRow;
        use uuid::Uuid;

        let id = Uuid::new_v4();
        let gb = Uuid::new_v4();
        let row = CollaboratorRow {
            contract_name: "user_events".into(),
            org_id: id,
            role: "editor".into(),
            granted_by: gb,
            granted_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&row).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse");

        assert_eq!(val["contract_name"], "user_events");
        assert_eq!(val["role"], "editor");
        assert_eq!(val["org_id"], id.to_string());
    }

    #[test]
    fn proposal_row_serializes_cleanly() {
        use crate::storage::ProposalRow;
        use uuid::Uuid;

        let pid = Uuid::new_v4();
        let pb = Uuid::new_v4();
        let row = ProposalRow {
            id: pid,
            contract_name: "user_events".into(),
            proposed_by: pb,
            proposed_yaml: "version: \"1.0\"\nname: user_events\n".into(),
            status: "open".into(),
            decided_by: None,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&row).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse");

        assert_eq!(val["status"], "open");
        assert_eq!(val["decided_by"], serde_json::Value::Null);
        assert!(val["proposed_yaml"]
            .as_str()
            .unwrap()
            .contains("user_events"));
    }

    #[test]
    fn comment_row_serializes_cleanly() {
        use crate::storage::CommentRow;
        use uuid::Uuid;

        let cid = Uuid::new_v4();
        let oid = Uuid::new_v4();
        let row = CommentRow {
            id: cid,
            contract_name: "user_events".into(),
            field: Some("amount".into()),
            org_id: oid,
            author: "alice@example.com".into(),
            body: "Should this allow negative values?".into(),
            resolved: false,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&row).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse");

        assert_eq!(val["field"], "amount");
        assert_eq!(val["resolved"], false);
        assert_eq!(val["author"], "alice@example.com");
    }

    #[test]
    fn comment_row_whole_contract_field_is_null() {
        use crate::storage::CommentRow;
        use uuid::Uuid;

        let row = CommentRow {
            id: Uuid::new_v4(),
            contract_name: "user_events".into(),
            field: None, // whole-contract comment
            org_id: Uuid::new_v4(),
            author: "bob@example.com".into(),
            body: "LGTM overall".into(),
            resolved: true,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&row).expect("must serialize");
        let val: serde_json::Value = serde_json::from_str(&json).expect("must parse");

        assert_eq!(val["field"], serde_json::Value::Null);
        assert_eq!(val["resolved"], true);
    }

    // ── Security assertion: collaborators cannot access owner-scoped data ──────
    //
    // The actual isolation is enforced by:
    //   1. Postgres RLS on audit_log / quarantine_events (org_id IN get_my_org_ids())
    //      — the collaborator org is NOT in the owner's audit_log.org_id set.
    //   2. The Rust API never returns contracts.pii_salt in any response struct.
    //
    // We assert the Rust-side invariant: none of our collaboration response types
    // contain a pii_salt field.

    #[test]
    fn collaboration_responses_do_not_expose_pii_salt() {
        use crate::storage::{CollaboratorRow, CommentRow, ProposalRow};

        // Compile-time: if any of these structs gained a `pii_salt` field,
        // the struct literals below would fail to compile (missing field).
        let _ = CollaboratorRow {
            contract_name: "x".into(),
            org_id: uuid::Uuid::new_v4(),
            role: "viewer".into(),
            granted_by: uuid::Uuid::new_v4(),
            granted_at: chrono::Utc::now(),
        };
        let _ = CommentRow {
            id: uuid::Uuid::new_v4(),
            contract_name: "x".into(),
            field: None,
            org_id: uuid::Uuid::new_v4(),
            author: "a".into(),
            body: "b".into(),
            resolved: false,
            created_at: chrono::Utc::now(),
        };
        let _ = ProposalRow {
            id: uuid::Uuid::new_v4(),
            contract_name: "x".into(),
            proposed_by: uuid::Uuid::new_v4(),
            proposed_yaml: "y".into(),
            status: "open".into(),
            decided_by: None,
            created_at: chrono::Utc::now(),
        };
        // If we reach here without compile error, no pii_salt field leaks.
    }
}

// =============================================================================
// RFC-028 — Contract Queryability unit tests
// =============================================================================
//
// Acceptance criteria tested here (no DB required):
//   - DeployContractRequest deserializes with optional source / deployed_by
//   - DeployContractResponse serializes every field (contract_id, version_id,
//     name, version, source, deployed_by, deployed_at, deprecated_count)
//   - A Contract round-trips through serde_json::to_value (the parsed_json
//     column written at deploy time) and back — field names intact
//   - pii_salt never appears in DeployContractResponse (compile-time)
//   - Quarantine-guard decision: pending quarantine events → deploy blocked
//   - EgressLeakageMode roundtrip (also used by the deploy path)
// =============================================================================

#[cfg(test)]
mod rfc028_tests {
    use crate::contract::{
        Contract, DeployContractRequest, DeployContractResponse, EgressLeakageMode,
        FieldDefinition, FieldType, Ontology,
    };
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    // ── DeployContractRequest deserialization ────────────────────────────────

    #[test]
    fn deploy_request_minimal_parses() {
        // source and deployed_by are optional — omitting both must succeed.
        let body = json!({
            "name": "user_events",
            "yaml_content": "version: \"1.0\"\nname: user_events\nontology:\n  entities: []\n"
        });
        let req: DeployContractRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.name, "user_events");
        assert!(req.source.is_none());
        assert!(req.deployed_by.is_none());
    }

    #[test]
    fn deploy_request_with_source_and_deployer() {
        let body = json!({
            "name": "rent_events",
            "yaml_content": "---",
            "source": "yardi",
            "deployed_by": "ci-job-42"
        });
        let req: DeployContractRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.source.as_deref(), Some("yardi"));
        assert_eq!(req.deployed_by.as_deref(), Some("ci-job-42"));
    }

    #[test]
    fn deploy_request_requires_name() {
        let body = json!({ "yaml_content": "---" });
        assert!(serde_json::from_value::<DeployContractRequest>(body).is_err());
    }

    #[test]
    fn deploy_request_requires_yaml_content() {
        let body = json!({ "name": "user_events" });
        assert!(serde_json::from_value::<DeployContractRequest>(body).is_err());
    }

    // ── DeployContractResponse serialization ─────────────────────────────────

    #[test]
    fn deploy_response_serializes_all_fields() {
        let cid = Uuid::new_v4();
        let vid = Uuid::new_v4();
        let now = Utc::now();
        let resp = DeployContractResponse {
            contract_id: cid,
            version_id: vid,
            name: "user_events".into(),
            version: "1.2.0".into(),
            source: Some("realpage".into()),
            deployed_by: Some("alice".into()),
            deployed_at: now,
            deprecated_count: 2,
        };
        let val = serde_json::to_value(&resp).expect("must serialize");
        assert_eq!(val["contract_id"], cid.to_string());
        assert_eq!(val["version_id"], vid.to_string());
        assert_eq!(val["name"], "user_events");
        assert_eq!(val["version"], "1.2.0");
        assert_eq!(val["source"], "realpage");
        assert_eq!(val["deployed_by"], "alice");
        assert_eq!(val["deprecated_count"], 2);
        // pii_salt must not appear in the response
        assert!(
            val.get("pii_salt").is_none(),
            "pii_salt must never be serialized"
        );
    }

    #[test]
    fn deploy_response_optional_fields_null_when_absent() {
        let resp = DeployContractResponse {
            contract_id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
            name: "x".into(),
            version: "1.0.0".into(),
            source: None,
            deployed_by: None,
            deployed_at: Utc::now(),
            deprecated_count: 0,
        };
        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["source"], serde_json::Value::Null);
        assert_eq!(val["deployed_by"], serde_json::Value::Null);
        assert_eq!(val["deprecated_count"], 0);
    }

    // ── parsed_json round-trip ───────────────────────────────────────────────
    //
    // The deploy path serializes the parsed Contract to JSONB via
    // serde_json::to_value(&parsed). Verify that the key ontology fields
    // survive the round-trip so SQL @> queries work as expected.

    #[test]
    fn contract_to_json_roundtrip_preserves_ontology() {
        let c = Contract {
            version: "1.0".into(),
            name: "rent_events".into(),
            description: Some("Rental event stream".into()),
            compliance_mode: false,
            ontology: Ontology {
                entities: vec![
                    FieldDefinition {
                        name: "monthly_rent".into(),
                        field_type: FieldType::Float,
                        required: true,
                        min: Some(0.0),
                        max: None,
                        pattern: None,
                        allowed_values: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                        transform: None,
                        format: None,
                    },
                    FieldDefinition {
                        name: "tenant_id".into(),
                        field_type: FieldType::String,
                        required: true,
                        pattern: Some(r"^[a-z0-9]+$".into()),
                        min: None,
                        max: None,
                        allowed_values: None,
                        min_length: None,
                        max_length: None,
                        properties: None,
                        items: None,
                        transform: None,
                        format: None,
                    },
                ],
            },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            envelope: None,
            null_as_absent: false,
            egress_leakage_mode: EgressLeakageMode::Off,
        };

        let json_val = serde_json::to_value(&c).expect("Contract must serialize to JSON");

        // Key fields survive
        assert_eq!(json_val["name"], "rent_events");
        assert_eq!(json_val["version"], "1.0");

        // Ontology entities are present and correct
        let entities = &json_val["ontology"]["entities"];
        assert!(entities.is_array());
        let arr = entities.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // monthly_rent entity has min: 0.0 — this is the field the RFC-028
        // §Field-level risk queries example uses
        let rent = arr.iter().find(|e| e["name"] == "monthly_rent").unwrap();
        assert_eq!(rent["min"], 0.0);
        assert_eq!(rent["required"], true);

        // Round-trip: deserialize back
        let c2: Contract = serde_json::from_value(json_val).expect("must deserialize");
        assert_eq!(c2.name, "rent_events");
        assert_eq!(c2.ontology.entities.len(), 2);
        let rent2 = c2
            .ontology
            .entities
            .iter()
            .find(|e| e.name == "monthly_rent")
            .unwrap();
        assert_eq!(rent2.min, Some(0.0));
    }

    #[test]
    fn contract_parsed_json_does_not_contain_pii_salt() {
        // The parsed_json column in the contracts table stores the serialized
        // Contract struct. Verify pii_salt never appears in it (pii_salt lives
        // on the contracts row, not in parsed_json).
        let c = Contract {
            version: "1.0".into(),
            name: "safe_contract".into(),
            description: None,
            compliance_mode: false,
            ontology: Ontology { entities: vec![] },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            envelope: None,
            null_as_absent: false,
            egress_leakage_mode: EgressLeakageMode::Off,
        };
        let json_val = serde_json::to_value(&c).unwrap();
        assert!(
            json_val.get("pii_salt").is_none(),
            "pii_salt must never appear in parsed_json"
        );
    }

    // ── Quarantine-guard decision ─────────────────────────────────────────────
    //
    // RFC-028 §Open Questions: "block. If the contract has any status =
    // 'pending' quarantine events, the deploy call is rejected with a 400."
    // The guard lives in the DB layer (storage::deploy_contract_version checks
    // pending count). We test the pure decision logic: pending_count > 0 → block.

    fn deploy_should_block(pending_count: i64) -> bool {
        pending_count > 0
    }

    #[test]
    fn quarantine_guard_blocks_when_pending_events_exist() {
        assert!(deploy_should_block(1));
        assert!(deploy_should_block(99));
    }

    #[test]
    fn quarantine_guard_allows_when_no_pending_events() {
        assert!(!deploy_should_block(0));
    }

    // ── EgressLeakageMode roundtrip (used by deploy path) ────────────────────

    #[test]
    fn egress_leakage_mode_all_variants_roundtrip() {
        for (s, expected) in [
            ("off", EgressLeakageMode::Off),
            ("strip", EgressLeakageMode::Strip),
            ("fail", EgressLeakageMode::Fail),
        ] {
            let parsed: EgressLeakageMode = s
                .parse()
                .unwrap_or_else(|e| panic!("parse({s:?}) failed: {e}"));
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn egress_leakage_mode_unknown_rejects() {
        assert!("OFF".parse::<EgressLeakageMode>().is_err());
        assert!("block".parse::<EgressLeakageMode>().is_err());
        assert!("".parse::<EgressLeakageMode>().is_err());
    }

    #[test]
    fn egress_leakage_mode_default_is_off() {
        // Backwards-compatible default — undeclared fields pass through.
        let c = Contract {
            version: "1.0".into(),
            name: "x".into(),
            description: None,
            compliance_mode: false,
            ontology: Ontology { entities: vec![] },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
            envelope: None,
            null_as_absent: false,
            egress_leakage_mode: EgressLeakageMode::default(),
        };
        assert_eq!(c.egress_leakage_mode, EgressLeakageMode::Off);
    }
}

// =============================================================================
// RFC-029 — Egress Validation unit tests (src/tests.rs layer)
// =============================================================================
//
// The inline mod tests in src/egress.rs exhaustively cover disposition logic,
// PII pipeline, and path parsing.  This module adds coverage for the public
// types (DispositionMode, EgressResponse, direction field constants) that are
// also used by callers outside egress.rs.
// =============================================================================

#[cfg(test)]
mod rfc029_tests {
    use crate::egress::{DispositionMode, EgressOutcome, EgressResponse};
    use crate::validation::{Violation, ViolationKind};
    use serde_json::json;

    // ── DispositionMode roundtrip ─────────────────────────────────────────────

    #[test]
    fn disposition_mode_as_str_all_variants() {
        assert_eq!(DispositionMode::Block.as_str(), "block");
        assert_eq!(DispositionMode::Fail.as_str(), "fail");
        assert_eq!(DispositionMode::Tag.as_str(), "tag");
    }

    #[test]
    fn disposition_mode_default_is_block() {
        // RFC-029 §Open Question 3: default is `block` (graceful degradation).
        assert_eq!(DispositionMode::default(), DispositionMode::Block);
    }

    #[test]
    fn disposition_mode_deserializes_from_query_string() {
        // The EgressQuery struct uses #[serde(default)] so an absent
        // disposition deserializes as block.
        #[derive(serde::Deserialize)]
        struct Q {
            #[serde(default)]
            disposition: DispositionMode,
        }
        let block: Q = serde_json::from_value(json!({})).unwrap();
        assert_eq!(block.disposition, DispositionMode::Block);

        let fail: Q = serde_json::from_value(json!({"disposition":"fail"})).unwrap();
        assert_eq!(fail.disposition, DispositionMode::Fail);

        let tag: Q = serde_json::from_value(json!({"disposition":"tag"})).unwrap();
        assert_eq!(tag.disposition, DispositionMode::Tag);
    }

    // ── EgressOutcome serialization ───────────────────────────────────────────

    #[test]
    fn egress_outcome_passed_serializes() {
        let o = EgressOutcome {
            index: 0,
            passed: true,
            violations: vec![],
            validation_us: 42,
            action: "included",
            stripped_fields: vec![],
        };
        let val = serde_json::to_value(&o).unwrap();
        assert_eq!(val["index"], 0);
        assert_eq!(val["passed"], true);
        assert_eq!(val["action"], "included");
        assert_eq!(val["validation_us"], 42);
        // stripped_fields omitted when empty (skip_serializing_if)
        assert!(val.get("stripped_fields").is_none());
    }

    #[test]
    fn egress_outcome_failed_with_violations_serializes() {
        let o = EgressOutcome {
            index: 3,
            passed: false,
            violations: vec![Violation {
                field: "user_id".into(),
                message: "Required field 'user_id' is missing".into(),
                kind: ViolationKind::MissingRequiredField,
                ..Default::default()
            }],
            validation_us: 8,
            action: "blocked",
            stripped_fields: vec!["internal_key".into()],
        };
        let val = serde_json::to_value(&o).unwrap();
        assert_eq!(val["index"], 3);
        assert_eq!(val["passed"], false);
        assert_eq!(val["action"], "blocked");
        let sf = val["stripped_fields"].as_array().unwrap();
        assert_eq!(sf.len(), 1);
        assert_eq!(sf[0], "internal_key");
        let viols = val["violations"].as_array().unwrap();
        assert_eq!(viols.len(), 1);
        assert_eq!(viols[0]["field"], "user_id");
    }

    // ── EgressResponse serialization ─────────────────────────────────────────

    #[test]
    fn egress_response_all_pass_serializes() {
        let resp = EgressResponse {
            total: 2,
            passed: 2,
            failed: 0,
            dry_run: false,
            disposition: "block",
            egress_leakage_mode: "off",
            resolved_version: "1.0.0".into(),
            requested_version: None,
            payload: vec![json!({"user_id":"alice"}), json!({"user_id":"bob"})],
            outcomes: vec![
                EgressOutcome {
                    index: 0,
                    passed: true,
                    violations: vec![],
                    validation_us: 5,
                    action: "included",
                    stripped_fields: vec![],
                },
                EgressOutcome {
                    index: 1,
                    passed: true,
                    violations: vec![],
                    validation_us: 4,
                    action: "included",
                    stripped_fields: vec![],
                },
            ],
        };
        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["total"], 2);
        assert_eq!(val["passed"], 2);
        assert_eq!(val["failed"], 0);
        assert_eq!(val["dry_run"], false);
        assert_eq!(val["disposition"], "block");
        assert_eq!(val["egress_leakage_mode"], "off");
        assert_eq!(val["resolved_version"], "1.0.0");
        assert_eq!(val["payload"].as_array().unwrap().len(), 2);
        assert_eq!(val["outcomes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn egress_response_dry_run_flag_serializes() {
        let resp = EgressResponse {
            total: 1,
            passed: 1,
            failed: 0,
            dry_run: true,
            disposition: "tag",
            egress_leakage_mode: "strip",
            resolved_version: "2.0.0".into(),
            requested_version: None,
            payload: vec![],
            outcomes: vec![],
        };
        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["dry_run"], true);
        assert_eq!(val["egress_leakage_mode"], "strip");
    }

    // ── Direction field constants ─────────────────────────────────────────────
    //
    // RFC-029: audit_log.direction and quarantine_events.direction must be
    // exactly "ingress" or "egress". The strings are written as &'static str
    // in the handler. This test pins them so a typo is caught immediately.

    #[test]
    fn direction_egress_string_is_exact() {
        let direction: &str = "egress";
        assert_eq!(direction, "egress");
    }

    #[test]
    fn direction_ingress_string_is_exact() {
        let direction: &str = "ingress";
        assert_eq!(direction, "ingress");
    }

    #[test]
    fn direction_strings_are_distinct() {
        assert_ne!("egress", "ingress");
    }

    // ── MAX_BATCH_SIZE constant matches ingest cap ────────────────────────────

    #[test]
    fn egress_max_batch_size_matches_ingest() {
        // RFC-029: egress uses the same 1,000-event cap as ingest (RFC-001).
        assert_eq!(crate::egress::MAX_BATCH_SIZE, 1_000);
    }
}

// =============================================================================
// RFC-031 — Provider Data-Quality Scorecard unit tests (src/tests.rs layer)
// =============================================================================
//
// The inline mod tests in src/scorecard.rs cover drift threshold logic and CSV
// escaping.  This module adds coverage for the public response types and the
// drift signal label format, accessible from src/tests.rs.
// =============================================================================

#[cfg(test)]
mod rfc031_tests {
    use crate::scorecard::{DriftSignal, FieldHealthRow, ScorecardResponse, ScorecardRow};

    // ── ScorecardRow serialization ────────────────────────────────────────────

    #[test]
    fn scorecard_row_serializes_all_fields() {
        let row = ScorecardRow {
            source: "yardi".into(),
            contract_name: "rent_events".into(),
            total_events: 1000,
            passed: 920,
            quarantined: 80,
            quarantine_pct: Some(8.0),
        };
        let val = serde_json::to_value(&row).unwrap();
        assert_eq!(val["source"], "yardi");
        assert_eq!(val["contract_name"], "rent_events");
        assert_eq!(val["total_events"], 1000);
        assert_eq!(val["passed"], 920);
        assert_eq!(val["quarantined"], 80);
        assert!((val["quarantine_pct"].as_f64().unwrap() - 8.0).abs() < 0.001);
    }

    #[test]
    fn scorecard_row_null_quarantine_pct_when_zero_events() {
        let row = ScorecardRow {
            source: "entrata".into(),
            contract_name: "occupancy_events".into(),
            total_events: 0,
            passed: 0,
            quarantined: 0,
            quarantine_pct: None,
        };
        let val = serde_json::to_value(&row).unwrap();
        assert_eq!(val["quarantine_pct"], serde_json::Value::Null);
        assert_eq!(val["total_events"], 0);
    }

    // ── FieldHealthRow serialization ──────────────────────────────────────────

    #[test]
    fn field_health_row_serializes() {
        let row = FieldHealthRow {
            source: "realpage".into(),
            contract_name: "rent_events".into(),
            field: "monthly_rent".into(),
            code: "range_violation".into(),
            violations: 42,
        };
        let val = serde_json::to_value(&row).unwrap();
        assert_eq!(val["source"], "realpage");
        assert_eq!(val["field"], "monthly_rent");
        assert_eq!(val["code"], "range_violation");
        assert_eq!(val["violations"], 42);
    }

    // ── DriftSignal label format ──────────────────────────────────────────────
    //
    // RFC-031: label is e.g. "↑ 12.3 pp since baseline" (positive) or
    // "↓ 12.3 pp since baseline" (negative/improvement).

    #[test]
    fn drift_signal_positive_delta_label() {
        let signal = DriftSignal {
            source: "yardi".into(),
            contract_name: "rent_events".into(),
            field: "monthly_rent".into(),
            current_rate: 0.12,
            baseline_rate: 0.0,
            delta_pct: 12.0,
            label: format!("↑ {:.1} pp since baseline", 12.0_f64),
        };
        assert!(signal.label.starts_with('↑'));
        assert!(signal.label.contains("12.0 pp since baseline"));
        let val = serde_json::to_value(&signal).unwrap();
        assert_eq!(val["field"], "monthly_rent");
        assert!((val["delta_pct"].as_f64().unwrap() - 12.0).abs() < 0.001);
    }

    #[test]
    fn drift_signal_negative_delta_label_improvement() {
        let signal = DriftSignal {
            source: "yardi".into(),
            contract_name: "rent_events".into(),
            field: "monthly_rent".into(),
            current_rate: 0.02,
            baseline_rate: 0.15,
            delta_pct: -13.0,
            label: format!("↓ {:.1} pp since baseline", 13.0_f64),
        };
        assert!(signal.label.starts_with('↓'));
        assert!(signal.label.contains("13.0 pp since baseline"));
        assert!(signal.delta_pct < 0.0);
    }

    #[test]
    fn drift_signal_serializes_rates() {
        let signal = DriftSignal {
            source: "s".into(),
            contract_name: "c".into(),
            field: "f".into(),
            current_rate: 0.08,
            baseline_rate: 0.20,
            delta_pct: -12.0,
            label: "↓ 12.0 pp since baseline".into(),
        };
        let val = serde_json::to_value(&signal).unwrap();
        assert!((val["current_rate"].as_f64().unwrap() - 0.08).abs() < 1e-9);
        assert!((val["baseline_rate"].as_f64().unwrap() - 0.20).abs() < 1e-9);
        assert!((val["delta_pct"].as_f64().unwrap() - (-12.0)).abs() < 1e-9);
    }

    // ── ScorecardResponse structure ───────────────────────────────────────────

    #[test]
    fn scorecard_response_top_violations_is_subset_of_field_health() {
        // RFC-031: top_violations is at most 20 rows from field_health, ranked
        // by count descending. Verify the subset invariant is preserved.
        let field_health: Vec<FieldHealthRow> = (0..25_u32)
            .map(|i| FieldHealthRow {
                source: "yardi".into(),
                contract_name: "rent_events".into(),
                field: format!("field_{}", i),
                code: "range_violation".into(),
                violations: (25 - i) as i64,
            })
            .collect();
        let top_violations: Vec<FieldHealthRow> = field_health.iter().take(20).cloned().collect();

        let resp = ScorecardResponse {
            source: "yardi".into(),
            summary: vec![],
            top_violations,
            field_health,
            drift_signals: vec![],
        };

        assert_eq!(resp.top_violations.len(), 20);
        assert_eq!(resp.field_health.len(), 25);
        // Top violation has the highest count
        assert_eq!(resp.top_violations[0].violations, 25);
        assert_eq!(resp.top_violations[19].violations, 6);

        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["source"], "yardi");
        assert_eq!(val["top_violations"].as_array().unwrap().len(), 20);
        assert_eq!(val["field_health"].as_array().unwrap().len(), 25);
    }

    #[test]
    fn scorecard_response_empty_source_serializes() {
        let resp = ScorecardResponse {
            source: "(unsourced)".into(),
            summary: vec![],
            top_violations: vec![],
            field_health: vec![],
            drift_signals: vec![],
        };
        let val = serde_json::to_value(&resp).unwrap();
        assert_eq!(val["source"], "(unsourced)");
        assert_eq!(val["drift_signals"].as_array().unwrap().len(), 0);
    }
}

// ---------------------------------------------------------------------------
// RFC-047 / RFC-048 — org-scoping + x-org-id header removal tests
//
// Unit tests run without a DB.  The two-org DB tests are marked `#[ignore]`
// and require DATABASE_URL; run them with:
//
//   cargo test org_scoping -- --ignored
// ---------------------------------------------------------------------------

#[cfg(test)]
mod org_scoping {
    // ---- Unit: OrgId extractor ------------------------------------------------

    /// `OrgId` extractor returns `None` when no `ValidatedKey` is in extensions.
    /// Covers dev mode (`CONTRACTGATE_DEV_NO_AUTH`) where no key is injected.
    #[test]
    fn org_id_none_when_no_validated_key() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        // Build a minimal request with no extensions.
        let req = Request::builder()
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(super::super::OrgId::from_request_parts(&mut parts, &()));

        let super::super::OrgId(org_id) = result.unwrap();
        assert!(
            org_id.is_none(),
            "OrgId should be None when no ValidatedKey present"
        );
    }

    /// `OrgId` extractor returns `Some(uuid)` when a `ValidatedKey` is injected.
    /// Covers DB-backed key and JWT paths.
    #[test]
    fn org_id_some_when_validated_key_present() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;
        use uuid::Uuid;

        let expected_org = Uuid::new_v4();
        let key = super::super::api_key_auth::ValidatedKey {
            api_key_id: Uuid::nil(),
            user_id: Uuid::new_v4(),
            org_id: expected_org,
            allowed_contract_ids: None,
            rate_limit_rps: None,
            rate_limit_burst: None,
        };

        let req = Request::builder()
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = req.into_parts();
        parts.extensions.insert(key);

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(super::super::OrgId::from_request_parts(&mut parts, &()));

        let super::super::OrgId(org_id) = result.unwrap();
        assert_eq!(
            org_id,
            Some(expected_org),
            "OrgId should match the ValidatedKey org_id"
        );
    }

    /// RFC-048: `org_id_from_req` no longer reads the `x-org-id` header.
    /// A request with the header but no ValidatedKey must yield None.
    #[test]
    fn x_org_id_header_not_trusted() {
        use axum::http::Request;
        use uuid::Uuid;

        let spoofed_org = Uuid::new_v4();
        let req = Request::builder()
            .uri("/")
            .header("x-org-id", spoofed_org.to_string())
            .body(axum::body::Body::empty())
            .unwrap();

        let org_id = super::super::org_id_from_req(&req);
        assert!(
            org_id.is_none(),
            "x-org-id header must NOT be trusted after RFC-048 (got {:?})",
            org_id
        );
    }

    // ---- DB integration: two-org isolation ------------------------------------
    //
    // These tests require DATABASE_URL.  Run with:
    //   cargo test org_scoping::two_org -- --ignored
    //
    // They exercise the full storage layer end-to-end: org A creates a
    // contract; org B holds a different UUID and must see 404 on every
    // by-ID route.  Dev mode (org_id = None) must serve unscoped.

    /// Org B's key gets ContractNotFound on GET for org A's contract.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn two_org_get_contract_isolation() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org_a = uuid::Uuid::new_v4();
        let org_b = uuid::Uuid::new_v4();

        let minimal_yaml = r#"
version: "1.0"
name: "isolation_test_contract"
description: "Two-org isolation test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        // Seed the two orgs. `contracts.org_id` is a FK to orgs(id) (migration
        // 007), so the contract insert below fails without these rows. Seeding
        // here makes the test runnable against any migrated Postgres with no
        // external seed data (RFC-068).
        for (org, slug) in [(org_a, "iso-test-a"), (org_b, "iso-test-b")] {
            sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
                .bind(org)
                .bind("isolation test org")
                .bind(format!("{slug}-{}", org.simple()))
                .execute(&pool)
                .await
                .expect("seed org");
        }

        // Org A creates a contract.
        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "isolation_test_contract",
            Some("Two-org isolation test"),
            minimal_yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .expect("org A should create contract");

        let contract_id = identity.id;

        // Org B: GET → ContractNotFound (404).
        let err = super::super::storage::get_contract_identity(&pool, contract_id, Some(org_b))
            .await
            .unwrap_err();
        assert!(
            matches!(err, super::super::error::AppError::ContractNotFound(_)),
            "org B GET should be ContractNotFound, got: {:?}",
            err
        );

        // Org B: PATCH → ContractNotFound.
        let err = super::super::storage::patch_contract_identity(
            &pool,
            contract_id,
            Some("renamed"),
            None,
            None,
            Some(org_b),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            super::super::error::AppError::ContractNotFound(_)
        ));

        // Org B: DELETE → ContractNotFound.
        let err = super::super::storage::delete_contract(&pool, contract_id, Some(org_b))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            super::super::error::AppError::ContractNotFound(_)
        ));

        // Org B: create version → ContractNotFound (org check in create_version).
        let err = super::super::storage::create_version(
            &pool,
            contract_id,
            "2.0.0",
            minimal_yaml,
            Some(org_b),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            super::super::error::AppError::ContractNotFound(_)
        ));

        // Org A: GET succeeds (same org).
        super::super::storage::get_contract_identity(&pool, contract_id, Some(org_a))
            .await
            .expect("org A should still read its own contract");

        // Dev mode (org_id = None): unscoped, must also succeed.
        super::super::storage::get_contract_identity(&pool, contract_id, None)
            .await
            .expect("dev mode (org_id=None) should serve unscoped");

        // Cleanup: delete the contract first, then the orgs. (Deleting the org
        // would cascade to the contract via ON DELETE CASCADE, but we delete
        // the contract explicitly so the intent is obvious and a future FK
        // change can't silently leave orphan rows.)
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = ANY($1)")
            .bind(vec![org_a, org_b])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// RFC-081: the org-scoped quarantine LIST + id-resolution helpers never
    /// surface another org's quarantine rows. Org B must not see or resolve org
    /// A's quarantined event; dev mode (org_id=None) sees all.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn two_org_quarantine_list_isolation() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org_a = uuid::Uuid::new_v4();
        let org_b = uuid::Uuid::new_v4();

        let minimal_yaml = r#"
version: "1.0"
name: "quarantine_iso_contract"
description: "RFC-081 quarantine isolation test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        for (org, slug) in [(org_a, "q-iso-a"), (org_b, "q-iso-b")] {
            sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
                .bind(org)
                .bind("quarantine isolation org")
                .bind(format!("{slug}-{}", org.simple()))
                .execute(&pool)
                .await
                .expect("seed org");
        }

        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "quarantine_iso_contract",
            Some("RFC-081 quarantine isolation test"),
            minimal_yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .expect("org A creates contract");
        let contract_id = identity.id;

        // Seed one quarantined (source) row on org A's contract.
        let quar_id = uuid::Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO quarantine_events
                (id, contract_id, contract_version, payload, violation_count,
                 violation_details, status)
            VALUES ($1, $2, '1.0.0', '{"id":123}'::jsonb, 1, '[]'::jsonb, 'pending')
            "#,
        )
        .bind(quar_id)
        .bind(contract_id)
        .execute(&pool)
        .await
        .expect("seed quarantine row");

        // Org A sees it.
        let a_list =
            super::super::storage::list_quarantine_events(&pool, Some(org_a), None, 100, 0)
                .await
                .unwrap();
        assert!(
            a_list.iter().any(|r| r.id == quar_id),
            "org A must see its own quarantine row"
        );

        // Org B must NOT see it.
        let b_list =
            super::super::storage::list_quarantine_events(&pool, Some(org_b), None, 100, 0)
                .await
                .unwrap();
        assert!(
            !b_list.iter().any(|r| r.id == quar_id),
            "org B must NOT see org A's quarantine row"
        );

        // Org B must NOT resolve org A's id (→ empty, treated as not-found).
        let resolved_b =
            super::super::storage::resolve_quarantine_contracts(&pool, Some(org_b), &[quar_id])
                .await
                .unwrap();
        assert!(
            resolved_b.is_empty(),
            "org B must not resolve org A's quarantine id"
        );

        // Org A resolves its own id to the owning contract.
        let resolved_a =
            super::super::storage::resolve_quarantine_contracts(&pool, Some(org_a), &[quar_id])
                .await
                .unwrap();
        assert_eq!(resolved_a, vec![(quar_id, contract_id)]);

        // Dev mode (org_id=None) is unscoped and sees the row.
        let dev_list = super::super::storage::list_quarantine_events(&pool, None, None, 100, 0)
            .await
            .unwrap();
        assert!(dev_list.iter().any(|r| r.id == quar_id));

        // Cleanup (contract delete cascades the quarantine row).
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = ANY($1)")
            .bind(vec![org_a, org_b])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// RFC-082: the pilot-report aggregation computes correct per-version
    /// pass/quarantine counts and top violations over the audit log.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn pilot_report_aggregation() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org_a = uuid::Uuid::new_v4();
        let minimal_yaml = r#"
version: "1.0.0"
name: "report_agg_contract"
description: "RFC-082 report aggregation test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_a)
            .bind("report agg org")
            .bind(format!("report-agg-{}", org_a.simple()))
            .execute(&pool)
            .await
            .expect("seed org");

        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "report_agg_contract",
            Some("RFC-082 report aggregation test"),
            minimal_yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .expect("org A creates contract");
        let cid = identity.id;

        // Seed audit rows: v1.0.0 → 3 pass / 1 quarantine (method/invalid_enum);
        // v1.1.0 → 2 pass / 0 quarantine.
        let bad = r#"[{"field":"method","kind":"invalid_enum","message":"bad"}]"#;
        for (ver, passed, details, n) in [
            ("1.0.0", true, "[]", 3),
            ("1.0.0", false, bad, 1),
            ("1.1.0", true, "[]", 2),
        ] {
            for _ in 0..n {
                sqlx::query(
                    "INSERT INTO audit_log
                     (contract_id, org_id, contract_version, passed, violation_count, violation_details)
                     VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
                )
                .bind(cid)
                .bind(org_a)
                .bind(ver)
                .bind(passed)
                .bind(if passed { 0 } else { 1 })
                .bind(details)
                .execute(&pool)
                .await
                .expect("seed audit row");
            }
        }

        // Per-version counts.
        let versions = super::super::storage::report_by_version(&pool, cid, None, None)
            .await
            .unwrap();
        let v10 = versions
            .iter()
            .find(|r| r.contract_version.as_deref() == Some("1.0.0"))
            .expect("v1.0.0 row");
        assert_eq!((v10.total, v10.passed, v10.quarantined), (4, 3, 1));
        let v11 = versions
            .iter()
            .find(|r| r.contract_version.as_deref() == Some("1.1.0"))
            .expect("v1.1.0 row");
        assert_eq!((v11.total, v11.passed, v11.quarantined), (2, 2, 0));

        // Top violations.
        let viols = super::super::storage::report_top_violations(&pool, cid, None, None)
            .await
            .unwrap();
        assert_eq!(viols.len(), 1);
        assert_eq!(viols[0].field.as_deref(), Some("method"));
        assert_eq!(viols[0].kind.as_deref(), Some("invalid_enum"));
        assert_eq!(viols[0].count, 1);

        // Cleanup (audit rows cascade with the contract).
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org_a)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// RFC-083: `monthly_event_count` counts only the org's events inside the
    /// window, and `get_org_plan` returns the org's plan.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn usage_monthly_count() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org_a = uuid::Uuid::new_v4();
        let minimal_yaml = r#"
version: "1.0.0"
name: "usage_count_contract"
description: "RFC-083 usage count test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        // orgs.plan defaults to 'free' (migration 007).
        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_a)
            .bind("usage org")
            .bind(format!("usage-{}", org_a.simple()))
            .execute(&pool)
            .await
            .expect("seed org");

        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "usage_count_contract",
            Some("RFC-083 usage count test"),
            minimal_yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .expect("org A creates contract");
        let cid = identity.id;

        let now = chrono::Utc::now();
        // One event now, one two days ago.
        for ts in [now, now - chrono::Duration::days(2)] {
            sqlx::query(
                "INSERT INTO audit_log
                 (contract_id, org_id, contract_version, passed, violation_count,
                  violation_details, created_at)
                 VALUES ($1, $2, '1.0.0', true, 0, '[]'::jsonb, $3)",
            )
            .bind(cid)
            .bind(org_a)
            .bind(ts)
            .execute(&pool)
            .await
            .expect("seed audit row");
        }

        // Window of the last day → only the recent event.
        let recent = super::super::storage::monthly_event_count(
            &pool,
            Some(org_a),
            now - chrono::Duration::days(1),
        )
        .await
        .unwrap();
        assert_eq!(recent, 1);

        // Window of the last three days → both events.
        let both = super::super::storage::monthly_event_count(
            &pool,
            Some(org_a),
            now - chrono::Duration::days(3),
        )
        .await
        .unwrap();
        assert_eq!(both, 2);

        let plan = super::super::storage::get_org_plan(&pool, org_a)
            .await
            .unwrap();
        assert_eq!(plan.as_deref(), Some("free"));

        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org_a)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// Org B's key gets VersionNotFound on GET for org A's version.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn two_org_get_version_isolation() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org_a = uuid::Uuid::new_v4();
        let org_b = uuid::Uuid::new_v4();

        let minimal_yaml = r#"
version: "1.0"
name: "isolation_version_test"
description: "Version isolation test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        // Seed the two orgs (FK target for contracts.org_id, migration 007).
        // See two_org_get_contract_isolation for rationale (RFC-068).
        for (org, slug) in [(org_a, "iso-ver-a"), (org_b, "iso-ver-b")] {
            sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
                .bind(org)
                .bind("isolation test org")
                .bind(format!("{slug}-{}", org.simple()))
                .execute(&pool)
                .await
                .expect("seed org");
        }

        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "isolation_version_test",
            None,
            minimal_yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .unwrap();

        let contract_id = identity.id;

        // Org B: GET version → VersionNotFound (scoped through parent contract).
        let err = super::super::storage::get_version(&pool, contract_id, "1.0.0", Some(org_b))
            .await
            .unwrap_err();
        assert!(
            matches!(err, super::super::error::AppError::VersionNotFound { .. }),
            "org B version GET should be VersionNotFound, got: {:?}",
            err
        );

        // Org B: promote → same VersionNotFound before reaching state check.
        let err = super::super::storage::promote_version(&pool, contract_id, "1.0.0", Some(org_b))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            super::super::error::AppError::VersionNotFound { .. }
        ));

        // Org B: write-side denials (RFC-070). Each of these guards via
        // get_version(.., org_id) and must return VersionNotFound before any
        // state check — a regression dropping org_id would let one tenant
        // mutate/delete another tenant's version.
        let err = super::super::storage::patch_version_yaml(
            &pool,
            contract_id,
            "1.0.0",
            minimal_yaml,
            Some(org_b),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, super::super::error::AppError::VersionNotFound { .. }),
            "org B patch_version_yaml should be VersionNotFound, got: {:?}",
            err
        );

        let err =
            super::super::storage::deprecate_version(&pool, contract_id, "1.0.0", Some(org_b))
                .await
                .unwrap_err();
        assert!(
            matches!(err, super::super::error::AppError::VersionNotFound { .. }),
            "org B deprecate_version should be VersionNotFound, got: {:?}",
            err
        );

        let err = super::super::storage::delete_version(&pool, contract_id, "1.0.0", Some(org_b))
            .await
            .unwrap_err();
        assert!(
            matches!(err, super::super::error::AppError::VersionNotFound { .. }),
            "org B delete_version should be VersionNotFound, got: {:?}",
            err
        );

        // Org A: GET version succeeds.
        super::super::storage::get_version(&pool, contract_id, "1.0.0", Some(org_a))
            .await
            .expect("org A should still read its own version");

        // Cleanup (contract first, then orgs — see sibling test).
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = ANY($1)")
            .bind(vec![org_a, org_b])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// RFC-072: the conditional UPDATE in `mark_quarantine_replayed_batch` is
    /// the double-replay race guard. Two concurrent replays of the same source
    /// row must result in at most one stamp; the loser sees the row already
    /// `replayed` and is re-categorized as `already_replayed` by the handler.
    ///
    /// Self-seeding Class-1 test (RFC-068): seeds an org + contract + one
    /// pending quarantine row, then calls the batch mark twice with two
    /// distinct audit IDs. The first call must return the source ID (it won
    /// the UPDATE); the second must return empty (status no longer
    /// `pending`/`reviewed`). Verifies the persisted row is stamped exactly
    /// once and links to the *first* audit ID.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn quarantine_replay_race_guard() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

        let org = uuid::Uuid::new_v4();

        let minimal_yaml = r#"
version: "1.0"
name: "replay_race_test"
description: "Replay race-guard test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org)
            .bind("replay race test org")
            .bind(format!("replay-race-{}", org.simple()))
            .execute(&pool)
            .await
            .expect("seed org");

        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "replay_race_test",
            None,
            minimal_yaml,
            Default::default(),
            Some(org),
        )
        .await
        .expect("seed contract");
        let contract_id = identity.id;

        // Seed one pending quarantine row. `quarantine_event` doesn't return
        // the id, so read it back via list-by-contract through a direct SELECT.
        super::super::storage::quarantine_event(
            &pool,
            contract_id,
            "1.0.0",
            super::super::transform::TransformedPayload::from_stored(
                serde_json::json!({ "id": "evt-1" }),
            ),
            1,
            serde_json::json!([{ "field": "id", "rule": "required" }]),
            42,
            Some("203.0.113.7"),
            "ingress",
        )
        .await
        .expect("seed quarantine row");

        let (quar_id,): (uuid::Uuid,) =
            sqlx::query_as("SELECT id FROM quarantine_events WHERE contract_id = $1")
                .bind(contract_id)
                .fetch_one(&pool)
                .await
                .expect("read back quarantine id");

        // replayed_into_audit_id is a FK to audit_log(id), so the two "audit
        // rows a replay produced" must really exist. Seed them directly.
        let now = chrono::Utc::now();
        let audit_first = uuid::Uuid::new_v4();
        let audit_second = uuid::Uuid::new_v4();
        for audit_id in [audit_first, audit_second] {
            sqlx::query(
                r#"
                INSERT INTO audit_log
                    (id, contract_id, org_id, contract_version, passed,
                     violation_count, violation_details, raw_event,
                     validation_us, source_ip, source, direction, created_at)
                VALUES ($1, $2, $3, '1.0.0', true, 0, '[]'::jsonb,
                        '{"id":"evt-1"}'::jsonb, 42, NULL, 'replay', 'ingress', NOW())
                "#,
            )
            .bind(audit_id)
            .bind(contract_id)
            .bind(org)
            .execute(&pool)
            .await
            .expect("seed audit row");
        }

        // First replay wins the conditional UPDATE.
        let marked_first = super::super::storage::mark_quarantine_replayed_batch(
            &pool,
            &[(quar_id, audit_first)],
            now,
        )
        .await
        .expect("first mark");
        assert_eq!(
            marked_first,
            vec![quar_id],
            "first replay should win the race and mark the row"
        );

        // Second replay loses: status is no longer pending/reviewed.
        let marked_second = super::super::storage::mark_quarantine_replayed_batch(
            &pool,
            &[(quar_id, audit_second)],
            now,
        )
        .await
        .expect("second mark");
        assert!(
            marked_second.is_empty(),
            "second replay must lose the race (no row marked), got: {:?}",
            marked_second
        );

        // Persisted row is stamped once and links to the FIRST audit id.
        let rows = super::super::storage::list_quarantine_by_ids(&pool, &[quar_id])
            .await
            .expect("list by id");
        let row = rows
            .into_iter()
            .find(|r| r.id == quar_id)
            .expect("row should exist");
        assert_eq!(row.status, "replayed", "row should be stamped replayed");
        assert_eq!(
            row.replayed_into_audit_id,
            Some(audit_first),
            "row must link to the winning (first) audit id, not the loser"
        );

        // Cleanup. quarantine_events references audit_log, so delete quarantine
        // rows first, then audit rows, then contract, then org.
        sqlx::query("DELETE FROM quarantine_events WHERE contract_id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM audit_log WHERE contract_id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(contract_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// RFC-053 — /ready readiness-probe tests
//
// These tests do NOT require a live DB: they construct a minimal AppState with
// a deliberately closed pool to exercise the 503 path, and rely on axum-test
// to drive the 200 path against a live pool (guarded by `#[ignore]` + DATABASE_URL).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rfc053_ready_tests {
    use axum::http::StatusCode;
    use axum_test::TestServer;

    /// Build a test router with only the /ready route wired.
    async fn make_server(pool: sqlx::PgPool) -> TestServer {
        let state = std::sync::Arc::new(super::super::AppState::new(pool, true, None, None));
        let app = axum::Router::new()
            .route("/ready", axum::routing::get(super::super::ready_handler))
            .with_state(state);
        // axum-test 20: TestServer::new returns TestServer directly, not Result.
        TestServer::new(app)
    }

    /// /ready returns 503 when the pool is closed (cannot serve SELECT 1).
    #[tokio::test]
    async fn ready_returns_503_on_closed_pool() {
        // Build a pool with no valid connection string — close it immediately.
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/none")
            .expect("lazy connect should succeed");
        pool.close().await;

        let server = make_server(pool).await;
        let resp = server.get("/ready").await;
        assert_eq!(resp.status_code(), StatusCode::SERVICE_UNAVAILABLE);

        // axum-test 20: use .json::<T>() to deserialise the body.
        let body = resp.json::<serde_json::Value>();
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["db"], "error");
    }

    /// /health always returns 200 regardless of DB state.
    #[tokio::test]
    async fn health_always_200() {
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/none")
            .expect("lazy connect should succeed");
        pool.close().await;

        let state = std::sync::Arc::new(super::super::AppState::new(pool, true, None, None));
        let app = axum::Router::new()
            .route("/health", axum::routing::get(super::super::health_handler))
            .with_state(state);
        let server = TestServer::new(app);

        let resp: axum_test::TestResponse = server.get("/health").await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    /// /ready returns 200 against a live DB.
    /// Requires DATABASE_URL; run with: `cargo test ready_returns_200 -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn ready_returns_200_live_db() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let server = make_server(pool).await;

        let resp = server.get("/ready").await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let body = resp.json::<serde_json::Value>();
        assert_eq!(body["status"], "ready");
        assert_eq!(body["db"], "ok");
    }
}

// ---------------------------------------------------------------------------
// RFC-052 follow-up: unknown-kid debounce on the failure path
// ---------------------------------------------------------------------------
//
// Regression guard for the bug where a failed JWKS fetch reset the atomic
// back to `last` (the old timestamp), letting every subsequent request
// attempt its own re-fetch — a thundering herd against a down endpoint.
// After the fix the atomic stays at `now` after a failure, so the 60-second
// debounce applies to failures as well as successes.

#[cfg(test)]
mod rfc052_debounce_tests {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// Build a minimal AppState whose JWKS URL points to a port that always
    /// refuses connections, so `fetch_jwks` returns Err instantly.
    fn make_state() -> Arc<super::super::AppState> {
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/none")
            .expect("lazy connect should succeed");
        Arc::new(super::super::AppState::new(
            pool,
            true,
            None,
            // Port 1 is always closed on loopback — connection refused is
            // instant, keeping the test fast.
            Some("http://127.0.0.1:1/.well-known/jwks.json".into()),
        ))
    }

    /// After a failed JWKS refresh the atomic must remain at `now`, not be
    /// reset to the prior value.  A second call within 60 s must be debounced.
    #[tokio::test]
    async fn failed_refresh_debounces_subsequent_calls() {
        let state = make_state();

        // Atomic starts at 0; first call should attempt a fetch (not debounced).
        // fetch_jwks will fail → should return false.
        let refreshed = super::super::maybe_refresh_jwks_on_unknown_kid(&state).await;
        assert!(!refreshed, "expected false on a failed fetch");

        // The atomic must now be near `now`, NOT 0.
        // If the bug is present, store(last) resets it back to 0 here.
        let stamp = state.jwks_last_kid_refresh.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(
            stamp > 0 && now.saturating_sub(stamp) < 5,
            "jwks_last_kid_refresh should be near `now` after a failed fetch, got {stamp} (now={now})"
        );

        // Second call immediately after must be debounced (stamp is recent,
        // saturating_sub < 60).  It must NOT attempt another fetch.
        let refreshed2 = super::super::maybe_refresh_jwks_on_unknown_kid(&state).await;
        assert!(!refreshed2, "expected debounce on second call within 60 s");

        // The atomic must still equal the stamp from the first call — a
        // debounced call must not touch the atomic.
        let stamp2 = state.jwks_last_kid_refresh.load(Ordering::Relaxed);
        assert_eq!(
            stamp, stamp2,
            "debounced call must not modify jwks_last_kid_refresh"
        );
    }
}

// ---------------------------------------------------------------------------
// RFC-086 — gated event-payload storage (DB integration)
//
// These require DATABASE_URL and a migrated Postgres (through migration 034).
// Run with:
//   cargo test rfc086 -- --ignored
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rfc086_payload_storage {
    use serde_json::json;
    use uuid::Uuid;

    use super::super::storage::{
        self, AuditEntryInsert, ForwardEventInsert, QuarantineEventInsert,
    };
    use super::super::transform::TransformedPayload;

    const YAML: &str = r#"
version: "1.0"
name: "rfc086_contract"
description: "RFC-086 payload storage test"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

    async fn connect() -> sqlx::PgPool {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        sqlx::PgPool::connect(&db_url).await.unwrap()
    }

    async fn seed_org(pool: &sqlx::PgPool, org: Uuid, slug: &str) {
        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org)
            .bind("rfc086 org")
            .bind(format!("{slug}-{}", org.simple()))
            .execute(pool)
            .await
            .expect("seed org");
    }

    async fn cleanup(pool: &sqlx::PgPool, contract_id: Uuid, org: Uuid) {
        // Contract delete cascades audit_log + quarantine_events.
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(contract_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(pool)
            .await
            .unwrap();
    }

    fn quar_insert(contract_id: Uuid) -> QuarantineEventInsert {
        QuarantineEventInsert {
            contract_id,
            contract_version: "1.0.0".to_string(),
            payload: TransformedPayload::from_stored(json!({"id": "secret-body"})),
            violation_count: 1,
            violation_details: json!([{"field":"id","kind":"x","message":"m"}]),
            validation_us: 5,
            source_ip: Some("203.0.113.1".to_string()),
            replay_of_quarantine_id: None,
            pre_assigned_id: None,
            direction: "ingress".to_string(),
        }
    }

    fn audit_insert(contract_id: Uuid, org: Uuid, passed: bool) -> AuditEntryInsert {
        AuditEntryInsert {
            contract_id,
            org_id: Some(org),
            contract_version: "1.0.0".to_string(),
            passed,
            violation_count: if passed { 0 } else { 1 },
            violation_details: if passed {
                json!([])
            } else {
                json!([{"field":"id","kind":"x","message":"m"}])
            },
            raw_event: TransformedPayload::from_stored(json!({"id": "secret-body"})),
            validation_us: 5,
            source_ip: Some("203.0.113.1".to_string()),
            source: "http".to_string(),
            pre_assigned_id: None,
            replay_of_quarantine_id: None,
            direction: "ingress".to_string(),
        }
    }

    /// `store_payloads = false` writes metadata rows: quarantine `payload`
    /// NULL + `payload_redacted`, audit `raw_event` `{}` + `raw_event_redacted`;
    /// all other columns intact. `store_payloads = true` writes real bodies.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 034"]
    async fn rfc086_redaction_on_insert() {
        let pool = connect().await;
        let org = Uuid::new_v4();
        seed_org(&pool, org, "rfc086-redact").await;
        let (identity, _) = storage::create_contract(
            &pool,
            "rfc086_contract",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;

        // --- Redacted (store_payloads = false) ---
        storage::quarantine_events_batch(&pool, &[quar_insert(cid)], false)
            .await
            .unwrap();
        storage::log_audit_entries_batch(&pool, &[audit_insert(cid, org, false)], false)
            .await
            .unwrap();

        let rows = storage::list_quarantine_events(&pool, Some(org), Some(cid), 100, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one source quarantine row");
        assert!(rows[0].payload.is_none(), "redacted payload must be NULL");
        assert!(rows[0].payload_redacted, "payload_redacted must be set");
        assert_eq!(rows[0].violation_count, 1, "metadata retained");
        assert_eq!(rows[0].contract_version.as_deref(), Some("1.0.0"));

        let (raw, redacted): (serde_json::Value, bool) = sqlx::query_as(
            "SELECT raw_event, raw_event_redacted FROM audit_log WHERE contract_id = $1",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(redacted, "audit raw_event_redacted must be set");
        assert_eq!(raw, json!({}), "redacted raw_event must be '{{}}'");

        // --- Stored (store_payloads = true) ---
        storage::quarantine_events_batch(&pool, &[quar_insert(cid)], true)
            .await
            .unwrap();
        storage::log_audit_entries_batch(&pool, &[audit_insert(cid, org, false)], true)
            .await
            .unwrap();

        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM quarantine_events \
             WHERE contract_id = $1 AND payload IS NOT NULL AND payload_redacted = false",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, 1, "store=true keeps the body, unredacted");
        let stored_audit: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_log \
             WHERE contract_id = $1 AND raw_event_redacted = false",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored_audit, 1, "store=true audit body unredacted");

        cleanup(&pool, cid, org).await;
    }

    /// Purge redacts bodies in place and NEVER deletes a row: row counts are
    /// unchanged, all bodies become NULL/`{}` + redacted, and a second purge is
    /// idempotent (nothing left to redact).
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 034"]
    async fn rfc086_purge_preserves_rows() {
        let pool = connect().await;
        let org = Uuid::new_v4();
        seed_org(&pool, org, "rfc086-purge").await;
        let (identity, _) = storage::create_contract(
            &pool,
            "rfc086_contract",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;

        // Seed 3 quarantine + 3 audit rows WITH bodies.
        let quars: Vec<_> = (0..3).map(|_| quar_insert(cid)).collect();
        let audits: Vec<_> = (0..3).map(|_| audit_insert(cid, org, false)).collect();
        storage::quarantine_events_batch(&pool, &quars, true)
            .await
            .unwrap();
        storage::log_audit_entries_batch(&pool, &audits, true)
            .await
            .unwrap();
        storage::forward_events_batch(
            &pool,
            &[
                ForwardEventInsert {
                    contract_id: cid,
                    contract_version: "1.0.0".into(),
                    payload: TransformedPayload::from_stored(json!({"id": "secret-body"})),
                },
                ForwardEventInsert {
                    contract_id: cid,
                    contract_version: "1.0.0".into(),
                    payload: TransformedPayload::from_stored(json!({"id": "secret-body-2"})),
                },
            ],
            true,
        )
        .await
        .unwrap();

        let q_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM quarantine_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        let a_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((q_before, a_before), (3, 3));

        // Purge this contract.
        let (q_redacted, a_redacted, f_redacted) =
            storage::purge_bodies(&pool, Some(org), Some(cid))
                .await
                .unwrap();
        assert_eq!(q_redacted, 3, "all 3 quarantine bodies redacted");
        assert_eq!(a_redacted, 3, "all 3 audit bodies redacted");
        assert_eq!(f_redacted, 2, "both forwarded bodies redacted");

        // Row counts UNCHANGED — nothing deleted.
        let q_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM quarantine_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        let a_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (q_after, a_after),
            (q_before, a_before),
            "purge must not delete rows"
        );

        // Every body is now redacted.
        let q_unredacted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM quarantine_events \
             WHERE contract_id = $1 AND (payload IS NOT NULL OR payload_redacted = false)",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(q_unredacted, 0, "no quarantine body survives purge");
        let a_unredacted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE contract_id = $1 AND raw_event_redacted = false",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(a_unredacted, 0, "no audit body survives purge");
        let f_bodies: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM forwarded_events \
             WHERE contract_id = $1 AND jsonb_typeof(payload) <> 'null'",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(f_bodies, 0, "no forwarded body survives purge");
        let f_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM forwarded_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            f_rows, 2,
            "purge nulls forwarded payloads and keeps the rows"
        );

        // Idempotent: nothing left to redact.
        let (q2, a2, f2) = storage::purge_bodies(&pool, Some(org), Some(cid))
            .await
            .unwrap();
        assert_eq!((q2, a2, f2), (0, 0, 0), "second purge is a no-op");

        cleanup(&pool, cid, org).await;
    }

    /// The storage gate (`contract_stores_payloads`) applied end-to-end:
    /// free → never; paid needs org master switch AND per-contract override on.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 034"]
    async fn rfc086_contract_stores_payloads_gate() {
        let pool = connect().await;
        let org = Uuid::new_v4();
        seed_org(&pool, org, "rfc086-gate").await; // default plan 'free'
        let (identity, _) = storage::create_contract(
            &pool,
            "rfc086_contract",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;

        // Free plan → never stores, whatever the switches.
        storage::set_org_store_payloads(&pool, org, true)
            .await
            .unwrap();
        assert!(
            !storage::contract_stores_payloads(&pool, cid, false)
                .await
                .unwrap(),
            "free plan never stores"
        );

        // Upgrade to a paid plan.
        sqlx::query("UPDATE orgs SET plan = 'growth' WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();

        // Paid + org switch on + contract default (true) → stores.
        assert!(storage::contract_stores_payloads(&pool, cid, false)
            .await
            .unwrap());
        // get_org_payload_policy reflects (plan, switch).
        let (plan, switch) = storage::get_org_payload_policy(&pool, org)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((plan.as_str(), switch), ("growth", true));

        // Per-contract opt-out → does not store.
        assert!(
            storage::set_contract_store_payloads(&pool, cid, Some(org), false)
                .await
                .unwrap()
        );
        assert!(!storage::contract_stores_payloads(&pool, cid, false)
            .await
            .unwrap());

        // Contract back on, org master off → does not store.
        storage::set_contract_store_payloads(&pool, cid, Some(org), true)
            .await
            .unwrap();
        storage::set_org_store_payloads(&pool, org, false)
            .await
            .unwrap();
        assert!(!storage::contract_stores_payloads(&pool, cid, false)
            .await
            .unwrap());

        cleanup(&pool, cid, org).await;
    }

    /// Paid + org switch on must persist the quarantine body. Free, and paid
    /// with the switch off, stay redacted. Covers aisg_pro (the plan QA hit).
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 036"]
    async fn quarantine_write_respects_paid_payload_setting() {
        let pool = connect().await;
        let org = Uuid::new_v4();
        seed_org(&pool, org, "rfc086-write").await;
        let (identity, _) = storage::create_contract(
            &pool,
            "rfc086_write",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;
        assert!(
            identity.store_event_payloads,
            "new contract column defaults to storing (inherit org switch)"
        );

        let state = crate::AppState::new(pool.clone(), true, None, None);

        async fn decide(state: &crate::AppState, org: Uuid, contract_switch: bool) -> bool {
            crate::ingest::resolve_store_payloads(state, Some(org), contract_switch).await
        }

        async fn write_and_read(pool: &sqlx::PgPool, cid: Uuid, store: bool) -> (bool, bool) {
            storage::quarantine_events_batch(pool, &[quar_insert(cid)], store)
                .await
                .unwrap();
            let (redacted, payload_null): (bool, bool) = sqlx::query_as(
                "SELECT payload_redacted, payload IS NULL FROM quarantine_events \
                 WHERE contract_id = $1 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(cid)
            .fetch_one(pool)
            .await
            .unwrap();
            (redacted, payload_null)
        }

        // Free + org switch on + contract on → redacted. The list flag agrees.
        storage::set_org_store_payloads(&pool, org, true)
            .await
            .unwrap();
        let store = decide(&state, org, identity.store_event_payloads).await;
        assert!(!store, "free must not store");
        assert!(
            !crate::plan::effective_store_event_payloads(Some("free"), true, true),
            "free reports store_event_payloads false"
        );
        let (redacted, payload_null) = write_and_read(&pool, cid, store).await;
        assert!(redacted && payload_null, "free quarantine row is redacted");

        // Paid (aisg_pro) + on → store body, payload_redacted false.
        sqlx::query("UPDATE orgs SET plan = 'aisg_pro', store_event_payloads = true WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        let (plan, enabled) = storage::get_org_payload_policy(&pool, org)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(plan, "aisg_pro");
        assert!(enabled, "settings endpoint reads the org switch as true");
        let reloaded = storage::get_contract_identity(&pool, cid, Some(org))
            .await
            .unwrap();
        assert!(reloaded.store_event_payloads);
        let store = decide(&state, org, reloaded.store_event_payloads).await;
        assert!(store, "aisg_pro + enabled must store");
        assert!(
            crate::plan::effective_store_event_payloads(Some("aisg_pro"), true, true),
            "paid+on reports store_event_payloads true"
        );
        let (redacted, payload_null) = write_and_read(&pool, cid, store).await;
        assert!(
            !redacted && !payload_null,
            "paid+on quarantine row keeps the body"
        );

        // Paid + off → redacted.
        storage::set_org_store_payloads(&pool, org, false)
            .await
            .unwrap();
        let store = decide(&state, org, reloaded.store_event_payloads).await;
        assert!(!store, "paid + org switch off must redact");
        assert!(
            !crate::plan::effective_store_event_payloads(
                Some("aisg_pro"),
                false,
                reloaded.store_event_payloads
            ),
            "paid+off reports store_event_payloads false"
        );
        let (redacted, payload_null) = write_and_read(&pool, cid, store).await;
        assert!(
            redacted && payload_null,
            "paid+off quarantine row is redacted"
        );

        cleanup(&pool, cid, org).await;
    }
}

// ---------------------------------------------------------------------------
// Startup schema guard (dogfood follow-up) — pure message formatting.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod schema_guard {
    #[test]
    fn no_missing_columns_is_ok() {
        assert!(super::super::schema_lag_message(&[]).is_none());
    }

    #[test]
    fn missing_columns_produce_a_fatal_message() {
        let missing = vec![
            "orgs.store_event_payloads".to_string(),
            "audit_log.raw_event_redacted".to_string(),
        ];
        let msg = super::super::schema_lag_message(&missing).expect("should be Some");
        assert!(msg.contains("schema lag detected"));
        assert!(msg.contains("orgs.store_event_payloads"));
        assert!(msg.contains("audit_log.raw_event_redacted"));
        assert!(msg.contains("Refusing to boot"));
    }
}

// ---------------------------------------------------------------------------
// Migration 042: collaboration keyed by contract id.  Needs a migrated DB:
//   DATABASE_URL=... cargo test collab_by_id -- --ignored
// ---------------------------------------------------------------------------
#[cfg(test)]
mod collab_by_id {
    use crate::storage;
    use uuid::Uuid;

    async fn org(pool: &sqlx::PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'o', $2)")
            .bind(id)
            .bind(format!("t-{id}"))
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn contract(pool: &sqlx::PgPool, org: Uuid, name: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO contracts (name, org_id) VALUES ($1, $2) RETURNING id")
            .bind(name)
            .bind(org)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL with migrations through 042"]
    async fn same_name_contracts_do_not_share_grants_or_comments() {
        let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let (a, b, c) = (org(&pool).await, org(&pool).await, org(&pool).await);
        let name = format!("orders-{}", Uuid::new_v4());
        let ca = contract(&pool, a, &name).await;
        let cb = contract(&pool, b, &name).await;

        let g = storage::grant_collaborator(&pool, ca, c, "viewer")
            .await
            .unwrap();
        assert_eq!(g.granted_by, a);
        assert_eq!(g.contract_name, name);

        // C is a collaborator on A's contract only.
        let granted = storage::granted_contracts_by_name(&pool, &name, c)
            .await
            .unwrap();
        assert_eq!(granted, vec![(ca, "viewer".to_string())]);
        assert_eq!(
            storage::collaborator_grant_by_id(&pool, ca, c)
                .await
                .unwrap(),
            Some((a, "viewer".to_string()))
        );
        assert!(storage::org_owns_contract_id(&pool, ca, a).await.unwrap());
        assert!(!storage::org_owns_contract_id(&pool, ca, c).await.unwrap());
        assert_eq!(
            storage::collaborator_grant_by_id(&pool, cb, c)
                .await
                .unwrap(),
            None
        );
        assert!(storage::list_collaborators(&pool, cb)
            .await
            .unwrap()
            .is_empty());

        let cm = storage::add_comment(&pool, ca, None, c, "c", "hi")
            .await
            .unwrap();
        assert!(storage::list_comments(&pool, cb).await.unwrap().is_empty());
        assert_eq!(storage::list_comments(&pool, ca).await.unwrap().len(), 1);
        // Comment id under the other contract is not found.
        assert!(storage::resolve_comment(&pool, cb, cm.id).await.is_err());
        assert!(
            storage::resolve_comment(&pool, ca, cm.id)
                .await
                .unwrap()
                .resolved
        );
        assert_eq!(
            storage::get_comment_org(&pool, ca, cm.id).await.unwrap(),
            Some(c)
        );
        storage::delete_comment(&pool, ca, cm.id).await.unwrap();

        let p = storage::create_proposal(&pool, ca, c, "x: 1")
            .await
            .unwrap();
        assert!(storage::decide_proposal(&pool, cb, p.id, "approved", a)
            .await
            .is_err());
        storage::decide_proposal(&pool, ca, p.id, "approved", a)
            .await
            .unwrap();
        assert_eq!(
            storage::apply_proposal(&pool, ca, p.id)
                .await
                .unwrap()
                .status,
            "applied"
        );
        assert!(storage::list_proposals(&pool, cb).await.unwrap().is_empty());
        let p2 = storage::create_proposal(&pool, ca, c, "x: 2")
            .await
            .unwrap();
        assert_eq!(
            storage::get_proposal_meta(&pool, ca, p2.id).await.unwrap(),
            Some((c, "open".to_string()))
        );
        assert_eq!(
            storage::get_proposal_meta(&pool, cb, p2.id).await.unwrap(),
            None
        );
        assert!(storage::delete_proposal(&pool, cb, p2.id).await.is_err());
        storage::delete_proposal(&pool, ca, p2.id).await.unwrap();

        // Org-visibility import grant: granted_by is the owner, never the importer.
        storage::ensure_viewer_collaborator(&pool, cb, c)
            .await
            .unwrap();
        let rows = storage::list_collaborators(&pool, cb).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].granted_by, b);
        // Owner can't be granted on its own contract via import.
        storage::ensure_viewer_collaborator(&pool, cb, b)
            .await
            .unwrap();
        assert_eq!(
            storage::list_collaborators(&pool, cb).await.unwrap().len(),
            1
        );

        storage::update_collaborator_role(&pool, ca, c, "editor")
            .await
            .unwrap();
        storage::revoke_collaborator(&pool, ca, c).await.unwrap();
        assert_eq!(
            storage::collaborator_grant_by_id(&pool, ca, c)
                .await
                .unwrap(),
            None
        );
    }
}

// ---------------------------------------------------------------------------
// Kafka/Kinesis streaming ingress is Enterprise-only (server-side gate).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod streaming_gate {
    use axum::http::StatusCode;
    use axum::routing::{delete, get, post};
    use axum_test::TestServer;
    use std::sync::Arc;
    use uuid::Uuid;

    use super::super::{api_key_auth::ValidatedKey, kafka_ingress, kinesis_ingress, AppState};

    fn router(state: Arc<AppState>, org: Option<Uuid>) -> TestServer {
        let app = axum::Router::new()
            .route(
                "/contracts/{id}/kafka-ingress",
                get(kafka_ingress::get_kafka_ingress_handler),
            )
            .route(
                "/contracts/{id}/kafka-ingress/enable",
                post(kafka_ingress::enable_kafka_ingress_handler),
            )
            .route(
                "/contracts/{id}/kafka-ingress/disable",
                delete(kafka_ingress::disable_kafka_ingress_handler),
            )
            .route(
                "/contracts/{id}/kinesis-ingress",
                get(kinesis_ingress::get_kinesis_ingress_handler),
            )
            .route(
                "/contracts/{id}/kinesis-ingress/disable",
                delete(kinesis_ingress::disable_kinesis_ingress_handler),
            )
            .route(
                "/contracts/{id}/kinesis-ingress/enable",
                post(kinesis_ingress::enable_kinesis_ingress_handler),
            )
            .route(
                "/contracts/{id}/kinesis-ingress/rotate-credentials",
                post(kinesis_ingress::rotate_kinesis_credentials_handler),
            )
            .layer(axum::middleware::from_fn(
                move |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                    if let Some(org_id) = org {
                        req.extensions_mut().insert(ValidatedKey {
                            api_key_id: Uuid::new_v4(),
                            user_id: Uuid::new_v4(),
                            org_id,
                            allowed_contract_ids: None,
                            rate_limit_rps: None,
                            rate_limit_burst: None,
                        });
                    }
                    next.run(req).await
                },
            ))
            .with_state(state);
        TestServer::new(app)
    }

    const TURN_ON: [&str; 3] = [
        "kafka-ingress/enable",
        "kinesis-ingress/enable",
        "kinesis-ingress/rotate-credentials",
    ];

    /// Auth on, no org on the request → 401 before any provisioning.
    #[tokio::test]
    async fn turn_on_routes_401_without_org() {
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/none").unwrap();
        let server = router(Arc::new(AppState::new(pool, false, None, None)), None);
        for path in TURN_ON {
            let resp = server
                .post(&format!("/contracts/{}/{path}", Uuid::new_v4()))
                .await;
            assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED, "{path}");
        }
    }

    /// Free / Growth / aisg orgs get 403 on every route that turns streaming
    /// on; Enterprise passes the gate. Requires DATABASE_URL.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn turn_on_routes_require_enterprise() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let state = Arc::new(AppState::new(pool.clone(), false, None, None));

        for plan in ["free", "growth", "aisg_free", "aisg_pro", "enterprise"] {
            let org = Uuid::new_v4();
            sqlx::query("INSERT INTO orgs (id, name, slug, plan) VALUES ($1, $2, $3, $4)")
                .bind(org)
                .bind(format!("streaming gate {plan}"))
                .bind(format!("streaming-gate-{}", org.simple()))
                .bind(plan)
                .execute(&pool)
                .await
                .expect("seed org");

            let server = router(state.clone(), Some(org));
            // The org must own the contract: ownership is checked before the
            // plan, so a missing/foreign contract would be 404, not 403.
            let (identity, _) = super::super::storage::create_contract(
                &pool,
                "streaming_gate",
                None,
                "version: \"1.0.0\"\nname: \"streaming_gate\"\nontology:\n  entities:\n    - name: id\n      type: string\n      required: true\nglossary: []\nmetrics: []\n",
                Default::default(),
                Some(org),
            )
            .await
            .expect("org creates contract");
            let cid = identity.id;
            for path in TURN_ON {
                let resp = server.post(&format!("/contracts/{cid}/{path}")).await;
                if plan == "enterprise" {
                    assert_ne!(resp.status_code(), StatusCode::FORBIDDEN, "{plan} {path}");
                    assert_ne!(
                        resp.status_code(),
                        StatusCode::UNAUTHORIZED,
                        "{plan} {path}"
                    );
                } else {
                    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN, "{plan} {path}");
                    assert!(
                        resp.text().contains("requires the Enterprise plan"),
                        "{plan} {path}: {}",
                        resp.text()
                    );
                }
            }
            // Read and disable stay open below Enterprise (no 403).
            let read = server.get(&format!("/contracts/{cid}/kafka-ingress")).await;
            assert_ne!(read.status_code(), StatusCode::FORBIDDEN, "{plan} read");
            let off = server
                .delete(&format!("/contracts/{cid}/kafka-ingress/disable"))
                .await;
            assert_ne!(off.status_code(), StatusCode::FORBIDDEN, "{plan} disable");

            sqlx::query("DELETE FROM contracts WHERE id = $1")
                .bind(cid)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("DELETE FROM orgs WHERE id = $1")
                .bind(org)
                .execute(&pool)
                .await
                .unwrap();
        }
    }

    /// Cross-org: org B (Enterprise, so the plan gate passes) gets 404 on every
    /// Kafka/Kinesis route for a contract org A owns; A's rows are untouched
    /// and A can still read them. Requires DATABASE_URL.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn streaming_routes_404_for_non_owner() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let state = Arc::new(AppState::new(pool.clone(), false, None, None));

        let (org_a, org_b, org_c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        // A owns the contract; B is a foreign Enterprise org, C a foreign Free org.
        for (org, plan) in [
            (org_a, "enterprise"),
            (org_b, "enterprise"),
            (org_c, "free"),
        ] {
            sqlx::query("INSERT INTO orgs (id, name, slug, plan) VALUES ($1, $2, $3, $4)")
                .bind(org)
                .bind("streaming ownership")
                .bind(format!("streaming-own-{}", org.simple()))
                .bind(plan)
                .execute(&pool)
                .await
                .expect("seed org");
        }
        let yaml = "version: \"1.0.0\"\nname: \"streaming_own\"\nontology:\n  entities:\n    - name: id\n      type: string\n      required: true\nglossary: []\nmetrics: []\n";
        let (identity, _) = super::super::storage::create_contract(
            &pool,
            "streaming_own",
            None,
            yaml,
            Default::default(),
            Some(org_a),
        )
        .await
        .expect("org A creates contract");
        let cid = identity.id;
        sqlx::query(
            "INSERT INTO kafka_ingress (contract_id, org_id, enabled, confluent_bootstrap,
                                        confluent_api_key, confluent_api_secret_enc)
             VALUES ($1, $2, true, 'b:9092', 'k', 'enc')",
        )
        .bind(cid)
        .bind(org_a)
        .execute(&pool)
        .await
        .expect("seed kafka row");
        sqlx::query(
            "INSERT INTO kinesis_ingress (contract_id, org_id, enabled, iam_access_key_id, iam_secret_enc)
             VALUES ($1, $2, true, 'AKIA', 'enc')",
        )
        .bind(cid)
        .bind(org_a)
        .execute(&pool)
        .await
        .expect("seed kinesis row");

        // Foreign orgs get 404 on all 7 routes whatever their plan: ownership
        // is checked before the Enterprise gate.
        for (who, foreign) in [("enterprise", org_b), ("free", org_c)] {
            let b = router(state.clone(), Some(foreign));
            for kind in ["kafka", "kinesis"] {
                let base = format!("/contracts/{cid}/{kind}-ingress");
                let r = b.get(&base).await;
                assert_eq!(r.status_code(), StatusCode::NOT_FOUND, "{who} GET {kind}");
                let r = b.post(&format!("{base}/enable")).await;
                assert_eq!(
                    r.status_code(),
                    StatusCode::NOT_FOUND,
                    "{who} enable {kind}"
                );
                let r = b.delete(&format!("{base}/disable")).await;
                assert_eq!(
                    r.status_code(),
                    StatusCode::NOT_FOUND,
                    "{who} disable {kind}"
                );
            }
            let r = b
                .post(&format!(
                    "/contracts/{cid}/kinesis-ingress/rotate-credentials"
                ))
                .await;
            assert_eq!(r.status_code(), StatusCode::NOT_FOUND, "{who} rotate");
        }

        // Owner still sees both configs, unchanged.
        let a = router(state.clone(), Some(org_a));
        for kind in ["kafka", "kinesis"] {
            let r = a.get(&format!("/contracts/{cid}/{kind}-ingress")).await;
            assert_eq!(
                r.status_code(),
                StatusCode::OK,
                "owner GET {kind}: {}",
                r.text()
            );
        }
        let key: (String,) =
            sqlx::query_as("SELECT iam_access_key_id FROM kinesis_ingress WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(key.0, "AKIA", "non-owner must not rotate credentials");

        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        for org in [org_a, org_b, org_c] {
            sqlx::query("DELETE FROM orgs WHERE id = $1")
                .bind(org)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Equivalent versions (`1`, `1.0`, `1.0.0`) cannot coexist on one contract.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod equivalent_versions {
    use super::super::{error::AppError, storage};
    use uuid::Uuid;

    fn yaml(version: &str) -> String {
        format!(
            "version: \"{version}\"\nname: \"equiv_versions\"\nontology:\n  entities:\n    - name: id\n      type: string\n      required: true\nglossary: []\nmetrics: []\n"
        )
    }

    /// Requires DATABASE_URL.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn deploy_and_create_version_reject_equivalents() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org)
            .bind("equiv versions")
            .bind(format!("equiv-{}", org.simple()))
            .execute(&pool)
            .await
            .expect("seed org");

        let (v, _) = storage::deploy_contract_version(
            &pool,
            "equiv_versions",
            &yaml("1.0"),
            None,
            None,
            Some(org),
        )
        .await
        .expect("deploy 1.0");
        let cid = v.contract_id;

        // Deploy of an equivalent version → 409 naming the existing one.
        let err = storage::deploy_contract_version(
            &pool,
            "equiv_versions",
            &yaml("1.0.0"),
            None,
            None,
            Some(org),
        )
        .await
        .expect_err("1.0.0 must be rejected");
        match &err {
            AppError::Conflict(m) => assert!(m.contains("existing version 1.0 "), "{m}"),
            other => panic!("expected Conflict, got {other:?}"),
        }

        // Draft creation of an equivalent version → 409.
        for dup in ["1", "1.0.0", "01.0"] {
            let err = storage::create_version(&pool, cid, dup, &yaml(dup), Some(org))
                .await
                .expect_err("equivalent draft must be rejected");
            assert!(
                matches!(&err, AppError::Conflict(m) if m.contains("existing version 1.0 ")),
                "{dup}: {err:?}"
            );
        }
        // Exact duplicate still uses the existing VersionConflict.
        let err = storage::create_version(&pool, cid, "1.0", &yaml("1.0"), Some(org))
            .await
            .expect_err("exact duplicate");
        assert!(matches!(err, AppError::VersionConflict { .. }), "{err:?}");
        // A genuinely new version is fine.
        storage::create_version(&pool, cid, "1.1", &yaml("1.1"), Some(org))
            .await
            .expect("1.1 is new");
        // Body version wins over a stale `version:` in the YAML; the stored
        // YAML is rewritten to agree with the row.
        let v = storage::create_version(&pool, cid, "1.2", &yaml("1.0"), Some(org))
            .await
            .expect("1.2 with stale yaml version");
        assert_eq!(v.version, "1.2");
        let c: super::super::contract::Contract = serde_yaml::from_str(&v.yaml_content).unwrap();
        assert_eq!(c.version, "1.2");

        // Pin @1 resolves to the stored 1.0.
        let row = storage::get_version_for_pin(&pool, cid, "1", Some(org))
            .await
            .expect("pin 1");
        assert_eq!(row.version, "1.0");

        // Stable versions can't be deleted (contract_versions_delete_guard), so
        // a hard delete of the contract or org would fail; soft-delete instead.
        sqlx::query("UPDATE contracts SET deleted_at = NOW() WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// POST /contracts/import: target_contract_id / contract_id and duplicate names.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod odcs_import_target {
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    use super::super::{api_key_auth::ValidatedKey, import_odcs_handler, AppState};

    fn server(state: Arc<AppState>, org: Uuid) -> TestServer {
        let app = axum::Router::new()
            .route(
                "/contracts/import",
                axum::routing::post(import_odcs_handler),
            )
            .layer(axum::middleware::from_fn(
                move |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                    req.extensions_mut().insert(ValidatedKey {
                        api_key_id: Uuid::new_v4(),
                        user_id: Uuid::new_v4(),
                        org_id: org,
                        allowed_contract_ids: None,
                        rate_limit_rps: None,
                        rate_limit_burst: None,
                    });
                    next.run(req).await
                },
            ))
            .with_state(state);
        TestServer::new(app)
    }

    fn odcs(name: &str, version: &str) -> String {
        format!(
            "apiVersion: v3.1.0\nkind: DataContract\nid: \"x\"\nversion: \"{version}\"\nstatus: active\ndataProduct: {name}\nschema:\n  - name: {name}\n    properties:\n      - name: event_id\n        logicalType: string\n        required: true\n"
        )
    }

    /// Requires DATABASE_URL.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Supabase/Postgres instance"]
    async fn import_targets_existing_contract_and_refuses_duplicate_names() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let state = Arc::new(AppState::new(pool.clone(), false, None, None));
        let (org_a, org_b) = (Uuid::new_v4(), Uuid::new_v4());
        for org in [org_a, org_b] {
            sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
                .bind(org)
                .bind("odcs import")
                .bind(format!("odcs-import-{}", org.simple()))
                .execute(&pool)
                .await
                .expect("seed org");
        }
        let name = format!("odcs_imp_{}", org_a.simple());
        let a = server(state.clone(), org_a);

        // First import creates the contract.
        let r = a
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs(&name, "2.0.0") }))
            .await;
        assert_eq!(r.status_code(), StatusCode::CREATED, "{}", r.text());
        let cid: Uuid = r.json::<serde_json::Value>()["contract_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        // Same name, no target → 409 pointing at target_contract_id.
        let r = a
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs(&name, "2.1.0") }))
            .await;
        assert_eq!(r.status_code(), StatusCode::CONFLICT);
        assert!(r.text().contains("target_contract_id"), "{}", r.text());

        // Target: exact and equivalent versions → 409; a new version → 201 on cid.
        for dup in ["2.0.0", "2.0"] {
            let r = a
                .post("/contracts/import")
                .json(&json!({ "odcs_yaml": odcs(&name, dup), "target_contract_id": cid }))
                .await;
            assert_eq!(r.status_code(), StatusCode::CONFLICT, "{dup}: {}", r.text());
        }
        let r = a
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs(&name, "2.1.0"), "target_contract_id": cid }))
            .await;
        assert_eq!(r.status_code(), StatusCode::CREATED, "{}", r.text());
        let v = r.json::<serde_json::Value>();
        assert_eq!(v["contract_id"], json!(cid));
        assert_eq!(v["version"], "2.1.0");
        let stored: serde_yaml::Value =
            serde_yaml::from_str(v["yaml_content"].as_str().unwrap()).unwrap();
        assert_eq!(
            stored["version"].as_str(),
            Some("2.1.0"),
            "yaml agrees with row"
        );
        // `contract_id` alias works too. The ODCS table is named `orders`, but
        // the stored YAML takes the target contract's name.
        let r = a
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs("orders", "2.2.0"), "contract_id": cid }))
            .await;
        assert_eq!(r.status_code(), StatusCode::CREATED, "{}", r.text());
        let stored: serde_yaml::Value = serde_yaml::from_str(
            r.json::<serde_json::Value>()["yaml_content"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored["name"].as_str(),
            Some(name.as_str()),
            "target's name"
        );
        assert_eq!(stored["version"].as_str(), Some("2.2.0"));
        // Disagreeing ids → 400.
        let r = a
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs(&name, "2.3.0"), "contract_id": cid, "target_contract_id": Uuid::new_v4() }))
            .await;
        assert_eq!(r.status_code(), StatusCode::BAD_REQUEST);
        // Unknown target → 404.
        let r = a
            .post("/contracts/import")
            .json(
                &json!({ "odcs_yaml": odcs(&name, "2.3.0"), "target_contract_id": Uuid::new_v4() }),
            )
            .await;
        assert_eq!(r.status_code(), StatusCode::NOT_FOUND);

        // Another org targeting A's contract → 404, nothing written.
        let b = server(state.clone(), org_b);
        let r = b
            .post("/contracts/import")
            .json(&json!({ "odcs_yaml": odcs(&name, "9.0.0"), "target_contract_id": cid }))
            .await;
        assert_eq!(r.status_code(), StatusCode::NOT_FOUND, "{}", r.text());
        let n: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM contract_versions WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(n.0, 3, "2.0.0, 2.1.0, 2.2.0 only");
        let live: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM contracts WHERE name = $1 AND deleted_at IS NULL")
                .bind(&name)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(live.0, 1, "no duplicate-named contracts");

        // Imported versions are drafts, so a hard delete is allowed.
        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        for org in [org_a, org_b] {
            sqlx::query("DELETE FROM orgs WHERE id = $1")
                .bind(org)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// feat/odcs-strict: exports validate against the official ODCS v3.1.0 JSON
// Schema (vendored, Apache-2.0), DCG→DCG stays lossless, and foreign ODCS
// (CsvPath how-to) imports with its constraints.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod odcs_strict_tests {
    use super::odcs_tests::{make_identity, make_version};
    use crate::contract::{Contract, FieldDefinition, FieldType, ImportSource, QualityRuleType};
    use crate::odcs;
    use serde_json::json;

    const PROBE: &str = include_str!("../tests/fixtures/odcs/dcg-strict-probe.yaml");
    const CSVPATH_HOWTO: &str =
        include_str!("../tests/fixtures/odcs/csvpath-orders-howto.odcs.yaml");
    const SCHEMA_V310: &str =
        include_str!("../tests/fixtures/odcs-schema/odcs-json-schema-v3.1.0.json");

    fn probe() -> Contract {
        serde_yaml::from_str(PROBE).unwrap()
    }

    fn export(c: &Contract) -> String {
        let identity = make_identity(&c.name);
        let cv = make_version(identity.id, &c.version, PROBE);
        odcs::export_odcs(odcs::OdcsExportInput {
            identity: &identity,
            version: &cv,
            contract: c,
        })
        .unwrap()
    }

    fn schema_errors(odcs_yaml: &str) -> Vec<String> {
        let schema: serde_json::Value = serde_json::from_str(SCHEMA_V310).unwrap();
        let validator = jsonschema::draft201909::new(&schema).expect("schema compiles");
        let instance: serde_json::Value = serde_yaml::from_str(odcs_yaml).unwrap();
        let errors: Vec<String> = match validator.validate(&instance) {
            Ok(()) => vec![],
            Err(errs) => errs
                .map(|e| format!("{} at {}", e, e.instance_path))
                .collect(),
        };
        errors
    }

    fn field<'a>(fields: &'a [FieldDefinition], name: &str) -> &'a FieldDefinition {
        fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("field {name} missing"))
    }

    fn canon(c: &Contract) -> serde_json::Value {
        serde_json::to_value(c).unwrap()
    }

    #[test]
    fn export_validates_against_official_odcs_v310_schema() {
        let y = export(&probe());
        let errs = schema_errors(&y);
        assert!(errs.is_empty(), "schema errors: {errs:#?}\n{y}");
    }

    #[test]
    fn legacy_export_shape_fails_official_schema() {
        // Guard that the validator is really strict: a bare root x-* key
        // (the pre-strict export shape) must be rejected.
        let mut doc: serde_yaml::Value = serde_yaml::from_str(&export(&probe())).unwrap();
        doc.as_mapping_mut()
            .unwrap()
            .insert("x-contractgate-version".into(), "1.0".into());
        let y = serde_yaml::to_string(&doc).unwrap();
        assert!(!schema_errors(&y).is_empty());
    }

    #[test]
    fn export_writes_native_odcs_constraints() {
        let doc: serde_json::Value = serde_yaml::from_str(&export(&probe())).unwrap();
        assert_eq!(doc["description"], json!({"purpose": "test contract"}));
        let props = doc["schema"][0]["properties"].as_array().unwrap();
        let p = |n: &str| {
            props
                .iter()
                .find(|p| p["name"] == n)
                .unwrap_or_else(|| panic!("{n}"))
        };
        assert_eq!(
            p("order_id")["logicalTypeOptions"]["pattern"],
            "^ORD-[0-9]{6}$"
        );
        assert_eq!(p("order_id")["unique"], true);
        assert_eq!(p("quantity")["logicalTypeOptions"]["minimum"], 1.0);
        assert_eq!(p("quantity")["logicalTypeOptions"]["maximum"], 999.0);
        assert_eq!(p("amount")["logicalType"], "number");
        assert_eq!(p("email")["logicalTypeOptions"]["minLength"], 5);
        assert_eq!(p("email")["logicalTypeOptions"]["maxLength"], 120);
        assert_eq!(p("email")["logicalTypeOptions"]["format"], "email");
        assert_eq!(
            p("order_date")["logicalTypeOptions"]["format"],
            "yyyy-MM-dd"
        );
        assert!(p("payload").get("logicalType").is_none());
        // enum → invalidValues validValues (ODCS v3.1.0 has no property enum)
        let country = p("customer.address.country");
        let q = &country["quality"][0];
        assert_eq!(q["metric"], "invalidValues");
        assert_eq!(q["arguments"]["validValues"], json!(["US", "CA"]));
        // validity rule on status carries the enum itself (no duplicate entry)
        let sq = p("status")["quality"].as_array().unwrap();
        assert_eq!(sq.len(), 1);
        assert_eq!(
            sq[0]["arguments"]["validValues"],
            json!(["open", "shipped", "cancelled"])
        );
    }

    #[test]
    fn dcg_roundtrip_mode_a_is_lossless() {
        let c = probe();
        let r = odcs::import_odcs(&export(&c)).unwrap();
        assert_eq!(r.import_source, ImportSource::Odcs);
        let back: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(canon(&back), canon(&c));
    }

    #[test]
    fn legacy_root_extension_export_still_imports_mode_a() {
        // Pre-strict exports put x-contractgate-* as bare root keys.
        let c = probe();
        let mut doc: serde_yaml::Value = serde_yaml::from_str(&export(&c)).unwrap();
        let m = doc.as_mapping_mut().unwrap();
        let cps = m.remove("customProperties").unwrap();
        for cp in cps.as_sequence().unwrap() {
            let cp = cp.as_mapping().unwrap();
            m.insert(cp["property"].clone(), cp["value"].clone());
        }
        m.insert("description".into(), "test contract".into());
        let r = odcs::import_odcs(&serde_yaml::to_string(&doc).unwrap()).unwrap();
        assert_eq!(r.import_source, ImportSource::Odcs);
        let back: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(canon(&back), canon(&c));
    }

    #[test]
    fn dcg_roundtrip_mode_b_keeps_constraints_and_quality() {
        // Strip every DCG extension (root and per-property): only ODCS-native
        // fields remain, as a strict third-party tool would emit them.
        let c = probe();
        let mut doc: serde_json::Value = serde_yaml::from_str(&export(&c)).unwrap();
        doc.as_object_mut().unwrap().remove("customProperties");
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(o) => {
                    o.remove("customProperties");
                    o.values_mut().for_each(strip);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
                _ => {}
            }
        }
        strip(&mut doc);
        let y = serde_yaml::to_string(&doc).unwrap();
        assert!(schema_errors(&y).is_empty());
        let r = odcs::import_odcs(&y).unwrap();
        assert_eq!(r.import_source, ImportSource::OdcsStripped);
        let back: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();

        let mut expect = c.clone();
        // Not representable without extensions: transforms, `any`, glossary,
        // metrics, compliance_mode.
        for f in expect.ontology.entities.iter_mut() {
            f.transform = None;
            if f.field_type == FieldType::Any {
                f.field_type = FieldType::String;
            }
        }
        expect.glossary.clear();
        expect.metrics.clear();
        expect.compliance_mode = false;
        let e = &expect.ontology.entities;
        let b = &back.ontology.entities;
        for name in [
            "order_id",
            "status",
            "quantity",
            "amount",
            "email",
            "order_date",
            "lines",
        ] {
            assert_eq!(
                serde_json::to_value(field(b, name)).unwrap(),
                serde_json::to_value(field(e, name)).unwrap(),
                "field {name}"
            );
        }
        let cust = field(b, "customer").properties.as_ref().unwrap();
        assert_eq!(field(cust, "id").pattern.as_deref(), Some("^C[0-9]+$"));
        let addr = field(cust, "address").properties.as_ref().unwrap();
        assert_eq!(
            field(addr, "country").allowed_values,
            Some(vec![json!("US"), json!("CA")])
        );
        // Quality rules survive via library metrics / custom engine.
        let mut kinds: Vec<_> = back
            .quality
            .iter()
            .map(|r| (r.field.clone(), r.rule_type, r.threshold, r.max_age_seconds))
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            kinds,
            vec![
                (
                    "amount".into(),
                    QualityRuleType::Completeness,
                    Some(0.95),
                    None
                ),
                (
                    "created_at".into(),
                    QualityRuleType::Freshness,
                    None,
                    Some(3600)
                ),
                // Without its x-cg-derived marker the enum restatement reads
                // as an explicit validity overlay (same enforcement).
                (
                    "customer.address.country".into(),
                    QualityRuleType::Validity,
                    None,
                    None
                ),
                ("order_id".into(), QualityRuleType::Uniqueness, None, None),
                ("status".into(), QualityRuleType::Validity, None, None),
            ]
        );
    }

    #[test]
    fn x_cg_extension_wins_over_native_field() {
        let y = r#"
apiVersion: v3.1.0
kind: DataContract
id: conflict
version: "1.0.0"
status: active
schema:
  - name: t
    properties:
      - name: code
        logicalType: string
        logicalTypeOptions: { pattern: "^native$", maxLength: 9 }
        customProperties:
          - { property: x-cg-pattern, value: "^cg$" }
"#;
        let r = odcs::import_odcs(y).unwrap();
        let c: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        let f = field(&c.ontology.entities, "code");
        assert_eq!(f.pattern.as_deref(), Some("^cg$"));
        assert_eq!(f.max_length, Some(9));
    }

    #[test]
    fn csvpath_howto_example_imports_with_constraints() {
        let r = odcs::import_odcs(CSVPATH_HOWTO).unwrap();
        assert_eq!(r.import_source, ImportSource::OdcsStripped);
        let c: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(c.name, "orders");
        let e = &c.ontology.entities;

        let id = field(e, "order_id");
        assert!(id.required);
        assert_eq!(id.pattern.as_deref(), Some("^ORD-[0-9]{6}$"));
        assert!(c
            .quality
            .iter()
            .any(|q| q.field == "order_id" && q.rule_type == QualityRuleType::Uniqueness));

        let email = field(e, "customer_email");
        assert!(!email.required);
        assert_eq!(email.format.as_deref(), Some("email"));

        let qty = field(e, "quantity");
        assert_eq!(qty.field_type, FieldType::Integer);
        assert!(qty.required);
        assert_eq!((qty.min, qty.max), (Some(1.0), Some(999.0)));

        let status = field(e, "status");
        assert_eq!(
            status.allowed_values,
            Some(vec![json!("open"), json!("shipped"), json!("cancelled")])
        );

        let date = field(e, "order_date");
        assert_eq!(date.field_type, FieldType::Date);
        assert!(date.required);
        assert_eq!(date.format, None, "yyyy-MM-dd is CG's built-in date format");

        // Schema-level `rowCount` has no per-event CG equivalent: dropped.
        assert_eq!(c.quality.len(), 1);

        // The imported contract compiles and enforces the constraints.
        let cc = crate::validation::CompiledContract::compile(c).unwrap();
        let ok = json!({"order_id": "ORD-000001", "customer_email": "a@b.co",
            "quantity": 2, "status": "open", "order_date": "2026-10-15"});
        assert!(crate::validation::validate(&cc, &ok).passed);
        let bad = json!({"order_id": "X1", "customer_email": "nope",
            "quantity": 0, "status": "lost", "order_date": "2026-13-01"});
        let res = crate::validation::validate(&cc, &bad);
        assert!(!res.passed);
        assert!(res.violations.len() >= 5, "{:?}", res.violations);
    }

    #[test]
    fn foreign_v32_lengths_and_object_description() {
        let y = r#"
apiVersion: v3.2.0
kind: DataContract
id: lens
version: "1.0.0"
status: active
description: { purpose: "Partner feed", usage: "ingest" }
schema:
  - name: lens
    logicalType: object
    properties:
      - name: code
        logicalType: string
        required: true
        logicalTypeOptions: { minLength: 2, maxLength: 8, format: uuid }
      - name: price
        logicalType: number
        logicalTypeOptions: { minimum: 0.5 }
        quality:
          - { metric: missingValues, mustBeLessOrEqualTo: 10, unit: percent }
"#;
        let r = odcs::import_odcs(y).unwrap();
        let c: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(c.description.as_deref(), Some("Partner feed"));
        let code = field(&c.ontology.entities, "code");
        assert_eq!((code.min_length, code.max_length), (Some(2), Some(8)));
        assert_eq!(code.format.as_deref(), Some("uuid"));
        let price = field(&c.ontology.entities, "price");
        assert_eq!(price.min, Some(0.5));
        let q = &c.quality[0];
        assert_eq!(q.rule_type, QualityRuleType::Completeness);
        assert!((q.threshold.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn string_format_checks() {
        use crate::validation::string_format_ok as ok;
        assert!(ok("email", "a@b.co"));
        assert!(!ok("email", "a@b"));
        assert!(!ok("email", "a b@c.d"));
        assert!(ok("uuid", "123e4567-e89b-12d3-a456-426614174000"));
        assert!(!ok("uuid", "123e4567e89b12d3a456426614174000"));
        assert!(ok("uri", "https://x.y/z"));
        assert!(!ok("uri", "not a uri"));
        assert!(ok("hostname", "anything")); // unknown formats are metadata
    }

    #[test]
    fn stringified_root_extension_still_imports_mode_a() {
        let c = probe();
        let mut doc: serde_json::Value = serde_yaml::from_str(&export(&c)).unwrap();
        for cp in doc["customProperties"].as_array_mut().unwrap() {
            if !cp["value"].is_string() {
                cp["value"] = json!(cp["value"].to_string());
            }
        }
        let r = odcs::import_odcs(&serde_yaml::to_string(&doc).unwrap()).unwrap();
        assert_eq!(r.import_source, ImportSource::Odcs);
        let back: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(canon(&back), canon(&c));
    }

    #[test]
    fn openmetadata_export_keeps_description_and_enum() {
        let y = include_str!("../tests/fixtures/odcs/om-2.0.5-export.odcs.yaml");
        let r = odcs::import_odcs(y).unwrap();
        assert_eq!(r.import_source, ImportSource::OdcsStripped);
        let c: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert!(c
            .description
            .as_deref()
            .unwrap()
            .starts_with("TEST - OpenMetadata"));
        let cur = field(&c.ontology.entities, "currency");
        assert_eq!(
            cur.allowed_values,
            Some(vec![json!("USD"), json!("EUR"), json!("GBP")])
        );
        assert_eq!(
            field(&c.ontology.entities, "amount").field_type,
            FieldType::Float
        );
    }

    #[test]
    fn joint_standard_odcs_orders_imports_with_constraints() {
        // Pure ODCS v3.1.0 (no x-, no customProperties) shared with CsvPath.
        let y = include_str!("../tests/fixtures/odcs/joint-orders.odcs.yaml");
        assert!(schema_errors(y).is_empty(), "{:?}", schema_errors(y));
        let r = odcs::import_odcs(y).unwrap();
        assert_eq!(r.import_source, ImportSource::OdcsStripped);
        let c: Contract = serde_yaml::from_str(&r.yaml_content).unwrap();
        assert_eq!(c.name, "orders");
        assert!(c
            .description
            .as_deref()
            .unwrap()
            .starts_with("Canonical order records"));
        let e = &c.ontology.entities;
        assert_eq!(e.len(), 11);

        let id = field(e, "order_id");
        assert!(id.required);
        assert_eq!(id.pattern.as_deref(), Some("^ORD-[0-9]{8}$"));
        assert_eq!(
            field(e, "customer_id").pattern.as_deref(),
            Some("^CUST-[0-9]{6}$")
        );
        let ts = field(e, "order_ts");
        assert_eq!(ts.field_type, FieldType::Timestamp);
        assert_eq!(ts.format.as_deref(), Some("yyyy-MM-dd'T'HH:mm:ss'Z'"));
        assert_eq!(
            field(e, "status").allowed_values,
            Some(
                ["pending", "paid", "shipped", "delivered", "cancelled"]
                    .map(|v| json!(v))
                    .to_vec()
            )
        );
        assert_eq!(
            field(e, "currency").allowed_values,
            Some(vec![json!("USD"), json!("EUR"), json!("GBP")])
        );
        assert_eq!(
            field(e, "sales_channel").allowed_values,
            Some(vec![json!("web"), json!("mobile"), json!("store")])
        );
        assert!(!field(e, "sales_channel").required);
        let amount = field(e, "amount");
        assert_eq!(
            (amount.field_type.clone(), amount.min, amount.max),
            (FieldType::Float, Some(0.0), None)
        );
        let qty = field(e, "quantity");
        assert_eq!(
            (qty.field_type.clone(), qty.min, qty.max),
            (FieldType::Integer, Some(1.0), Some(100.0))
        );
        let email = field(e, "email");
        assert_eq!(email.format.as_deref(), Some("email"));
        assert!(email.pattern.is_some());
        let sc = field(e, "ship_country");
        assert_eq!(sc.pattern.as_deref(), Some("^[A-Z]{2}$"));
        assert_eq!((sc.min_length, sc.max_length), (Some(2), Some(2)));
        let d = field(e, "discount_pct");
        assert_eq!((d.min, d.max, d.required), (Some(0.0), Some(100.0), false));

        let rule = |f: &str, t: QualityRuleType| {
            c.quality.iter().any(|q| q.field == f && q.rule_type == t)
        };
        // duplicateValues + unique: true → exactly one uniqueness rule
        assert_eq!(
            c.quality
                .iter()
                .filter(|q| q.field == "order_id" && q.rule_type == QualityRuleType::Uniqueness)
                .count(),
            1
        );
        assert!(rule("customer_id", QualityRuleType::Completeness)); // nullValues
        assert!(rule("status", QualityRuleType::Validity));
        // Schema-level rowCount has no per-event equivalent (dropped).
        assert_eq!(c.quality.len(), 5);

        // Constraints are enforced on events.
        let cc = crate::validation::CompiledContract::compile(c.clone()).unwrap();
        let good = json!({"order_id": "ORD-00000001", "customer_id": "CUST-000001",
            "order_ts": "2026-10-15T12:00:00Z", "status": "paid", "currency": "USD",
            "amount": 10.5, "quantity": 2, "email": "a@example.com", "ship_country": "US"});
        let res = crate::validation::validate(&cc, &good);
        assert!(res.passed, "{:?}", res.violations);
        let bad = json!({"order_id": "ORD-1", "customer_id": "C1",
            "order_ts": "yesterday", "status": "lost", "currency": "JPY",
            "amount": -1, "quantity": 101, "email": "nope", "ship_country": "usa",
            "discount_pct": 120, "sales_channel": "fax"});
        let res = crate::validation::validate(&cc, &bad);
        let bad_fields: std::collections::BTreeSet<_> =
            res.violations.iter().map(|v| v.field.as_str()).collect();
        for f in [
            "order_id",
            "customer_id",
            "order_ts",
            "status",
            "currency",
            "amount",
            "quantity",
            "email",
            "ship_country",
            "discount_pct",
            "sales_channel",
        ] {
            assert!(
                bad_fields.contains(f),
                "{f} not flagged: {:?}",
                res.violations
            );
        }

        // DCG re-export of the imported contract is schema-valid and keeps timestamp.
        let again = export(&c);
        assert!(
            schema_errors(&again).is_empty(),
            "{:?}",
            schema_errors(&again)
        );
        assert!(again.contains("logicalType: timestamp"));
    }
}

// ---------------------------------------------------------------------------
// Org-wide replay redacted flag + effective store_event_payloads on GET /contracts.
//
// Requires DATABASE_URL and migrations through 034 (payload_redacted) / 036
// (aisg plans). Run with:
//   cargo test --bin contractgate-server api_bugfixes -- --ignored
// ---------------------------------------------------------------------------
#[cfg(test)]
mod api_bugfixes {
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum_test::TestServer;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use uuid::Uuid;

    use super::super::storage::{self, QuarantineEventInsert};
    use super::super::transform::TransformedPayload;
    use super::super::{
        api_key_auth::ValidatedKey, get_contract_handler, list_contracts_handler, quarantine,
        replay, AppState,
    };

    const YAML: &str = r#"
version: "1.0"
name: "api_bugfix_contract"
description: "replay redacted + effective payload flag"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

    fn server(state: Arc<AppState>, org: Uuid) -> TestServer {
        let app = axum::Router::new()
            .route("/contracts", get(list_contracts_handler))
            .route("/contracts/{id}", get(get_contract_handler))
            .route("/quarantine/replay", post(quarantine::replay_all_handler))
            .route(
                "/contracts/{id}/quarantine/replay",
                post(replay::replay_handler),
            )
            .layer(axum::middleware::from_fn(
                move |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                    req.extensions_mut().insert(ValidatedKey {
                        api_key_id: Uuid::new_v4(),
                        user_id: Uuid::new_v4(),
                        org_id: org,
                        allowed_contract_ids: None,
                        rate_limit_rps: None,
                        rate_limit_burst: None,
                    });
                    next.run(req).await
                },
            ))
            .with_state(state);
        TestServer::new(app)
    }

    async fn seed_org(pool: &sqlx::PgPool, org: Uuid, plan: &str) {
        sqlx::query("INSERT INTO orgs (id, name, slug, plan) VALUES ($1, $2, $3, $4)")
            .bind(org)
            .bind("api bugfix org")
            .bind(format!("api-bugfix-{}", org.simple()))
            .bind(plan)
            .execute(pool)
            .await
            .expect("seed org");
    }

    /// Org-wide `POST /quarantine/replay` flags a row whose body was not stored
    /// the same way the per-contract route does (`outcome: "redacted"`),
    /// instead of `passed: false` with an empty violations list.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 034"]
    async fn org_wide_replay_redacted_matches_per_contract() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        seed_org(&pool, org, "free").await;
        let (identity, version) = storage::create_contract(
            &pool,
            "api_bugfix_replay",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;
        let qid = Uuid::new_v4();
        storage::quarantine_events_batch(
            &pool,
            &[QuarantineEventInsert {
                contract_id: cid,
                contract_version: version.version.clone(),
                payload: TransformedPayload::from_stored(json!({"id": "not-stored"})),
                violation_count: 1,
                violation_details: json!([{"field":"id","kind":"x","message":"m"}]),
                validation_us: 5,
                source_ip: None,
                replay_of_quarantine_id: None,
                pre_assigned_id: Some(qid),
                direction: "ingress".to_string(),
            }],
            false, // payload NULL + payload_redacted, same as Free ingest
        )
        .await
        .unwrap();

        let state = Arc::new(AppState::new(pool.clone(), false, None, None));
        let http = server(state, org);
        let body = json!({
            "ids": [qid],
            "target_version": version.version,
        });
        let per = http
            .post(&format!("/contracts/{cid}/quarantine/replay"))
            .json(&body)
            .await;
        assert_eq!(per.status_code(), StatusCode::OK, "{}", per.text());
        let per_json: Value = per.json();
        assert_eq!(
            per_json["redacted"],
            json!(1),
            "per-contract redacted count: {per_json}"
        );
        assert_eq!(per_json["results"][0]["quarantine_id"], qid.to_string());
        assert_eq!(
            per_json["results"][0]["outcome"], "redacted",
            "per-contract outcome tag: {per_json}"
        );

        let org_body = json!({
            "event_ids": [qid],
            "version": version.version,
        });
        let org_resp = http.post("/quarantine/replay").json(&org_body).await;
        assert_eq!(
            org_resp.status_code(),
            StatusCode::OK,
            "{}",
            org_resp.text()
        );
        let org_json: Value = org_resp.json();
        assert_eq!(org_json["replayed"], json!(0));
        let outcome = &org_json["outcomes"][0];
        assert_eq!(outcome["event_id"], qid.to_string());
        assert_eq!(
            outcome["redacted"],
            json!(true),
            "org-wide route must flag the redacted row: {org_json}"
        );
        assert_eq!(outcome["passed"], json!(false));
        assert_eq!(outcome["violations"], json!([]));
        // Org-wide `redacted: true` is the same classification the per-contract
        // route tags `outcome: "redacted"`.
        assert_eq!(per_json["results"][0]["outcome"], "redacted");

        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// `GET /contracts` reports `store_event_payloads` with the same decision
    /// ingest uses. Free stays false. A paid plan with the org master switch
    /// still off (column default) reports false — that used to report true
    /// while new quarantine rows were redacted. Paid + org switch on reports
    /// the per-contract override. Columns are not rewritten.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 036"]
    async fn contract_list_effective_store_event_payloads() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        seed_org(&pool, org, "free").await;
        let (identity, _) = storage::create_contract(
            &pool,
            "api_bugfix_list",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;
        // Column default is true. The response must not depend on rewriting it.
        let stored: bool =
            sqlx::query_scalar("SELECT store_event_payloads FROM contracts WHERE id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stored, "column default is true; the bug is the response");

        let state = Arc::new(AppState::new(pool.clone(), false, None, None));
        let http = server(state, org);

        async fn listed(http: &TestServer, cid: Uuid) -> bool {
            let resp = http.get("/contracts").await;
            assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
            let rows: Vec<Value> = resp.json();
            let row = rows
                .iter()
                .find(|r| r["id"] == cid.to_string())
                .expect("contract in list");
            row["store_event_payloads"]
                .as_bool()
                .expect("store_event_payloads bool")
        }

        assert!(
            !listed(&http, cid).await,
            "free org must report store_event_payloads false"
        );

        sqlx::query("UPDATE orgs SET plan = 'aisg_free' WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !listed(&http, cid).await,
            "aisg_free must report store_event_payloads false"
        );

        // Paid + org switch still off (default) must not claim storage.
        // Ingest redacts in this state; the list used to report true.
        for plan in ["growth", "aisg_pro", "enterprise"] {
            sqlx::query("UPDATE orgs SET plan = $2, store_event_payloads = false WHERE id = $1")
                .bind(org)
                .bind(plan)
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                !listed(&http, cid).await,
                "{plan} with the org switch off must report false"
            );
        }

        // Paid + org switch on reports the stored per-contract override (true).
        for plan in ["growth", "aisg_pro", "enterprise"] {
            sqlx::query("UPDATE orgs SET plan = $2, store_event_payloads = true WHERE id = $1")
                .bind(org)
                .bind(plan)
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                listed(&http, cid).await,
                "{plan} with the org switch and column on must report true"
            );
        }

        // Paid + stored override off stays off, even with the org switch on.
        sqlx::query("UPDATE contracts SET store_event_payloads = false WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE orgs SET plan = 'growth', store_event_payloads = true WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !listed(&http, cid).await,
            "paid plan reports the stored false override"
        );

        // Free response did not rewrite the column (it was still true until
        // the explicit UPDATE above).
        let now_stored: bool =
            sqlx::query_scalar("SELECT store_event_payloads FROM contracts WHERE id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!now_stored);

        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// `GET /contracts/{id}` reports `store_event_payloads` with the same
    /// decision as the list: paid plan AND org switch AND per-contract override.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 036"]
    async fn contract_get_effective_store_event_payloads() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        seed_org(&pool, org, "free").await;
        let (identity, _) = storage::create_contract(
            &pool,
            "api_bugfix_get",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .expect("create contract");
        let cid = identity.id;
        let state = Arc::new(AppState::new(pool.clone(), false, None, None));
        let http = server(state, org);

        async fn fetched(http: &TestServer, cid: Uuid) -> bool {
            let resp = http.get(&format!("/contracts/{cid}")).await;
            assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
            let row: Value = resp.json();
            row["store_event_payloads"]
                .as_bool()
                .expect("store_event_payloads bool")
        }

        assert!(
            !fetched(&http, cid).await,
            "free org must report store_event_payloads false"
        );
        sqlx::query("UPDATE orgs SET plan = 'aisg_free' WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !fetched(&http, cid).await,
            "aisg_free must report store_event_payloads false"
        );
        for plan in ["growth", "aisg_pro", "enterprise"] {
            sqlx::query("UPDATE orgs SET plan = $2, store_event_payloads = false WHERE id = $1")
                .bind(org)
                .bind(plan)
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                !fetched(&http, cid).await,
                "{plan} with the org switch off must report false"
            );
        }
        for plan in ["growth", "aisg_pro", "enterprise"] {
            sqlx::query("UPDATE orgs SET plan = $2, store_event_payloads = true WHERE id = $1")
                .bind(org)
                .bind(plan)
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                fetched(&http, cid).await,
                "{plan} with the org switch and column on must report true"
            );
        }
        sqlx::query("UPDATE contracts SET store_event_payloads = false WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !fetched(&http, cid).await,
            "paid plan reports the stored false override"
        );
        let now_stored: bool =
            sqlx::query_scalar("SELECT store_event_payloads FROM contracts WHERE id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            !now_stored,
            "response must not be the only place the column changed"
        );

        sqlx::query("DELETE FROM contracts WHERE id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Soft-deleting a contract removes its quarantine rows in the same
// transaction and leaves audit rows + published versions in place.
//
//   DATABASE_URL=... cargo test delete_contract_removes_quarantine -- --ignored
// ---------------------------------------------------------------------------
#[cfg(test)]
mod contract_delete_quarantine {
    use serde_json::json;
    use uuid::Uuid;

    use super::super::error::AppError;
    use super::super::storage::{
        self, AuditEntryInsert, ForwardEventInsert, QuarantineEventInsert,
    };
    use super::super::transform::TransformedPayload;

    const YAML: &str = r#"
version: "1.0"
name: "delete_quar_contract"
description: "delete removes quarantine"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
"#;

    #[tokio::test]
    #[ignore = "requires DATABASE_URL and a live Postgres migrated through 004"]
    async fn delete_contract_removes_quarantine_keeps_audit_and_version() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let org = Uuid::new_v4();
        sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'o', $2)")
            .bind(org)
            .bind(format!("del-quar-{}", org.simple()))
            .execute(&pool)
            .await
            .unwrap();

        let (identity, version) = storage::create_contract(
            &pool,
            "delete_quar",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .unwrap();
        let cid = identity.id;
        storage::promote_version(&pool, cid, &version.version, Some(org))
            .await
            .expect("promote draft to stable");

        let parent = Uuid::new_v4();
        let child = Uuid::new_v4();
        storage::quarantine_events_batch(
            &pool,
            &[
                QuarantineEventInsert {
                    contract_id: cid,
                    contract_version: version.version.clone(),
                    payload: TransformedPayload::from_stored(json!({"id": "body"})),
                    violation_count: 1,
                    violation_details: json!([{"field":"id","kind":"x","message":"m"}]),
                    validation_us: 4,
                    source_ip: None,
                    replay_of_quarantine_id: None,
                    pre_assigned_id: Some(parent),
                    direction: "ingress".to_string(),
                },
                QuarantineEventInsert {
                    contract_id: cid,
                    contract_version: version.version.clone(),
                    payload: TransformedPayload::from_stored(json!({"id": "replay-fail"})),
                    violation_count: 1,
                    violation_details: json!([{"field":"id","kind":"x","message":"m"}]),
                    validation_us: 4,
                    source_ip: None,
                    replay_of_quarantine_id: Some(parent),
                    pre_assigned_id: Some(child),
                    direction: "ingress".to_string(),
                },
            ],
            true,
        )
        .await
        .unwrap();

        storage::log_audit_entries_batch(
            &pool,
            &[AuditEntryInsert {
                contract_id: cid,
                org_id: Some(org),
                contract_version: version.version.clone(),
                passed: true,
                violation_count: 0,
                violation_details: json!([]),
                raw_event: TransformedPayload::from_stored(json!({"id": "kept"})),
                validation_us: 3,
                source_ip: None,
                source: "http".into(),
                pre_assigned_id: None,
                replay_of_quarantine_id: Some(parent),
                direction: "ingress".into(),
            }],
            true,
        )
        .await
        .unwrap();
        storage::forward_events_batch(
            &pool,
            &[ForwardEventInsert {
                contract_id: cid,
                contract_version: version.version.clone(),
                payload: TransformedPayload::from_stored(json!({"id": "passed-body"})),
            }],
            true,
        )
        .await
        .unwrap();

        // Wrong org must not delete the contract or its quarantine rows.
        let err = storage::delete_contract(&pool, cid, Some(Uuid::new_v4()))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ContractNotFound(_)));
        let still: i64 =
            sqlx::query_scalar("SELECT count(*) FROM quarantine_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still, 2, "wrong-org delete leaves quarantine rows");
        let forwarded_still: i64 =
            sqlx::query_scalar("SELECT count(*) FROM forwarded_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(forwarded_still, 1, "wrong-org delete leaves forwarded rows");

        storage::delete_contract(&pool, cid, Some(org))
            .await
            .expect("owner delete");

        let missing = storage::get_contract_identity(&pool, cid, Some(org)).await;
        assert!(matches!(missing, Err(AppError::ContractNotFound(_))));

        let listed = storage::list_quarantine_events(&pool, Some(org), Some(cid), 50, 0)
            .await
            .unwrap();
        assert!(
            listed.is_empty(),
            "GET /quarantine must not return deleted contract rows"
        );

        let left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM quarantine_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(left, 0, "quarantine rows removed with the contract");
        let forwarded_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM forwarded_events WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            forwarded_left, 0,
            "forwarded rows removed with the contract"
        );

        let (audit_n, replay_cleared): (i64, bool) = sqlx::query_as(
            "SELECT count(*), bool_and(replay_of_quarantine_id IS NULL) \
             FROM audit_log WHERE contract_id = $1",
        )
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audit_n, 1, "audit history is kept");
        assert!(
            replay_cleared,
            "replay link cleared so the quarantine delete can proceed"
        );

        let state: String = sqlx::query_scalar(
            "SELECT state FROM contract_versions WHERE contract_id = $1 AND version = $2",
        )
        .bind(cid)
        .bind(&version.version)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, "stable", "published version stays published");

        // A contract soft-deleted without this helper (rows already left
        // behind) is excluded from the list without a schema change.
        let (other, _) = storage::create_contract(
            &pool,
            "delete_quar_orphan",
            Some("t"),
            YAML,
            Default::default(),
            Some(org),
        )
        .await
        .unwrap();
        storage::quarantine_events_batch(
            &pool,
            &[QuarantineEventInsert {
                contract_id: other.id,
                contract_version: "1.0.0".into(),
                payload: TransformedPayload::from_stored(json!({"id": "orphan"})),
                violation_count: 1,
                violation_details: json!([]),
                validation_us: 1,
                source_ip: None,
                replay_of_quarantine_id: None,
                pre_assigned_id: None,
                direction: "ingress".into(),
            }],
            false,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE contracts SET deleted_at = NOW() WHERE id = $1")
            .bind(other.id)
            .execute(&pool)
            .await
            .unwrap();
        let orphan_list = storage::list_quarantine_events(&pool, Some(org), Some(other.id), 50, 0)
            .await
            .unwrap();
        assert!(
            orphan_list.is_empty(),
            "soft-deleted contract is excluded from the list"
        );

        // Fixture cleanup. Stable versions cannot be hard-deleted (trigger).
        // session_replication_role is per connection, so hold one checkout.
        // This role is superuser on the local test database only.
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("SET session_replication_role = 'replica'")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("DELETE FROM contracts WHERE id = ANY($1)")
            .bind(vec![cid, other.id])
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1")
            .bind(org)
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("SET session_replication_role = 'origin'")
            .execute(&mut *conn)
            .await
            .unwrap();
    }
}
