//! `GET /v1/contracts/{id}/passed` — cursor page of events that passed validation.
//!
//! Rows come from `forwarded_events`. A Fivetran Connector SDK connector uses
//! the opaque cursor to pull only passing records, oldest first. The cursor
//! is base64url of `{created_at microseconds},{id}`. Ids are random UUIDs, so
//! order is `(created_at, id)`, not id alone.
//!
//! Rows newer than 5 seconds are held back. A cursor that does not decode is
//! 400 and the message contains `invalid cursor`; that is the only 400 on
//! which the connector drops its cursor. A bad `limit` is a different 400.
//! A cursor older than the plan's retention window is 410.

use axum::{
    extract::{Extension, Path, Query, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;
use crate::error::{AppError, AppResult};
use crate::plan;
use crate::storage::{self, ForwardedEventRow};
use crate::AppState;

/// Default page size when `limit` is omitted.
pub const DEFAULT_LIMIT: u32 = 500;
/// Largest page size. Larger requests are capped, not rejected.
pub const MAX_LIMIT: u32 = 1000;
/// Reject cursors longer than this before decoding them.
const MAX_CURSOR_LEN: usize = 256;

/// Stable phrase the connector matches. Other 400s (a bad `limit`) must not
/// contain it, so a limit error does not wipe a stored cursor.
const CURSOR_MALFORMED: &str = "invalid cursor; drop the cursor and re-sync from the start";
const LIMIT_RANGE: &str = "limit must be an integer 1-1000";

#[derive(Debug, Deserialize)]
pub struct PassedQuery {
    pub cursor: Option<String>,
    /// Raw query text. Parsed in [`parse_limit`] so `-1` and `abc` are the
    /// standard JSON 400, not serde's "invalid digit" error.
    pub limit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PassedEvent {
    pub id: Uuid,
    pub contract_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct PassedEventsResponse {
    pub events: Vec<PassedEvent>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

/// Exclusive lower bound used when the caller sent no cursor.
///
/// Every stored row is newer than the Unix epoch and has a v4 id, so
/// `(created_at, id) > (epoch, nil)` is the start of the table.
pub(crate) fn cursor_origin() -> (DateTime<Utc>, Uuid) {
    (DateTime::UNIX_EPOCH, Uuid::nil())
}

/// Cap `limit` without allocating from the raw query value.
///
/// Missing or blank is [`DEFAULT_LIMIT`]. Integers above [`MAX_LIMIT`] become
/// [`MAX_LIMIT`]. Zero, negatives, and non-integers are 400 with
/// [`LIMIT_RANGE`]. The returned value is what the SQL `LIMIT` is derived
/// from (`value + 1` for the look-ahead row).
pub(crate) fn parse_limit(raw: Option<&str>) -> AppResult<u32> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_LIMIT);
    };
    let Ok(n) = text.parse::<i64>() else {
        return Err(AppError::BadRequest(LIMIT_RANGE.into()));
    };
    if n > i64::from(MAX_LIMIT) {
        return Ok(MAX_LIMIT);
    }
    if n < 1 {
        return Err(AppError::BadRequest(LIMIT_RANGE.into()));
    }
    Ok(u32::try_from(n).unwrap_or(MAX_LIMIT))
}

pub(crate) fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> String {
    let raw = format!("{},{id}", created_at.timestamp_micros());
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

fn decode_cursor(token: &str) -> Result<EventCursor, ()> {
    if token.is_empty() || token.len() > MAX_CURSOR_LEN {
        return Err(());
    }
    let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| ())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ())?;
    let (micros, id) = text.split_once(',').ok_or(())?;
    if micros.is_empty() || id.is_empty() {
        return Err(());
    }
    let micros: i64 = micros.parse().map_err(|_| ())?;
    let created_at = DateTime::from_timestamp_micros(micros).ok_or(())?;
    let id = Uuid::parse_str(id).map_err(|_| ())?;
    Ok(EventCursor { created_at, id })
}

fn cursor_expired_message(days: i32) -> String {
    format!(
        "passed-event cursor is older than the {days}-day retention window for this plan; drop the cursor and re-sync from the start"
    )
}

fn retention_start(now: DateTime<Utc>, days: i32) -> DateTime<Utc> {
    let window = Duration::try_days(i64::from(days)).expect("retention days fit in a duration");
    now - window
}

/// Bound for the keyset predicate, plus the cursor to echo on an empty page.
#[derive(Debug)]
pub(crate) struct ResolvedCursor {
    pub after_created_at: DateTime<Utc>,
    pub after_id: Uuid,
    /// Request cursor, returned when this page has no events. `None` when the
    /// caller sent no cursor.
    pub echo: Option<String>,
}

/// `Ok` with the origin bound when `raw` is missing or blank.
/// Malformed → 400. Older than `retention_days` → 410.
/// `retention_days == None` (self-host / dev, no org) does not expire cursors.
pub(crate) fn resolve_cursor(
    raw: Option<&str>,
    now: DateTime<Utc>,
    retention_days: Option<i32>,
) -> AppResult<ResolvedCursor> {
    let Some(token) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        let (after_created_at, after_id) = cursor_origin();
        return Ok(ResolvedCursor {
            after_created_at,
            after_id,
            echo: None,
        });
    };
    let cursor =
        decode_cursor(token).map_err(|()| AppError::BadRequest(CURSOR_MALFORMED.into()))?;
    if let Some(days) = retention_days {
        if cursor.created_at < retention_start(now, days) {
            return Err(AppError::Gone(cursor_expired_message(days)));
        }
    }
    Ok(ResolvedCursor {
        after_created_at: cursor.created_at,
        after_id: cursor.id,
        echo: Some(token.to_string()),
    })
}

