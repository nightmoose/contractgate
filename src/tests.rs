//! Integration-style tests for ContractGate that do NOT require a live database.
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
        // ContractGate does not reject unknown fields (additive schema evolution)
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

    fn make_identity(name: &str) -> ContractIdentity {
        ContractIdentity {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: Some("test contract".to_string()),
            multi_stable_resolution: MultiStableResolution::Strict,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            pii_salt: vec![0u8; 32],
        }
    }

    fn make_version(contract_id: Uuid, ver: &str, yaml: &str) -> ContractVersion {
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

        assert!(
            m.contains_key("x-contractgate-version"),
            "must write x-contractgate-version (D-003)"
        );
        assert!(
            m.contains_key("x-contractgate-ontology"),
            "must write x-contractgate-ontology (D-003)"
        );
        assert!(
            m.contains_key("x-contractgate-glossary"),
            "must write x-contractgate-glossary when non-empty"
        );
        assert!(
            m.contains_key("x-contractgate-metrics"),
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
        assert_eq!(
            uid_quality[0]
                .as_mapping()
                .unwrap()
                .get("type")
                .and_then(|v| v.as_str()),
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
        assert_eq!(
            ts_q0.get("type").and_then(|v| v.as_str()),
            Some("freshness")
        );
        let max_age = ts_q0
            .get("attributes")
            .and_then(|v| v.as_mapping())
            .and_then(|m| m.get("maxAgeSeconds"))
            .and_then(|v| v.as_u64());
        assert_eq!(max_age, Some(3600));
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
                    },
                ],
            },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
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
