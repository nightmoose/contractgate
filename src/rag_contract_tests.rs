//! RFC-077 — RAG-ingestion contract profile tests.
//!
//! These load the *actual* example contracts shipped in `examples/contracts/rag/`
//! (via `include_str!`) and exercise them against the real validation engine.
//! The point is to prove RFC-077's central claim — "no engine change is needed,
//! the profile is expressible in the existing contract format" — and to keep the
//! example files from silently rotting as the engine evolves.
//!
//! If any assertion here fails, the example contract (or the engine) has drifted
//! and RFC-077 needs revisiting before it leaves Draft.

#[cfg(test)]
mod tests {
    use crate::contract::Contract;
    use crate::validation::{validate, CompiledContract, ViolationKind};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    const RAG_CORPUS_YAML: &str = include_str!("../examples/contracts/rag/rag_corpus_ingest.yaml");
    const FINE_TUNING_YAML: &str =
        include_str!("../examples/contracts/rag/fine_tuning_corpus.yaml");

    fn compile(yaml: &str) -> CompiledContract {
        let contract: Contract =
            serde_yaml::from_str(yaml).expect("example contract must parse as a Contract");
        CompiledContract::compile(contract).expect("example contract must compile")
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn has_kind(r: &crate::validation::ValidationResult, kind: ViolationKind) -> bool {
        r.violations.iter().any(|v| v.kind == kind)
    }

    // ── rag_corpus_ingest ───────────────────────────────────────────────────

    #[test]
    fn rag_clean_record_passes() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({
            "text": "Quarterly revenue grew 12% QoQ.",
            "_cg": {
                "source": "confluence",
                "doc_id": "conf:page-12345",
                "ingested_at": now(),
                "pii_redacted": true
            }
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "clean record should pass; got {:?}", r.violations);
    }

    #[test]
    fn rag_unredacted_pii_is_rejected() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({
            "text": "Contact john@example.com for details.",
            "_cg": {
                "source": "confluence",
                "doc_id": "conf:page-12345",
                "ingested_at": now(),
                "pii_redacted": false   // attestation not satisfied
            }
        });
        let r = validate(&cc, &event);
        assert!(!r.passed, "pii_redacted:false must be rejected");
        assert!(
            has_kind(&r, ViolationKind::EnumViolation),
            "expected an enum violation on _cg.pii_redacted; got {:?}",
            r.violations
        );
    }

    #[test]
    fn rag_stale_document_is_rejected() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({
            "text": "An old note.",
            "_cg": {
                "source": "gdrive",
                "doc_id": "gdrive:abc",
                "ingested_at": now() - 60 * 60 * 24 * 365, // ~1 year old
                "pii_redacted": true
            }
        });
        let r = validate(&cc, &event);
        assert!(!r.passed, "stale ingested_at must be rejected");
        assert!(
            has_kind(&r, ViolationKind::FreshnessViolation),
            "expected a freshness violation; got {:?}",
            r.violations
        );
    }

    #[test]
    fn rag_disallowed_source_is_rejected() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({
            "text": "Some text.",
            "_cg": {
                "source": "random-scraper",  // not in the enum allowlist
                "doc_id": "x:1",
                "ingested_at": now(),
                "pii_redacted": true
            }
        });
        let r = validate(&cc, &event);
        assert!(!r.passed, "off-allowlist source must be rejected");
        assert!(
            has_kind(&r, ViolationKind::EnumViolation),
            "expected an enum violation on _cg.source; got {:?}",
            r.violations
        );
    }

    #[test]
    fn rag_missing_envelope_is_rejected() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({ "text": "No envelope at all." });
        let r = validate(&cc, &event);
        assert!(!r.passed, "missing _cg envelope must be rejected");
        assert!(
            has_kind(&r, ViolationKind::MissingRequiredField),
            "expected a missing-required-field violation; got {:?}",
            r.violations
        );
    }

    #[test]
    fn rag_undeclared_top_level_key_is_rejected_under_compliance_mode() {
        let cc = compile(RAG_CORPUS_YAML);
        let event = json!({
            "text": "Body.",
            "stray_top_level": "should not be here",
            "_cg": {
                "source": "confluence",
                "doc_id": "conf:1",
                "ingested_at": now(),
                "pii_redacted": true
            }
        });
        let r = validate(&cc, &event);
        assert!(
            !r.passed,
            "undeclared top-level key must be rejected under compliance_mode"
        );
    }

    // ── fine_tuning_corpus ──────────────────────────────────────────────────

    #[test]
    fn fine_tuning_clean_record_passes() {
        let cc = compile(FINE_TUNING_YAML);
        let event = json!({
            "prompt": "Translate 'hello' to French.",
            "completion": "Bonjour.",
            "_cg": {
                "source": "human-labeled",
                "example_id": "ex:0001",
                "ingested_at": now(),
                "pii_redacted": true,
                "license_ok": true
            }
        });
        let r = validate(&cc, &event);
        assert!(r.passed, "clean record should pass; got {:?}", r.violations);
    }

    #[test]
    fn fine_tuning_uncleared_license_is_rejected() {
        let cc = compile(FINE_TUNING_YAML);
        let event = json!({
            "prompt": "p",
            "completion": "c",
            "_cg": {
                "source": "synthetic",
                "example_id": "ex:0002",
                "ingested_at": now(),
                "pii_redacted": true,
                "license_ok": false   // not cleared for training use
            }
        });
        let r = validate(&cc, &event);
        assert!(!r.passed, "license_ok:false must be rejected");
        assert!(
            has_kind(&r, ViolationKind::EnumViolation),
            "expected an enum violation on _cg.license_ok; got {:?}",
            r.violations
        );
    }
}
