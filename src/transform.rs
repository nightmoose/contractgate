//! PII transform engine (RFC-004).
//!
//! Runs AFTER validation, BEFORE any write to `audit_log` /
//! `quarantine_events` / forward destination.  The invariant this module
//! enforces: raw PII values never leave the in-memory validator.  Every
//! payload that lands on disk or flows downstream has been through
//! [`apply_transforms`] first.
//!
//! Four kinds (signed off 2026-04-19):
//!   - `mask` — `style: opaque` (default, `"****"`) or
//!     `format_preserving` (same length + char-class per position,
//!     deterministic per contract).
//!   - `hash` — HMAC-SHA256 keyed on the per-contract `pii_salt`; emitted
//!     as `"hmac-sha256:<hex>"`.  Deterministic so downstream joins on
//!     hashed keys work forever.
//!   - `drop` — the field is removed from the payload.
//!   - `redact` — replaced with the literal sentinel `"<REDACTED>"`.
//!
//! All four operate on top-level string fields only.  Non-string fields
//! with a transform declared are rejected at contract-compile time by
//! `validation::validate_transform_types`, so by the time we get here we
//! can trust the string assumption.

use crate::contract::{MaskStyle, TransformKind};
use crate::validation::CompiledContract;
use hmac::{Hmac, Mac};
use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;
use rand_core::SeedableRng;
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A payload that has been through the transform engine.  Storage helpers
/// (`log_audit_entry_batch`, `quarantine_events_batch`) and the forward
/// path take this type instead of a raw `serde_json::Value`, so the
/// compiler prevents accidentally writing un-transformed data to disk.
///
/// The wrapper is a transparent newtype — `into_inner` yields the underlying
/// JSON for serialization.  There is no `From<Value> for TransformedPayload`
/// impl: the only legal way to produce one is [`apply_transforms`].
#[derive(Debug, Clone)]
pub struct TransformedPayload(Value);

impl TransformedPayload {
    /// Consume the wrapper and return the underlying JSON.
    pub fn into_inner(self) -> Value {
        self.0
    }

    /// Borrow the underlying JSON (for read paths that don't need ownership).
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// Mint a `TransformedPayload` from a value that was **already** stored
    /// in the post-transform form — i.e. read back from
    /// `quarantine_events.payload` or `audit_log.raw_event`.  This is the
    /// single documented crack in the "only `apply_transforms` produces a
    /// `TransformedPayload`" invariant (see RFC-004 §Replay).
    ///
    /// Legitimate callers:
    ///   - `src/replay.rs` — re-validating a quarantined payload under a
    ///     new target version writes the stored form back verbatim.
    ///   - Summary audit rows (batch-rejected, deprecated-pin) that carry
    ///     synthetic bookkeeping JSON rather than user event data.
    ///
    /// **Never call this on data that came from an HTTP request body or
    /// any other client-controlled source** — that bypasses the whole
    /// "raw PII never leaves the validator" guarantee.  If you are
    /// working with a raw event, route it through
    /// [`apply_transforms`] instead.
    pub fn from_stored(value: Value) -> Self {
        Self(value)
    }
}