#[derive(Debug)]
pub(crate) enum PassedDeny {
    Unauthorized,
    NotFound,
    Forbidden,
}

/// Plan and visibility gate for the passed-event read.
///
/// `contract_visible` is false when the contract is missing, owned by
/// another org, or soft-deleted (`get_contract_identity` returns not found
/// for all three). That is 404, including for a free plan, so the response
/// does not confirm the contract exists. A visible contract on `free` or
/// `aisg_free` is 403. `Ok(None)` is self-host / dev (no org): no plan and
/// no cursor expiry. `Ok(Some(days))` is the retention window.
pub(crate) fn authorize_passed_export(
    auth_configured: bool,
    org_id: Option<Uuid>,
    plan: Option<&str>,
    contract_visible: bool,
    key_permits: bool,
    self_hosted: bool,
) -> Result<Option<i32>, PassedDeny> {
    if auth_configured && org_id.is_none() {
        return Err(PassedDeny::Unauthorized);
    }
    if !contract_visible {
        return Err(PassedDeny::NotFound);
    }
    if !key_permits {
        return Err(PassedDeny::Unauthorized);
    }
    if org_id.is_none() {
        return Ok(None);
    }
    match plan {
        Some(p) if plan::gate_paid(self_hosted, p) => {
            Ok(Some(plan::gate_retention_days(self_hosted, p)))
        }
        _ => Err(PassedDeny::Forbidden),
    }
}

fn deny_error(deny: PassedDeny, contract_id: Uuid) -> AppError {
    match deny {
        PassedDeny::Unauthorized => AppError::Unauthorized,
        PassedDeny::NotFound => AppError::ContractNotFound(contract_id),
        PassedDeny::Forbidden => AppError::Forbidden(plan::passed_events_upgrade_message()),
    }
}

/// `require_api_key` already consumed this request's token and stored the
/// outcome. Checking again would charge twice. The dev no-auth path has no
/// outcome; it uses the key id, or a nil UUID when there is no key.
fn take_rate_limit(
    state: &AppState,
    key: Option<&ValidatedKey>,
    existing: Option<crate::rate_limit::RateLimitOutcome>,
) -> crate::rate_limit::RateLimitOutcome {
    if let Some(outcome) = existing {
        return outcome;
    }
    let (bucket, rps, burst) = match key {
        Some(k) => (k.api_key_id, k.rate_limit_rps, k.rate_limit_burst),
        None => (Uuid::nil(), None, None),
    };
    state.rate_limiter.check(bucket, rps, burst)
}

