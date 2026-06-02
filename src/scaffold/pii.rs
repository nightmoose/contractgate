//! PII field detection for the brownfield scaffolder (RFC-024 §D).
//!
//! Two-signal scoring:
//!   - Signal 1 (weight 0.6): field-name match against a curated list.
//!   - Signal 2 (weight 0.4): regex match against sampled string values.
//!
//! Confidence threshold for emitting a TODO: 0.4 (configurable).
//! Auto-apply of transforms is PERMANENTLY BLOCKED regardless of confidence.
//!
//! Developer tooling — not part of the patent-core validation engine.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// PII name list (Signal 1)
// ---------------------------------------------------------------------------

/// Canonical PII field-name tokens.  The detector splits snake_case /
/// camelCase field names into tokens and checks each token against this set.
static PII_NAME_TOKENS: &[&str] = &[
    // Identity
    "email",
    "ssn",
    "social",
    "security",
    "passport",
    "license",
    "nid",
    // Names
    "firstname",
    "lastname",
    "fullname",
    "username",
    "name",
    // Contact
    "phone",
    "mobile",
    "fax",
    "address",
    "street",
    "city",
    "zip",
    "postal",
    "postcode",
    // Financial
    "creditcard",
    "cardnumber",
    "cvv",
    "iban",
    "bankaccount",
    "routing",
    // Auth
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "privatekey",
    // Location / dates
    "dob",
    "birthdate",
    "birthday",
    "latitude",
    "longitude",
    "geolocation",
    // Network
    "ipaddress",
    "ip",
    "mac",
    "deviceid",
    "imei",
    // Biometric
    "fingerprint",
    "faceprint",
    "voiceprint",
];

/// Exact-match tokens that score at full 1.0 on Signal 1 (zero ambiguity).
static PII_EXACT_TOKENS: &[&str] = &[
    "email",
    "ssn",
    "password",
    "passwd",
    "creditcard",
    "cvv",
    "iban",
];

// ---------------------------------------------------------------------------
// PII value regex patterns (Signal 2)
// ---------------------------------------------------------------------------

struct PiiPattern {
    name: &'static str,
    re: Regex,
}

static PII_PATTERNS: Lazy<Vec<PiiPattern>> = Lazy::new(|| {
    vec![
        PiiPattern {
            name: "email",
            re: Regex::new(r"(?i)[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}").unwrap(),
        },
        PiiPattern {
            name: "us_ssn",
            re: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
        },
        PiiPattern {
            name: "us_phone",
            re: Regex::new(r"\b(\+1\s?)?\(?\d{3}\)?[\s\-]\d{3}[\s\-]\d{4}\b").unwrap(),
        },
        PiiPattern {
            name: "credit_card",
            re: Regex::new(
                r"\b(?:4\d{12}(?:\d{3})?|5[1-5]\d{14}|3[47]\d{13}|6(?:011|5\d{2})\d{12})\b",
            )
            .unwrap(),
        },
        PiiPattern {
            name: "ip_address",
            re: Regex::new(
                r"\b(?:(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\.){3}(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\b",
            )
            .unwrap(),
        },
        // Low-confidence UUID hint (might be a user ID or session ID).
        PiiPattern {
            name: "uuid_possible_id",
            re: Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
                .unwrap(),
        },
    ]
});

