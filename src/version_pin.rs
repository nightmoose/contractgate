//! D1: one parser and one 422 shape for `{contract_id}@{version}` pins on
//! every route that accepts them (`/ingest`, `/v1/ingest`, `/egress`).

use crate::error::{AppError, AppResult};
use crate::validation::{Violation, ViolationKind};
use uuid::Uuid;

/// Machine-readable code on every deprecated-pin 422.
pub const DEPRECATED_CODE: &str = "contract_version_deprecated";

/// Split `<uuid>` or `<uuid>@<version>` into `(uuid, optional_version)`.
/// An empty suffix (`<uuid>@`) means "no pin".
pub fn parse_pinned_id(raw: &str) -> AppResult<(Uuid, Option<String>)> {
    let (uuid_part, version) = match raw.split_once('@') {
        Some((id, v)) if !v.trim().is_empty() => (id, Some(v.trim().to_string())),
        Some((id, _)) => (id, None),
        None => (raw, None),
    };
    let uuid = Uuid::parse_str(uuid_part)
        .map_err(|_| AppError::BadRequest(format!("invalid contract_id: {uuid_part}")))?;
    Ok((uuid, version))
}

/// The message every route uses. `fate` says what happened to the events.
pub fn deprecated_message(version: &str, contract_id: Uuid, fate: &str) -> String {
    format!(
        "Version {version} of contract {contract_id} is deprecated and no longer \
         accepts events ({fate}). Remove the version pin to use the latest stable version, \
         or pin a stable version (e.g. {contract_id}@X.Y or ?version=X.Y)."
    )
}

/// Per-event violation so results are never an empty list on a deprecated pin.
pub fn deprecated_violation(version: &str, latest_stable: Option<&str>) -> Violation {
    Violation {
        field: String::new(),
        message: format!("contract version {version} is deprecated"),
        kind: ViolationKind::ContractVersionDeprecated,
        received: None,
        expected: latest_stable.map(|s| format!("a stable version (latest stable: {s})")),
        suggestion: Some("Remove the version pin or pin a stable version.".into()),
    }
}

/// Normalised form of a numeric version for pin matching: `1`, `1.0` and
/// `1.0.0` all become `[1, 0, 0]`. Anything that is not 1–3 dot-separated
/// unsigned integers (pre-release tags, build metadata, free text) returns
/// `None` and only ever matches exactly.
pub fn normalize_version(v: &str) -> Option<[u64; 3]> {
    let parts: Vec<&str> = v.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut out = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out[i] = p.parse().ok()?;
    }
    Some(out)
}

/// Pick the stored version a pin refers to: an exact string match first,
/// then the single stored version with the same normalised form. Several
/// normalised matches (e.g. both `1.0` and `1.0.0` stored) is ambiguous and
/// returns `Err(candidates)`; no match returns `Ok(None)`.
pub fn match_pin<'a>(pin: &str, stored: &'a [String]) -> Result<Option<&'a str>, Vec<&'a str>> {
    if let Some(v) = stored.iter().find(|v| v.as_str() == pin) {
        return Ok(Some(v.as_str()));
    }
    let Some(want) = normalize_version(pin) else {
        return Ok(None);
    };
    let hits: Vec<&str> = stored
        .iter()
        .filter(|v| normalize_version(v) == Some(want))
        .map(String::as_str)
        .collect();
    match hits.len() {
        0 => Ok(None),
        1 => Ok(Some(hits[0])),
        _ => Err(hits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pads_numeric_versions() {
        assert_eq!(normalize_version("1"), Some([1, 0, 0]));
        assert_eq!(normalize_version("1.0"), Some([1, 0, 0]));
        assert_eq!(normalize_version("1.0.0"), Some([1, 0, 0]));
        assert_eq!(normalize_version("01.2"), Some([1, 2, 0]));
        assert_eq!(normalize_version("1.0.0-beta"), None);
        assert_eq!(normalize_version("1.0.0.0"), None);
        assert_eq!(normalize_version("v1"), None);
        assert_eq!(normalize_version("1..0"), None);
    }

    #[test]
    fn match_pin_exact_then_normalised() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let stored = s(&["1.0", "1.1.0", "2.0.0-rc1"]);
        assert_eq!(match_pin("1.0", &stored), Ok(Some("1.0")));
        assert_eq!(match_pin("1.0.0", &stored), Ok(Some("1.0")));
        assert_eq!(match_pin("1.1", &stored), Ok(Some("1.1.0")));
        assert_eq!(match_pin("2.0.0-rc1", &stored), Ok(Some("2.0.0-rc1")));
        assert_eq!(match_pin("2.0.0", &stored), Ok(None));
        assert_eq!(match_pin("3", &stored), Ok(None));
        // Exact wins even when another stored version normalises the same.
        let both = s(&["1.0", "1.0.0"]);
        assert_eq!(match_pin("1.0.0", &both), Ok(Some("1.0.0")));
        assert_eq!(match_pin("1", &both), Err(vec!["1.0", "1.0.0"]));
    }

    const ID: &str = "7d0c4a8e-1111-4222-8333-444455556666";

    #[test]
    fn parses_plain_pinned_and_empty_suffix() {
        let id = Uuid::parse_str(ID).unwrap();
        assert_eq!(parse_pinned_id(ID).unwrap(), (id, None));
        assert_eq!(
            parse_pinned_id(&format!("{ID}@1.0")).unwrap(),
            (id, Some("1.0".into()))
        );
        assert_eq!(parse_pinned_id(&format!("{ID}@")).unwrap(), (id, None));
    }

    #[test]
    fn bad_uuid_is_400() {
        use axum::response::IntoResponse;
        let r = parse_pinned_id("nope@1.0").unwrap_err().into_response();
        assert_eq!(r.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn violation_serializes_with_code_kind() {
        let v = serde_json::to_value(deprecated_violation("1.0", Some("2.0"))).unwrap();
        assert_eq!(v["kind"], DEPRECATED_CODE);
        assert!(v["expected"].as_str().unwrap().contains("2.0"));
    }

    #[test]
    fn message_names_version_and_fix() {
        let m = deprecated_message("1.0", Uuid::nil(), "every event was quarantined");
        assert!(m.contains("1.0") && m.contains("deprecated") && m.contains("@X.Y"));
    }
}