pub(crate) fn finish_page(
    mut rows: Vec<ForwardedEventRow>,
    limit: usize,
    echo: Option<String>,
) -> PassedEventsResponse {
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    // Always a resume point. `has_more` is the only "keep paging now" flag.
    // A one-page contract still returns the last event's cursor so the next
    // poll does not start over. An empty page repeats the request cursor.
    let next_cursor = match rows.last() {
        Some(row) => Some(encode_cursor(row.created_at, row.id)),
        None => echo,
    };
    let events = rows
        .into_iter()
        .map(|row| PassedEvent {
            id: row.id,
            contract_version: row.contract_version,
            created_at: row.created_at,
            payload: row.payload,
        })
        .collect();
    PassedEventsResponse {
        events,
        next_cursor,
        has_more,
    }
}

/// `GET /v1/contracts/{id}/passed?cursor=&limit=`
///
/// Auth is the v1 router's `x-api-key` middleware. The key's org owns the
/// contract. Paid plans only (`growth`, `aisg_pro`, `enterprise`).
pub async fn list_passed_events(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    Query(query): Query<PassedQuery>,
    key_ext: Option<Extension<ValidatedKey>>,
    rl_ext: Option<Extension<crate::rate_limit::RateLimitOutcome>>,
) -> AppResult<Json<PassedEventsResponse>> {
    // Before auth and the database, so `-1` / `abc` are this 400 and not a
    // later 401/404. The extractor accepts the raw string.
    let limit = parse_limit(query.limit.as_deref())?;
    let key = key_ext.as_ref().map(|Extension(k)| k);
    let rl = take_rate_limit(&state, key, rl_ext.map(|Extension(outcome)| outcome));
    if !rl.allowed {
        return Err(AppError::RateLimitExceeded(rl));
    }

    let org_id = key.map(|k| k.org_id);
    // Same first branch as `authorize_passed_export`: do not load a contract
    // with a missing org while auth is on (`None` skips the org filter).
    if state.auth_configured() && org_id.is_none() {
        return Err(AppError::Unauthorized);
    }

    let contract_visible =
        match storage::get_contract_identity(&state.db, contract_id, org_id).await {
            Ok(_) => true,
            Err(AppError::ContractNotFound(_)) => false,
            Err(e) => return Err(e),
        };
    let plan = if contract_visible {
        match org_id {
            Some(oid) => storage::get_org_plan(&state.db, oid).await?,
            None => None,
        }
    } else {
        None
    };
    let key_permits = key.map(|k| k.permits_contract(contract_id)).unwrap_or(true);
    let retention_days = match authorize_passed_export(
        state.auth_configured(),
        org_id,
        plan.as_deref(),
        contract_visible,
        key_permits,
        state.self_hosted,
    ) {
        Ok(days) => days,
        Err(deny) => return Err(deny_error(deny, contract_id)),
    };

    let resolved = resolve_cursor(query.cursor.as_deref(), Utc::now(), retention_days)?;
    // Look ahead one row for `has_more`. `fetch_n` is at most 1001 and is a
    // SQL parameter, not a buffer size.
    let fetch_n = i64::from(limit) + 1;
    let rows = storage::list_forwarded_events_page(
        &state.db,
        contract_id,
        resolved.after_created_at,
        resolved.after_id,
        fetch_n,
    )
    .await?;
    Ok(Json(finish_page(rows, limit as usize, resolved.echo)))
}