impl serde::Serialize for TransformedPayload {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

/// Apply every declared transform on top of `raw`, returning the
/// post-transform payload.  If the compiled contract has
/// `compliance_mode = true`, any field not declared in `ontology.entities`
/// is ALSO stripped from the stored form — the violation was already
/// raised by the validator, stripping here just prevents the raw value
/// from leaking through the audit / forward path.
pub fn apply_transforms(compiled: &CompiledContract, raw: Value) -> TransformedPayload {
    // Non-object roots can't have named fields, so there's nothing to
    // transform.  Preserve the original shape (the validator already
    // emitted a type-mismatch violation for this case).
    let mut obj = match raw {
        Value::Object(map) => map,
        other => return TransformedPayload(other),
    };

    // Apply each entity's transform, if any.  Only top-level fields are
    // supported in v1; nested transforms are a future RFC.
    for entity in &compiled.contract.ontology.entities {
        let Some(transform) = &entity.transform else {
            continue;
        };

        // Entity missing from this event — nothing to rewrite.  If the
        // field was required, the validator already raised a violation.
        let Some(existing) = obj.get(&entity.name) else {
            continue;
        };

        // Transforms only operate on strings; compile-time check in
        // validation::validate_transform_types guarantees the entity is
        // typed as `string`.  If the inbound event put a non-string value
        // in a string field the validator already flagged it; here we
        // leave the field untouched rather than coerce (preserving the
        // value means the quarantine row surfaces what the client
        // actually sent, which is useful for debugging).
        let Some(s) = existing.as_str() else { continue };

        let replacement = match transform.kind {
            TransformKind::Drop => {
                obj.remove(&entity.name);
                continue;
            }
            TransformKind::Redact => json!("<REDACTED>"),
            TransformKind::Mask => {
                let style = transform.style.unwrap_or_default();
                match style {
                    MaskStyle::Opaque => json!("****"),
                    MaskStyle::FormatPreserving => {
                        json!(format_preserving_mask(s, &compiled.pii_salt, &entity.name))
                    }
                }
            }
            TransformKind::Hash => {
                json!(format!(
                    "hmac-sha256:{}",
                    hmac_sha256_hex(&compiled.pii_salt, s.as_bytes())
                ))
            }
        };

        obj.insert(entity.name.clone(), replacement);
    }

    // RFC-004 Q4: under compliance mode, strip undeclared fields from the
    // stored payload so the raw value never reaches audit / quarantine /
    // forward.  The validator already raised UNDECLARED_FIELD for them.
    if compiled.contract.compliance_mode {
        obj.retain(|k, _| compiled.declared_top_level_fields.contains(k));
    }

    TransformedPayload(Value::Object(obj))
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/// HMAC-SHA256 of `msg` keyed by `key`, returned as lowercase hex.
/// Deterministic: same (key, msg) always yields the same hex.  Exposed
/// `pub(crate)` so tests + the validator integration tests can assert
/// determinism without re-implementing HMAC.
pub(crate) fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    // HMAC-SHA256 accepts keys of any length, including empty.  The
    // `.expect()` is defensive — the concrete `new_from_slice` on
    // `HmacSha256` can only fail for algorithms with a fixed key size,
    // which SHA-256 is not.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(msg);
    hex::encode(mac.finalize().into_bytes())
}

/// Format-preserving mask (RFC-004 Q6 = seeded ChaCha20 per-position
/// scramble).  For every position in `input`:
///   - ASCII digit → random ASCII digit (0–9)
///   - ASCII upper → random ASCII upper (A–Z)
///   - ASCII lower → random ASCII lower (a–z)
///   - any other → passed through unchanged (symbols, whitespace,
///     non-ASCII bytes keep their original value)
///
/// The PRNG is seeded deterministically on `(salt, field_name)` so the
/// same input under the same contract + field always produces the same
/// output.  Not reversible, not a formal FPE scheme.  Not intended to
/// resist a motivated attacker with the salt — see RFC-004 non-goals.
pub(crate) fn format_preserving_mask(input: &str, salt: &[u8], field_name: &str) -> String {
    use rand::rngs::StdRng;
    use rand::Rng;
    use rand::SeedableRng;

    let seed_bytes = {
        let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC-SHA256 accepts any key length");
        mac.update(field_name.as_bytes());
        mac.finalize().into_bytes()
    };
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);

    let mut rng = StdRng::from_seed(seed);

    let mut out = Vec::with_capacity(input.len());
    for b in input.bytes() {
        let replacement = if b.is_ascii_digit() {
            b'0' + (rng.random_range(0..10)) as u8
        } else if b.is_ascii_uppercase() {
            b'A' + (rng.random_range(0..26)) as u8
        } else if b.is_ascii_lowercase() {
            b'a' + (rng.random_range(0..26)) as u8
        } else {
            b
        };
        out.push(replacement);
    }

