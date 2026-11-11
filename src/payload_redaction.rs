//! Strip offending values from stored violation JSON when the record body
//! is not kept.
//!
//! Free (and any other plan that does not store payloads) still records why a
//! row failed. The offending value used to ride along in `received` and, for
//! some kinds, inside `message` (`value "…"`). That is the same data the body
//! redaction is supposed to drop. The sentinel is `"<REDACTED>"`, the same
//! string `redact` transforms write.

use serde_json::Value;

/// Same sentinel as [`crate::transform`] `redact`.
pub const REDACTED_RECEIVED: &str = "<REDACTED>";

/// Copy `details` for a durable write. When `store_payloads` is false, the
/// copy has no offending values. When it is true, the copy is unchanged.
pub fn violation_details_for_storage(details: &Value, store_payloads: bool) -> Value {
    if store_payloads {
        return details.clone();
    }
    let mut redacted = details.clone();
    redact_violation_details(&mut redacted);
    redacted
}

/// Mask `received` and the `value …` clause in `message` on every object in
/// a violation array. A non-array (or a null `received`) is left alone.
/// Calling this twice is a no-op the second time.
pub fn redact_violation_details(details: &mut Value) {
    let Some(items) = details.as_array_mut() else {
        return;
    };
    for item in items {
        redact_one(item);
    }
}

fn redact_one(item: &mut Value) {
    let Some(obj) = item.as_object_mut() else {
        return;
    };
    let Some(received) = obj.get("received").cloned() else {
        return;
    };
    if received.is_null() {
        return;
    }
    if let Some(message) = obj.get("message").and_then(Value::as_str) {
        let scrubbed = scrub_value_clause(message, &received);
        obj.insert("message".into(), Value::String(scrubbed));
    }
    obj.insert(
        "received".into(),
        Value::String(REDACTED_RECEIVED.to_string()),
    );
}

/// Replace the first `value <received>` clause. Later copies of the same
/// token (an allowed-set that happens to contain it) stay.
fn scrub_value_clause(message: &str, received: &Value) -> String {
    let mut phrases = received_phrases(received);
    phrases.sort_by_key(|phrase| std::cmp::Reverse(phrase.len()));
    phrases.dedup();
    for phrase in phrases {
        if phrase.is_empty() {
            continue;
        }
        if let Some(scrubbed) = replace_value_clause(message, &phrase) {
            return scrubbed;
        }
    }
    message.to_string()
}

fn received_phrases(received: &Value) -> Vec<String> {
    let mut phrases = Vec::new();
    let json = received.to_string();
    phrases.push(format!("{json:?}"));
    phrases.push(json);
    match received {
        Value::String(text) => {
            phrases.push(format!("{text:?}"));
            phrases.push(text.clone());
        }
        Value::Number(number) => {
            if let Some(n) = number.as_i64() {
                phrases.push(n.to_string());
            }
            if let Some(n) = number.as_u64() {
                phrases.push(n.to_string());
            }
            if let Some(n) = number.as_f64() {
                phrases.push(n.to_string());
            }
        }
        Value::Bool(flag) => phrases.push(flag.to_string()),
        _ => {}
    }
    phrases
}

fn replace_value_clause(message: &str, phrase: &str) -> Option<String> {
    let needle = format!("value {phrase}");
    let mut start = 0;
    while let Some(rel) = message[start..].find(&needle) {
        let idx = start + rel;
        let end = idx + needle.len();
        let boundary = message[end..].chars().next();
        let boundary_ok = match boundary {
            None => true,
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' => false,
            Some(_) => true,
        };
        if boundary_ok {
            let mut out = String::with_capacity(message.len() + REDACTED_RECEIVED.len());
            out.push_str(&message[..idx]);
            out.push_str("value ");
            out.push_str(REDACTED_RECEIVED);
            out.push_str(&message[end..]);
            return Some(out);
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_received_and_its_message_clause_are_masked() {
        let mut details = json!([{
            "field": "pickup_datetime",
            "kind": "pattern_mismatch",
            "message": "Field 'pickup_datetime' value \"01/01/2019 00:30\" does not match required pattern",
            "received": "01/01/2019 00:30",
            "expected": "match regex /^\\d{4}-/"
        }]);
        redact_violation_details(&mut details);
        assert_eq!(details[0]["received"], REDACTED_RECEIVED);
        let message = details[0]["message"].as_str().unwrap();
        assert!(message.contains("value <REDACTED>"), "{message}");
        assert!(!message.contains("01/01/2019"), "{message}");
        assert_eq!(details[0]["field"], "pickup_datetime");
        assert_eq!(details[0]["expected"], "match regex /^\\d{4}-/");
    }

    #[test]
    fn enum_value_does_not_eat_the_allowed_set() {
        let mut details = json!([{
            "field": "payment_type",
            "kind": "enum_violation",
            "message": "Field 'payment_type' value 1 not in allowed set: [1, 2, 3]",
            "received": 1,
            "expected": "one of [1, 2, 3]"
        }]);
        redact_violation_details(&mut details);
        assert_eq!(details[0]["received"], REDACTED_RECEIVED);
        assert_eq!(
            details[0]["message"],
            "Field 'payment_type' value <REDACTED> not in allowed set: [1, 2, 3]"
        );
        assert_eq!(details[0]["expected"], "one of [1, 2, 3]");
    }

    #[test]
    fn numeric_prefix_does_not_split_a_longer_number() {
        let message = scrub_value_clause(
            "Field 'fare_amount' value -5.5 is below minimum 0",
            &json!(-5.5),
        );
        assert_eq!(
            message,
            "Field 'fare_amount' value <REDACTED> is below minimum 0"
        );
        assert!(!message.contains("-5.5"));
    }

    #[test]
    fn null_received_and_missing_key_stay() {
        let mut details = json!([
            {"field": "vendorid", "kind": "missing_required_field", "message": "Required field 'vendorid' is missing", "received": null},
            {"field": "total", "kind": "missing_required_field", "message": "Required field 'total' is missing"}
        ]);
        redact_violation_details(&mut details);
        assert!(details[0]["received"].is_null());
        assert_eq!(
            details[0]["message"],
            "Required field 'vendorid' is missing"
        );
        assert!(details[1].get("received").is_none());
    }

    #[test]
    fn second_pass_is_stable_and_storage_keeps_paid_bodies() {
        let original = json!([{"field": "note", "message": "Field 'note' value \"secret\" does not match required pattern", "received": "secret"}]);
        assert_eq!(violation_details_for_storage(&original, true), original);
        let stored = violation_details_for_storage(&original, false);
        let mut again = stored.clone();
        redact_violation_details(&mut again);
        assert_eq!(again, stored);
        assert_eq!(stored[0]["received"], REDACTED_RECEIVED);
        assert!(!stored[0]["message"].as_str().unwrap().contains("secret"));
    }

    #[test]
    fn non_array_is_unchanged() {
        let mut details = json!({"received": "secret"});
        redact_violation_details(&mut details);
        assert_eq!(details["received"], "secret");
    }
}