/// OpenAPI operation object for `build_openapi` in `v1_ingest.rs`.
pub(crate) fn openapi_operation() -> Value {
    let upgrade = plan::passed_events_upgrade_message();
    json!({
        "operationId": "list_passed_events",
        "summary": "List passed events for incremental export",
        "description": format!(
            "Cursor page of forwarded_events (records that passed validation), oldest first. \
             DataContractGate Free (free) and AISchemaGate Free (aisg_free) receive 403. \
             {upgrade} Payload is null when event-body storage is off. \
             A cursor that does not decode is 400 and the message contains \"invalid cursor\"; \
             the connector drops its cursor only on that 400. A bad limit is a different 400. \
             A cursor older than plan retention is 410; drop the cursor and re-sync. \
             Rows newer than 5 seconds are held back. next_cursor is always a resume cursor: \
             the last returned event when the page has events (even when has_more is false), \
             or the request cursor when the page is empty (null only if no cursor was sent). \
             has_more alone says whether to keep paging now. \
             Kafka and Kinesis consumers do not write this table."
        ),
        "parameters": [
            {
                "name": "id",
                "in": "path",
                "required": true,
                "schema": {"type": "string", "format": "uuid"}
            },
            {
                "name": "cursor",
                "in": "query",
                "required": false,
                "description": "Opaque base64url cursor from next_cursor. A cursor that does not decode is 400 and the message contains \"invalid cursor\"; drop the stored cursor only on that 400. Older than plan retention → 410.",
                "schema": {"type": "string"}
            },
            {
                "name": "limit",
                "in": "query",
                "required": false,
                "description": format!(
                    "Page size. Default 500. Integers above 1000 are capped at 1000. \
                     Zero, negatives, and non-integers are 400: {LIMIT_RANGE}."
                ),
                "schema": {"type": "integer", "default": 500, "minimum": 1, "maximum": 1000}
            }
        ],
        "responses": {
            "200": {
                "description": "One page of passed events, oldest first",
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object",
                            "required": ["events", "next_cursor", "has_more"],
                            "properties": {
                                "events": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "required": ["id", "contract_version", "created_at", "payload"],
                                        "properties": {
                                            "id": {"type": "string", "format": "uuid"},
                                            "contract_version": {"type": ["string", "null"]},
                                            "created_at": {"type": "string", "format": "date-time"},
                                            "payload": {
                                                "description": "Stored event body, or null when payload storage is off",
                                                "type": ["object", "null"]
                                            }
                                        }
                                    }
                                },
                                "next_cursor": {
                                    "type": ["string", "null"],
                                    "description": "Resume cursor. When this page has events, the cursor of the last returned event, even if has_more is false. When this page is empty, the request cursor, or null if none was sent. has_more alone says whether to keep paging now."
                                },
                                "has_more": {
                                    "type": "boolean",
                                    "description": "True when another page is available now. False does not clear next_cursor."
                                }
                            }
                        }
                    }
                }
            },
            "400": {
                "description": format!(
                    "Invalid request: {CURSOR_MALFORMED} The connector drops its cursor only when this 400 contains \"invalid cursor\". Also returned when {LIMIT_RANGE}; that 400 does not contain \"invalid cursor\", so the cursor is kept."
                ),
                "content": {"application/json": {"schema": error_schema()}}
            },
            "401": {
                "description": "Missing or invalid x-api-key, or the key's contract allow-list does not include this contract",
                "content": {"application/json": {"schema": error_schema()}}
            },
            "403": {
                "description": format!("Forbidden: {upgrade} free and aisg_free are denied."),
                "content": {"application/json": {"schema": error_schema()}}
            },
            "404": {
                "description": "Contract does not exist, is not in the caller's org, or is soft-deleted",
                "content": {"application/json": {"schema": error_schema()}}
            },
            "410": {
                "description": "Cursor is older than the plan retention window. Drop the cursor and re-sync.",
                "content": {"application/json": {"schema": error_schema()}}
            },
            "429": {
                "description": "Per-key rate limit exceeded",
                "content": {"application/json": {"schema": error_schema()}}
            }
        },
        "security": [{"ApiKeyAuth": []}]
    })
}