// Per-pattern confidence contribution to Signal 2.
fn pattern_confidence(name: &str) -> f32 {
    match name {
        "email" | "us_ssn" | "credit_card" => 1.0,
        "us_phone" => 0.8,
        "ip_address" => 0.6,
        "uuid_possible_id" => 0.15, // many UUIDs are just IDs, not PII per se
        _ => 0.5,
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// A PII detection hit for a single field.
#[derive(Debug, Clone)]
pub struct PiiCandidate {
    pub field_name: String,
    /// Combined confidence in [0.0, 1.0].
    pub confidence: f32,
    /// Human-readable explanation for the YAML comment.
    pub reason: String,
    /// Suggested transform kind string (for the TODO comment).
    pub suggested_transform: &'static str,
}

// ---------------------------------------------------------------------------
// Detector
// ---------------------------------------------------------------------------

/// Detect PII candidates across a set of field names and sampled event values.
///
/// `samples` may be empty — Signal 1 (name matching) fires independently.
/// `threshold` is the minimum confidence to emit a `PiiCandidate`.
pub fn detect_pii(field_names: &[String], samples: &[Value], threshold: f32) -> Vec<PiiCandidate> {
    // Pre-build an index: field name → list of sampled string values.
    let mut field_values: HashMap<String, Vec<String>> = HashMap::new();
    for event in samples {
        if let Some(obj) = event.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    field_values
                        .entry(k.clone())
                        .or_default()
                        .push(s.to_string());
                }
            }
        }
    }

    let mut candidates = Vec::new();

    for field in field_names {
        let (sig1, sig1_reason) = name_signal(field);
        let (sig2, sig2_reason) = value_signal(field, &field_values);

        let confidence = sig1 * 0.6 + sig2 * 0.4;
        if confidence < threshold {
            continue;
        }

        let mut reason_parts = Vec::new();
        if !sig1_reason.is_empty() {
            reason_parts.push(format!("field_name:{}", sig1_reason));
        }
        if !sig2_reason.is_empty() {
            reason_parts.push(format!("value_match:{}", sig2_reason));
        }

        // Suggest a transform based on the detected PII type.
        let suggested_transform = suggest_transform(&sig1_reason, &sig2_reason);

        candidates.push(PiiCandidate {
            field_name: field.clone(),
            confidence,
            reason: reason_parts.join(" + "),
            suggested_transform,
        });
    }

    // Sort by confidence descending.
    candidates.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    candidates
}

/// Signal 1: name-based scoring.
fn name_signal(field_name: &str) -> (f32, String) {
    let tokens = tokenize_field_name(field_name);

    // Exact match on any token against PII_EXACT_TOKENS → score 1.0.
    for tok in &tokens {
        if PII_EXACT_TOKENS.contains(&tok.as_str()) {
            return (1.0, tok.clone());
        }
    }

    // Substring match against PII_NAME_TOKENS.
    let field_lower = field_name.to_lowercase();
    let mut best_score = 0.0f32;
    let mut best_match = String::new();
    for &pii_tok in PII_NAME_TOKENS {
        let full_token_match = tokens.iter().any(|t| t == pii_tok);
        if field_lower.contains(pii_tok) || full_token_match {
            let score = pii_tok.len() as f32 / field_lower.len().max(1) as f32;
            // Full token match: floor at 0.75 so compound names (e.g. "phone_number")
            // are not penalised by unrelated suffixes.
            let score = if full_token_match {
                score.max(0.75)
            } else {
                score
            };
            let score = score.min(0.9); // cap at 0.9 — only exact tokens reach 1.0
            if score > best_score {
                best_score = score;
                best_match = pii_tok.to_string();
            }
        }
    }

    (best_score, best_match)
}

/// Signal 2: value-based regex scoring.
fn value_signal(_field_name: &str, field_values: &HashMap<String, Vec<String>>) -> (f32, String) {
    let values = match field_values.get(_field_name) {
        Some(v) if !v.is_empty() => v,
        _ => return (0.0, String::new()),
    };

    let total = values.len() as f32;
    let mut best_score = 0.0f32;
    let mut best_name = String::new();

    for pattern in PII_PATTERNS.iter() {
        let matches = values.iter().filter(|v| pattern.re.is_match(v)).count();
        let hit_rate = matches as f32 / total;
        if hit_rate >= 0.10 {
            let score = pattern_confidence(pattern.name) * hit_rate.min(1.0);
            if score > best_score {
                best_score = score;
                best_name = pattern.name.to_string();
            }
        }
    }

    (best_score, best_name)
}

fn suggest_transform(name_reason: &str, value_reason: &str) -> &'static str {
    let combined = format!("{} {}", name_reason, value_reason).to_lowercase();
    if combined.contains("password") || combined.contains("passwd") || combined.contains("secret") {
        "drop"
    } else if combined.contains("ssn")
        || combined.contains("us_ssn")
        || combined.contains("credit_card")
    {
        "redact"
    } else {
        "hash"
    }
}

