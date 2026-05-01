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
        Contract, FieldDefinition, FieldType, GlossaryEntry, MetricDefinition, Ontology,
        QualityRule,
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
    use crate::contract::{Contract, FieldType, GlossaryEntry, MetricDefinition};
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
                name: "valid_event".into(),
                description: "Event must have a valid event_type".into(),
                check: "event_type in ['click', 'view', 'purchase', 'login']".into(),
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
        ContractVersion, CreateContractRequest, CreateVersionRequest, MultiStableResolution,
        PatchContractRequest, PatchVersionRequest, VersionResponse, VersionState,
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
        ContractIdentity, ContractVersion, FieldType, GlossaryEntry, ImportSource,
        MetricDefinition, MultiStableResolution, VersionState,
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
            vec![QualityRule {
                name: "valid_event".into(),
                description: "Event must have a valid event_type".into(),
                check: "event_type in ['click', 'view', 'purchase', 'login']".into(),
            }],
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

        let result = odcs::import_odcs(odcs_yaml).expect("stripped import must succeed");

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
}