fn error_schema() -> Value {
    json!({
        "type": "object",
        "required": ["error", "status"],
        "properties": {
            "error": {"type": "string"},
            "status": {"type": "integer"}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, response::IntoResponse};
    use chrono::Duration;
    use serde_json::json;

    /// Holdback in the page query. The SQL text is asserted to use this many
    /// seconds, so the in-memory page and the statement stay on one number.
    const HOLDBACK_SECS: i64 = 5;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn row(id: u128, at: DateTime<Utc>, payload: Value) -> ForwardedEventRow {
        ForwardedEventRow {
            id: Uuid::from_u128(id),
            contract_version: Some("1.0.0".into()),
            created_at: at,
            payload,
        }
    }

    /// In-memory stand-in for [`LIST_FORWARDED_EVENTS_PAGE_SQL`]: hold back
    /// rows newer than 5 seconds, keyset on `(created_at, id)`, ascending.
    fn select_page(
        rows: &[ForwardedEventRow],
        cursor: Option<&str>,
        limit: u32,
        at: DateTime<Utc>,
    ) -> PassedEventsResponse {
        let resolved = resolve_cursor(cursor, at, Some(365)).unwrap();
        let cutoff = at - Duration::seconds(HOLDBACK_SECS);
        let mut eligible = Vec::new();
        for row in rows {
            let after_ts = row.created_at > resolved.after_created_at
                || (row.created_at == resolved.after_created_at && row.id > resolved.after_id);
            if row.created_at <= cutoff && after_ts {
                eligible.push(row.clone());
            }
        }
        eligible.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let fetch_n = usize::try_from(limit).unwrap_or(0).saturating_add(1);
        if eligible.len() > fetch_n {
            eligible.truncate(fetch_n);
        }
        finish_page(eligible, usize::try_from(limit).unwrap_or(0), resolved.echo)
    }

    async fn error_json(err: AppError) -> (axum::http::StatusCode, Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn list_sql_matches_keyset_holdback_and_bound_limit() {
        let sql = storage::LIST_FORWARDED_EVENTS_PAGE_SQL;
        assert!(
            sql.contains(&format!("INTERVAL '{HOLDBACK_SECS} seconds'")),
            "{sql}"
        );
        assert!(sql.contains("(created_at, id) > ($2, $3)"), "{sql}");
        assert!(sql.contains("ORDER BY created_at ASC, id ASC"), "{sql}");
        assert!(sql.contains("LIMIT $4"), "{sql}");
        assert!(!sql.to_ascii_lowercase().contains("limit {"), "{sql}");
    }

    #[test]
    fn cursor_round_trip_is_url_safe() {
        let at = now();
        let id = Uuid::from_u128(0x1234_5678);
        let token = encode_cursor(at, id);
        assert!(!token.contains('+') && !token.contains('/') && !token.contains('='));
        let decoded = decode_cursor(&token).unwrap();
        assert_eq!(decoded.created_at, at);
        assert_eq!(decoded.id, id);
    }

    #[test]
    fn paging_across_pages_follows_created_at_then_id() {
        let at = now();
        let rows = vec![
            row(2, at - Duration::seconds(80), json!({"n": 2})),
            row(9, at - Duration::seconds(50), json!({"n": 9})),
            row(1, at - Duration::seconds(100), json!({"n": 1})),
            row(4, at - Duration::seconds(50), json!({"n": 4})),
            row(5, at - Duration::seconds(10), json!({"n": 5})),
            row(3, at - Duration::seconds(30), json!({"n": 3})),
            row(6, at - Duration::seconds(1), json!({"n": 6})),
            row(7, at, json!({"n": 7})),
        ];
        let mut cursor = None;
        let mut seen = Vec::new();
        for _ in 0..6 {
            let page = select_page(&rows, cursor.as_deref(), 2, at);
            if page.events.is_empty() {
                assert!(!page.has_more);
                assert_eq!(page.next_cursor, cursor);
                break;
            }
            assert!(page.events.len() <= 2);
            let last = page.events.last().unwrap();
            assert_eq!(
                page.next_cursor.as_deref(),
                Some(encode_cursor(last.created_at, last.id).as_str())
            );
            seen.extend(page.events.iter().map(|e| e.id.as_u128()));
            cursor = page.next_cursor;
            if !page.has_more {
                let tail = select_page(&rows, cursor.as_deref(), 2, at);
                assert!(tail.events.is_empty());
                assert!(!tail.has_more);
                assert_eq!(tail.next_cursor, cursor);
                break;
            }
        }
        assert_eq!(seen, vec![1, 2, 4, 9, 3, 5]);
    }

    #[test]
    fn one_page_contract_still_returns_a_resume_cursor() {
        let at = now();
        let old = at - Duration::hours(1);
        let rows = vec![
            row(1, old, json!({"n": 1})),
            row(2, old + Duration::seconds(1), json!({"n": 2})),
        ];
        let page = select_page(&rows, None, 10, at);
        assert_eq!(page.events.len(), 2);
        assert!(!page.has_more);
        let last = page.events.last().unwrap();
        let resume = encode_cursor(last.created_at, last.id);
        assert_eq!(page.next_cursor.as_deref(), Some(resume.as_str()));

        let again = select_page(&rows, page.next_cursor.as_deref(), 10, at);
        assert!(again.events.is_empty());
        assert!(!again.has_more);
        assert_eq!(again.next_cursor, page.next_cursor);

        let empty_start = select_page(&[], None, 10, at);
        assert!(empty_start.events.is_empty());
        assert!(empty_start.next_cursor.is_none());
    }

    #[test]
    fn holdback_keeps_the_cutoff_and_drops_newer_rows() {
        let at = now();
        let cutoff = at - Duration::seconds(HOLDBACK_SECS);
        let rows = vec![
            row(1, cutoff, json!({"edge": true})),
            row(
                2,
                cutoff + Duration::microseconds(1),
                json!({"edge": false}),
            ),
            row(3, at - Duration::seconds(1), json!({"fresh": true})),
        ];
        let page = select_page(&rows, None, 10, at);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].id.as_u128(), 1);
        assert!(!page.has_more);
        let only = &page.events[0];
        assert_eq!(
            page.next_cursor.as_deref(),
            Some(encode_cursor(only.created_at, only.id).as_str())
        );
    }

    #[test]
    fn limit_cap_and_default() {
        assert_eq!(parse_limit(None).unwrap(), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("")).unwrap(), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("1")).unwrap(), 1);
        assert_eq!(parse_limit(Some("1000")).unwrap(), MAX_LIMIT);
        assert_eq!(parse_limit(Some("1001")).unwrap(), MAX_LIMIT);
        assert_eq!(parse_limit(Some(&u32::MAX.to_string())).unwrap(), MAX_LIMIT);
        for bad in ["0", "-1", "abc", "1.5"] {
            let err = parse_limit(Some(bad)).unwrap_err();
            assert!(err.to_string().contains(LIMIT_RANGE), "{bad}: {err}");
        }

        let at = now();
        let old = at - Duration::hours(1);
        let rows: Vec<_> = (1..=1001).map(|i| row(i, old, json!({"i": i}))).collect();
        let capped = parse_limit(Some("50000")).unwrap();
        let first = select_page(&rows, None, capped, at);
        assert_eq!(first.events.len(), 1000);
        assert!(first.has_more);
        assert!(first.next_cursor.is_some());
        let second = select_page(&rows, first.next_cursor.as_deref(), capped, at);
        assert_eq!(second.events.len(), 1);
        assert!(!second.has_more);
        let last = &second.events[0];
        assert_eq!(last.id.as_u128(), 1001);
        assert_eq!(
            second.next_cursor.as_deref(),
            Some(encode_cursor(last.created_at, last.id).as_str())
        );

        let defaulted = select_page(&rows, None, parse_limit(None).unwrap(), at);
        assert_eq!(defaulted.events.len(), DEFAULT_LIMIT as usize);
        assert!(defaulted.has_more);
    }

    #[test]
    fn null_payload_serializes_as_null_and_stored_body_is_kept() {
        let at = now();
        let old = at - Duration::minutes(1);
        let rows = vec![
            row(1, old, Value::Null),
            row(2, old + Duration::seconds(1), json!({"user_id": "u_1"})),
        ];
        let page = select_page(&rows, None, 10, at);
        let body = serde_json::to_value(&page).unwrap();
        assert!(body["events"][0]["payload"].is_null());
        assert_eq!(body["events"][1]["payload"]["user_id"], "u_1");
        assert!(body["events"][0]["contract_version"].is_string());
        assert!(!page.has_more);
        let last = page.events.last().unwrap();
        assert_eq!(
            body["next_cursor"].as_str(),
            Some(encode_cursor(last.created_at, last.id).as_str())
        );
    }

    #[tokio::test]
    async fn bad_cursor_is_400_and_says_resync() {
        let at = now();
        let junk = [
            "@@@",
            &URL_SAFE_NO_PAD.encode("nope"),
            &URL_SAFE_NO_PAD.encode("12,not-a-uuid"),
            &URL_SAFE_NO_PAD.encode("1,2,3"),
            &"a".repeat(MAX_CURSOR_LEN + 1),
        ];
        for token in junk {
            let err = resolve_cursor(Some(token), at, Some(90)).unwrap_err();
            let (status, body) = error_json(err).await;
            assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
            assert_eq!(body["status"], 400);
            let msg = body["error"].as_str().unwrap();
            assert!(msg.contains("invalid cursor"), "{msg}");
            assert!(msg.contains("re-sync"), "{msg}");
            assert!(!msg.contains(LIMIT_RANGE), "{msg}");
        }
        let blank = resolve_cursor(Some("   "), at, Some(90)).unwrap();
        let (origin_at, origin_id) = cursor_origin();
        assert_eq!(blank.after_created_at, origin_at);
        assert_eq!(blank.after_id, origin_id);
        assert!(blank.echo.is_none());
    }

    #[tokio::test]
    async fn bad_limit_is_standard_json_not_a_parse_error() {
        use axum::body::Body;
        use axum::http::Request;
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        async fn probe(Query(q): Query<PassedQuery>) -> AppResult<Json<Value>> {
            let n = parse_limit(q.limit.as_deref())?;
            Ok(Json(json!({ "limit": n })))
        }

        let app = Router::new()
            .route("/passed", get(probe))
            .layer(axum::middleware::from_fn(crate::error::json_error_bodies));

        for raw in ["-1", "abc", "1.5", "0"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::get(format!("/passed?limit={raw}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{raw}");
            assert_eq!(body["status"], 400, "{raw}");
            let msg = body["error"].as_str().unwrap();
            assert!(msg.contains(LIMIT_RANGE), "{raw}: {msg}");
            assert!(
                !msg.to_ascii_lowercase().contains("invalid digit"),
                "{raw}: {msg}"
            );
            assert!(
                !msg.contains("invalid cursor"),
                "{raw}: a limit error must not look like an invalid cursor: {msg}"
            );
        }

        let ok = app
            .clone()
            .oneshot(
                Request::get("/passed?limit=1001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(ok.into_body(), 4096).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["limit"], MAX_LIMIT);
    }

    #[tokio::test]
    async fn cursor_older_than_retention_is_410() {
        let at = now();
        for (plan_name, days) in [("growth", 90), ("aisg_pro", 90), ("enterprise", 365)] {
            assert_eq!(plan::log_retention_days(plan_name), days);
            let stale_at = at - Duration::try_days(i64::from(days) + 1).expect("days");
            let stale = encode_cursor(stale_at, Uuid::from_u128(1));
            let err = resolve_cursor(Some(&stale), at, Some(days)).unwrap_err();
            let (status, body) = error_json(err).await;
            assert_eq!(status, axum::http::StatusCode::GONE, "{plan_name}");
            assert_eq!(body["status"], 410);
            let msg = body["error"].as_str().unwrap();
            assert!(msg.contains("re-sync"), "{msg}");
            assert!(msg.contains(&days.to_string()), "{msg}");

            let on_window = encode_cursor(retention_start(at, days), Uuid::from_u128(1));
            assert!(resolve_cursor(Some(&on_window), at, Some(days)).is_ok());
        }
        let ancient = encode_cursor(
            at - Duration::try_days(4000).expect("days"),
            Uuid::from_u128(1),
        );
        assert!(resolve_cursor(Some(&ancient), at, None).is_ok());
    }

    #[tokio::test]
    async fn free_plan_is_403_with_paid_plan_message() {
        let org = Uuid::new_v4();
        let contract = Uuid::new_v4();
        for plan_name in ["free", "aisg_free", "bogus"] {
            let deny = authorize_passed_export(true, Some(org), Some(plan_name), true, true, false)
                .unwrap_err();
            let (status, body) = error_json(deny_error(deny, contract)).await;
            assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{plan_name}");
            assert_eq!(body["status"], 403);
            let msg = body["error"].as_str().unwrap();
            assert!(msg.starts_with("Forbidden: "), "{msg}");
            assert!(msg.contains(&plan::passed_events_upgrade_message()));
            for paid in plan::PAID_PLANS {
                assert!(msg.contains(paid), "{msg} missing {paid}");
            }
        }
        assert_eq!(
            authorize_passed_export(true, Some(org), Some("growth"), true, true, false).unwrap(),
            Some(90)
        );
        assert_eq!(
            authorize_passed_export(true, Some(org), Some("aisg_pro"), true, true, false).unwrap(),
            Some(90)
        );
        assert_eq!(
            authorize_passed_export(true, Some(org), Some("enterprise"), true, true, false)
                .unwrap(),
            Some(365)
        );
    }

    #[tokio::test]
    async fn self_hosted_free_org_is_allowed_and_hosted_free_is_403() {
        let org = Uuid::new_v4();
        let contract = Uuid::new_v4();
        for plan_name in ["free", "aisg_free"] {
            let allowed =
                authorize_passed_export(true, Some(org), Some(plan_name), true, true, true)
                    .unwrap();
            assert_eq!(allowed, Some(90), "{plan_name}");
            let deny = authorize_passed_export(true, Some(org), Some(plan_name), true, true, false)
                .unwrap_err();
            let (status, _) = error_json(deny_error(deny, contract)).await;
            assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{plan_name}");
        }
    }

    #[tokio::test]
    async fn other_org_is_404() {
        let contract = Uuid::new_v4();
        let caller = Uuid::new_v4();
        // Missing, another org, and soft-deleted all surface as not visible.
        for plan_name in ["free", "aisg_free", "growth", "aisg_pro", "enterprise"] {
            let deny =
                authorize_passed_export(true, Some(caller), Some(plan_name), false, true, false)
                    .unwrap_err();
            let (status, body) = error_json(deny_error(deny, contract)).await;
            assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "{plan_name}");
            assert_eq!(body["status"], 404);
            let msg = body["error"].as_str().unwrap();
            assert!(msg.contains(&contract.to_string()), "{msg}");
            assert!(!msg.contains("Forbidden"), "{msg}");
        }
    }

    #[test]
    fn scoped_key_and_missing_org_are_401_before_the_plan_gate() {
        let org = Uuid::new_v4();
        assert!(matches!(
            authorize_passed_export(true, None, Some("growth"), true, true, false),
            Err(PassedDeny::Unauthorized)
        ));
        assert!(matches!(
            authorize_passed_export(true, Some(org), Some("growth"), true, false, false),
            Err(PassedDeny::Unauthorized)
        ));
        assert!(matches!(
            authorize_passed_export(false, None, None, true, true, false),
            Ok(None)
        ));
    }

    #[tokio::test]
    async fn reuses_middleware_rate_limit_without_a_second_charge() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
            .unwrap();
        let state = AppState::new(pool, false, None, None);
        let key_id = Uuid::new_v4();
        let first = state.rate_limiter.check(key_id, Some(10), Some(20));
        let remaining = first.remaining;
        let reused = take_rate_limit(&state, None, Some(first));
        assert_eq!(reused.remaining, remaining);
        let second = state.rate_limiter.check(key_id, Some(10), Some(20));
        assert!(second.remaining < remaining);
    }
}