/// Split a snake_case / camelCase / PascalCase field name into lowercase tokens.
fn tokenize_field_name(name: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();

    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == '.' {
            if !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
        } else if ch.is_uppercase() && !current.is_empty() {
            tokens.push(current.to_lowercase());
            current.clear();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }
    tokens
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> String {
        name.to_string()
    }

    // --- Signal 1: name list ---

    #[test]
    fn detects_email_field_name() {
        let c = detect_pii(&[field("email")], &[], 0.4);
        assert!(!c.is_empty(), "email field should be detected");
        assert!(c[0].confidence >= 0.4);
    }

    #[test]
    fn detects_user_email_camel_case() {
        let c = detect_pii(&[field("userEmail")], &[], 0.4);
        assert!(!c.is_empty());
    }

    #[test]
    fn detects_ssn_field_name() {
        let c = detect_pii(&[field("ssn")], &[], 0.4);
        assert!(!c.is_empty());
        assert!(c[0].confidence >= 0.6);
    }

    #[test]
    fn detects_phone_number_snake_case() {
        let c = detect_pii(&[field("phone_number")], &[], 0.4);
        assert!(!c.is_empty());
    }

    #[test]
    fn non_pii_fields_below_threshold() {
        let fields: Vec<String> = ["amount", "timestamp", "event_type", "product_id"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = detect_pii(&fields, &[], 0.4);
        assert!(c.is_empty(), "non-PII fields should not trigger: {c:?}");
    }

    // --- Signal 2: regex ---

    #[test]
    fn email_regex_fires_on_email_values() {
        let samples = vec![
            serde_json::json!({"contact": "alice@example.com"}),
            serde_json::json!({"contact": "bob@corp.org"}),
            serde_json::json!({"contact": "carol@test.io"}),
        ];
        let c = detect_pii(&[field("contact")], &samples, 0.1);
        assert!(!c.is_empty(), "email values should raise PII on 'contact'");
    }

    #[test]
    fn ssn_regex_true_positives() {
        let pats = &PII_PATTERNS;
        let ssn_re = pats.iter().find(|p| p.name == "us_ssn").unwrap();
        for val in &["123-45-6789", "000-12-3456", "987-65-4321"] {
            assert!(ssn_re.re.is_match(val), "should match SSN: {val}");
        }
    }

    #[test]
    fn ssn_regex_true_negatives() {
        let pats = &PII_PATTERNS;
        let ssn_re = pats.iter().find(|p| p.name == "us_ssn").unwrap();
        for val in &["not-a-ssn", "123456789", "2026-05-06", "hello world"] {
            assert!(!ssn_re.re.is_match(val), "should NOT match SSN: {val}");
        }
    }

    #[test]
    fn email_regex_true_positives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "email").unwrap();
        for val in &["user@example.com", "x.y+z@sub.domain.org", "TEST@CAPS.IO"] {
            assert!(re.re.is_match(val), "should match email: {val}");
        }
    }

    #[test]
    fn email_regex_true_negatives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "email").unwrap();
        for val in &["not-an-email", "missing@", "@nodomain", "2026-05-06"] {
            assert!(!re.re.is_match(val), "should NOT match email: {val}");
        }
    }

    #[test]
    fn credit_card_regex_true_positives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "credit_card").unwrap();
        // Visa (16 digit), Amex (15 digit)
        for val in &["4111111111111111", "4111111111111", "378282246310005"] {
            assert!(re.re.is_match(val), "should match CC: {val}");
        }
    }

    #[test]
    fn credit_card_regex_true_negatives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "credit_card").unwrap();
        for val in &["1234", "99999999999999999", "not-a-number"] {
            assert!(!re.re.is_match(val), "should NOT match CC: {val}");
        }
    }

    #[test]
    fn ip_address_regex_true_positives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "ip_address").unwrap();
        for val in &["192.168.1.1", "10.0.0.1", "255.255.255.0"] {
            assert!(re.re.is_match(val), "should match IP: {val}");
        }
    }

    #[test]
    fn ip_address_regex_true_negatives() {
        let pats = &PII_PATTERNS;
        let re = pats.iter().find(|p| p.name == "ip_address").unwrap();
        for val in &["999.1.1.1", "not-an-ip", "2026-05-06"] {
            assert!(!re.re.is_match(val), "should NOT match IP: {val}");
        }
    }

    #[test]
    fn tokenizer_handles_snake_case() {
        let toks = tokenize_field_name("user_email_address");
        assert_eq!(toks, vec!["user", "email", "address"]);
    }

    #[test]
    fn tokenizer_handles_camel_case() {
        let toks = tokenize_field_name("userEmailAddress");
        assert_eq!(toks, vec!["user", "email", "address"]);
    }
}