    String::from_utf8(out).expect("format-preserving mask produced invalid UTF-8")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Contract, FieldDefinition, FieldType, Ontology, Transform};

    fn entity(name: &str, transform: Option<Transform>) -> FieldDefinition {
        FieldDefinition {
            name: name.to_string(),
            field_type: FieldType::String,
            required: false,
            pattern: None,
            allowed_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            properties: None,
            items: None,
            transform,
        }
    }

    fn compile(
        fields: Vec<FieldDefinition>,
        salt: Vec<u8>,
        compliance_mode: bool,
    ) -> CompiledContract {
        let contract = Contract {
            version: "1.0".into(),
            name: "test".into(),
            description: None,
            compliance_mode,
            ontology: Ontology { entities: fields },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
        };
        CompiledContract::compile_with_salt(contract, salt).unwrap()
    }

    // -- hash ----------------------------------------------------------------

    #[test]
    fn hash_is_deterministic_same_salt_same_input() {
        let cc = compile(
            vec![entity(
                "email",
                Some(Transform {
                    kind: TransformKind::Hash,
                    style: None,
                }),
            )],
            b"salt-A".to_vec(),
            false,
        );
        let a = apply_transforms(&cc, json!({"email": "user@example.com"})).into_inner();
        let b = apply_transforms(&cc, json!({"email": "user@example.com"})).into_inner();
        assert_eq!(a, b);
        let hashed = a.get("email").unwrap().as_str().unwrap();
        assert!(hashed.starts_with("hmac-sha256:"));
        assert_eq!(hashed.len(), "hmac-sha256:".len() + 64);
    }

    #[test]
    fn hash_is_salt_isolated_different_contracts_different_hashes() {
        let a = compile(
            vec![entity(
                "email",
                Some(Transform {
                    kind: TransformKind::Hash,
                    style: None,
                }),
            )],
            b"salt-A".to_vec(),
            false,
        );
        let b = compile(
            vec![entity(
                "email",
                Some(Transform {
                    kind: TransformKind::Hash,
                    style: None,
                }),
            )],
            b"salt-B".to_vec(),
            false,
        );
        let out_a = apply_transforms(&a, json!({"email": "same@example.com"})).into_inner();
        let out_b = apply_transforms(&b, json!({"email": "same@example.com"})).into_inner();
        assert_ne!(out_a, out_b);
    }

    // -- mask ----------------------------------------------------------------

    #[test]
    fn mask_opaque_replaces_with_fixed_sentinel() {
        let cc = compile(
            vec![entity(
                "ssn",
                Some(Transform {
                    kind: TransformKind::Mask,
                    style: Some(MaskStyle::Opaque),
                }),
            )],
            b"any".to_vec(),
            false,
        );
        let out = apply_transforms(&cc, json!({"ssn": "123-45-6789"})).into_inner();
        assert_eq!(out.get("ssn").unwrap().as_str().unwrap(), "****");
    }

    #[test]
    fn mask_format_preserving_preserves_char_class_per_position() {
        let cc = compile(
            vec![entity(
                "phone",
                Some(Transform {
                    kind: TransformKind::Mask,
                    style: Some(MaskStyle::FormatPreserving),
                }),
            )],
            b"salt".to_vec(),
            false,
        );
        let out = apply_transforms(&cc, json!({"phone": "+1 415-555-0199"})).into_inner();
        let masked = out.get("phone").unwrap().as_str().unwrap();

        // Every position's class preserved:
        let original = "+1 415-555-0199";
        assert_eq!(masked.len(), original.len());
        for (orig, new) in original.chars().zip(masked.chars()) {
            if orig.is_ascii_digit() {
                assert!(
                    new.is_ascii_digit(),
                    "digit at position lost class: {orig} -> {new}"
                );
            } else if orig.is_ascii_uppercase() {
                assert!(new.is_ascii_uppercase());
            } else if orig.is_ascii_lowercase() {
                assert!(new.is_ascii_lowercase());
            } else {
                // symbols, whitespace pass through unchanged
                assert_eq!(orig, new, "symbol position was rewritten");
            }
        }
    }

    #[test]
    fn mask_format_preserving_is_deterministic() {
        let cc = compile(
            vec![entity(
                "phone",
                Some(Transform {
                    kind: TransformKind::Mask,
                    style: Some(MaskStyle::FormatPreserving),
                }),
            )],
            b"stable-salt".to_vec(),
            false,
        );
        let a = apply_transforms(&cc, json!({"phone": "+1 415-555-0199"})).into_inner();
        let b = apply_transforms(&cc, json!({"phone": "+1 415-555-0199"})).into_inner();
        assert_eq!(a, b);
    }

    #[test]
    fn mask_defaults_to_opaque_when_style_omitted() {
        let cc = compile(
            vec![entity(
                "ssn",
                Some(Transform {
                    kind: TransformKind::Mask,
                    style: None,
                }),
            )],
            b"any".to_vec(),
            false,
        );
        let out = apply_transforms(&cc, json!({"ssn": "123-45-6789"})).into_inner();
        assert_eq!(out.get("ssn").unwrap().as_str().unwrap(), "****");
    }

    // -- drop / redact -------------------------------------------------------

    #[test]
    fn drop_removes_the_field() {
        let cc = compile(
            vec![entity(
                "debug",
                Some(Transform {
                    kind: TransformKind::Drop,
                    style: None,
                }),
            )],
            b"any".to_vec(),
            false,
        );
        let out = apply_transforms(&cc, json!({"debug": "blob", "keep": "yes"})).into_inner();
        assert!(out.get("debug").is_none(), "dropped field should be absent");
        assert_eq!(out.get("keep").unwrap().as_str().unwrap(), "yes");
    }

    #[test]
    fn redact_replaces_with_sentinel_string() {
        let cc = compile(
            vec![entity(
                "secret",
                Some(Transform {
                    kind: TransformKind::Redact,
                    style: None,
                }),
            )],
            b"any".to_vec(),
            false,
        );
        let out = apply_transforms(&cc, json!({"secret": "hunter2"})).into_inner();
        assert_eq!(out.get("secret").unwrap().as_str().unwrap(), "<REDACTED>");
    }

    // -- identity / compliance_mode -----------------------------------------

    #[test]
    fn no_transforms_is_identity() {
        let cc = compile(vec![entity("plain", None)], b"any".to_vec(), false);
        let input = json!({"plain": "value", "extra": 42});
        let out = apply_transforms(&cc, input.clone()).into_inner();
        assert_eq!(out, input);
    }

    #[test]
    fn compliance_mode_strips_undeclared_fields() {
        let cc = compile(
            vec![entity("declared", None)],
            b"any".to_vec(),
            true, // compliance_mode on
        );
        let out =
            apply_transforms(&cc, json!({"declared": "yes", "stowaway": "leaked"})).into_inner();
        assert_eq!(out.get("declared").unwrap().as_str().unwrap(), "yes");
        assert!(
            out.get("stowaway").is_none(),
            "undeclared field should be stripped under compliance mode"
        );
    }

    #[test]
    fn compliance_mode_off_keeps_undeclared_fields() {
        let cc = compile(vec![entity("declared", None)], b"any".to_vec(), false);
        let out =
            apply_transforms(&cc, json!({"declared": "yes", "stowaway": "kept"})).into_inner();
        assert_eq!(out.get("stowaway").unwrap().as_str().unwrap(), "kept");
    }

    // -- edge cases ----------------------------------------------------------

    #[test]
    fn transform_skipped_when_field_is_absent() {
        let cc = compile(
            vec![entity(
                "optional_email",
                Some(Transform {
                    kind: TransformKind::Hash,
                    style: None,
                }),
            )],
            b"any".to_vec(),
            false,
        );
        // Field not present — should be a no-op, not a panic.
        let out = apply_transforms(&cc, json!({"other": "value"})).into_inner();
        assert!(out.get("optional_email").is_none());
    }

    #[test]
    fn non_object_root_passes_through_untouched() {
        let cc = compile(
            vec![entity(
                "email",
                Some(Transform {
                    kind: TransformKind::Hash,
                    style: None,
                }),
            )],
            b"any".to_vec(),
            false,
        );
        // Root is an array — validator already raised type-mismatch.
        let out = apply_transforms(&cc, json!(["not", "an", "object"])).into_inner();
        assert_eq!(out, json!(["not", "an", "object"]));
    }

    // -- compile-time rejection ---------------------------------------------

    #[test]
    fn compile_rejects_transform_on_non_string_entity() {
        let mut bad = entity(
            "amount",
            Some(Transform {
                kind: TransformKind::Hash,
                style: None,
            }),
        );
        bad.field_type = FieldType::Float;
        let contract = Contract {
            version: "1.0".into(),
            name: "bad".into(),
            description: None,
            compliance_mode: false,
            ontology: Ontology {
                entities: vec![bad],
            },
            glossary: vec![],
            metrics: vec![],
            quality: vec![],
        };
        let err = CompiledContract::compile_with_salt(contract, b"x".to_vec())
            .expect_err("non-string transform should fail compile");
        let msg = format!("{err}");
        assert!(
            msg.contains("transform") && msg.contains("string"),
            "error should mention transform + string constraint, got: {msg}"
        );
    }
}
