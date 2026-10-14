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

#[cfg(test)]
mod tests {
    use super::*;

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
