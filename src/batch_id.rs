//! Optional run id for ingest and egress.
//!
//! A caller stamps one pipeline run on `POST /v1/ingest/{contract_id}`,
//! `POST /ingest/{contract_id}`, and `POST /egress/{contract_id}`. The id is
//! stored on the quarantine rows that call creates. `GET /quarantine/summary`
//! counts those rows so dbt or Airflow can fail the run.
//!
//! The header is the form to send. Airflow's default `run_id` contains `+`,
//! and a query string decodes `+` as a space unless the client percent-encodes
//! it. Query `batch_id` / `run_id` are accepted as aliases.

use axum::http::HeaderMap;

use crate::error::{AppError, AppResult};

/// Maximum length, in Unicode scalar values. Matches the column check.
pub const MAX_BATCH_ID_LEN: usize = 255;

const HEADER_BATCH: &str = "x-batch-id";
const HEADER_RUN: &str = "x-run-id";

/// Resolve the optional run id from the query aliases and the headers.
///
/// Accepted spellings, with no priority among them:
/// - query `batch_id`
/// - query `run_id`
/// - header `X-Batch-Id`
/// - header `X-Run-Id`
///
/// A blank value is ignored. Every non-blank value must be the same string
/// after trimming, or the request is rejected. A value longer than
/// [`MAX_BATCH_ID_LEN`] characters, or one that contains an ASCII control
/// character, is rejected.
pub fn resolve_batch_id(
    query_batch_id: Option<&str>,
    query_run_id: Option<&str>,
    headers: &HeaderMap,
) -> AppResult<Option<String>> {
    let mut chosen: Option<String> = None;
    for raw in [
        query_batch_id,
        query_run_id,
        header_str(headers, HEADER_BATCH),
        header_str(headers, HEADER_RUN),
    ]
    .into_iter()
    .flatten()
    {
        let Some(value) = normalize_batch_id(raw)? else {
            continue;
        };
        match &chosen {
            None => chosen = Some(value),
            Some(existing) if existing == &value => {}
            Some(_) => {
                return Err(AppError::BadRequest(
                    "batch_id and run_id must match when more than one is set".into(),
                ));
            }
        }
    }
    Ok(chosen)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn normalize_batch_id(raw: &str) -> AppResult<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_BATCH_ID_LEN {
        return Err(AppError::BadRequest(format!(
            "batch_id must be {MAX_BATCH_ID_LEN} characters or fewer"
        )));
    }
    if value.chars().any(|c| c.is_ascii_control()) {
        return Err(AppError::BadRequest(
            "batch_id must not contain control characters".into(),
        ));
    }
    Ok(Some(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn absent_when_nothing_is_set() {
        let id = resolve_batch_id(None, None, &HeaderMap::new()).unwrap();
        assert!(id.is_none());
    }

    #[test]
    fn blank_values_are_absent() {
        let id =
            resolve_batch_id(Some("  "), Some(""), &headers(&[("x-batch-id", "   ")])).unwrap();
        assert!(id.is_none());
    }

    #[test]
    fn header_is_the_run_id() {
        let id = resolve_batch_id(
            None,
            None,
            &headers(&[("x-batch-id", "scheduled__2026-11-09T00:00:00+00:00")]),
        )
        .unwrap();
        assert_eq!(id.as_deref(), Some("scheduled__2026-11-09T00:00:00+00:00"));
    }

    #[test]
    fn run_id_header_and_query_are_aliases() {
        let from_header = resolve_batch_id(None, None, &headers(&[("x-run-id", "inv-1")])).unwrap();
        let from_query = resolve_batch_id(None, Some("inv-1"), &HeaderMap::new()).unwrap();
        let from_batch = resolve_batch_id(Some("  inv-1  "), None, &HeaderMap::new()).unwrap();
        assert_eq!(from_header.as_deref(), Some("inv-1"));
        assert_eq!(from_query, from_header);
        assert_eq!(from_batch, from_header);
    }

    #[test]
    fn agreeing_spellings_collapse_to_one_id() {
        let id = resolve_batch_id(
            Some("dbt-inv"),
            Some("dbt-inv"),
            &headers(&[("x-batch-id", "dbt-inv"), ("x-run-id", "dbt-inv")]),
        )
        .unwrap();
        assert_eq!(id.as_deref(), Some("dbt-inv"));
    }

    #[test]
    fn disagreeing_spellings_are_rejected() {
        let err = resolve_batch_id(
            Some("from-query"),
            None,
            &headers(&[("x-run-id", "from-header")]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("must match"), "{err}");
    }

    #[test]
    fn query_aliases_that_differ_are_rejected() {
        let err = resolve_batch_id(Some("a"), Some("b"), &HeaderMap::new()).unwrap_err();
        assert!(err.to_string().contains("must match"), "{err}");
    }

    #[test]
    fn overlong_id_is_rejected() {
        let raw = "x".repeat(MAX_BATCH_ID_LEN + 1);
        let err = resolve_batch_id(Some(&raw), None, &HeaderMap::new()).unwrap_err();
        assert!(err.to_string().contains("255"), "{err}");
    }

    #[test]
    fn max_length_is_accepted() {
        let raw = "x".repeat(MAX_BATCH_ID_LEN);
        let id = resolve_batch_id(Some(&raw), None, &HeaderMap::new()).unwrap();
        assert_eq!(id.as_deref(), Some(raw.as_str()));
    }

    #[test]
    fn control_characters_are_rejected() {
        let err = resolve_batch_id(Some("run\nid"), None, &HeaderMap::new()).unwrap_err();
        assert!(err.to_string().contains("control"), "{err}");
    }
}
