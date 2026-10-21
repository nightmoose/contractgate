//! `POST /v1/integrations/fivetran/connect`
//!
//! Uploads the vendored DataContractGate connector into the caller's own Fivetran
//! account and creates a paused Connector SDK connection. The Fivetran API key
//! and secret are used for that one request and are not stored. A new
//! DataContractGate API key, named `Fivetran connector`, is minted for the
//! connection and sent as a Fivetran secret.

use axum::{extract::State, Json};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::api_key_auth::ValidatedKey;
use crate::error::{AppError, AppResult, DbOpContext};
use crate::plan;
use crate::{storage, AppState};

/// Pinned source. `vendor/fivetran-connector/PIN` records the same commit.
#[cfg(test)]
const CONNECTOR_COMMIT: &str = "3f53aa21f4fd71fcf4092d70aa080a847811993c";
const CONNECTOR_PY: &str = include_str!("../vendor/fivetran-connector/connector.py");
const CONNECTOR_REQUIREMENTS: &str = include_str!("../vendor/fivetran-connector/requirements.txt");
#[cfg(test)]
const CONNECTOR_PIN: &str = include_str!("../vendor/fivetran-connector/PIN");

const FIVETRAN_API: &str = "https://api.fivetran.com";
const PACKAGE_PATH: &str = "/v1/connector-sdk/packages";
const CONNECTIONS_PATH: &str = "/v1/connections";
const CONNECTORS_PATH: &str = "/v1/connectors";
const USER_AGENT: &str = "contractgate-fivetran-connect/0.4.0";
const PYTHON_VERSION: &str = "3.12";
const KEY_NAME: &str = "Fivetran connector";

const MAX_CONNECT_BODY: usize = 64 * 1024;
const MAX_SECRET_LEN: usize = 256;
const MIN_SECRET_LEN: usize = 8;
const MAX_GROUP_ID_LEN: usize = 128;
const MAX_CONTRACTS: usize = 32;
const MAX_TABLE_NAMES_LEN: usize = 2_048;
const MAX_TABLE_NAME_LEN: usize = 63;
/// First 8 hex digits of the contract id, appended so default names stay unique.
const TABLE_ID_SUFFIX_HEX: usize = 8;
const MAX_SLUG_LEN: usize = MAX_TABLE_NAME_LEN - TABLE_ID_SUFFIX_HEX - 1;
const MAX_UPSTREAM_BODY: usize = 65_536;
/// `FIVETRAN_NAMING` inserts an underscore at every letter/digit boundary, so
/// `user_events_427c3304` becomes `user_events_427_c_3304`. `SOURCE_NAMING`
/// keeps the name the connector declares.
const DESTINATION_SCHEMA_NAMES: &str = "SOURCE_NAMING";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

const INSERT_KEY_SQL: &str = "\
INSERT INTO api_keys (user_id, org_id, name, key_prefix, key_hash, allowed_contract_ids) \
VALUES ($1, $2, 'Fivetran connector', $3, $4, $5) \
RETURNING id";

// ---------------------------------------------------------------------------
// ZIP of the vendored connector (store method, no compression)
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// ZIP the vendored connector the way `fivetran package` does: code in, no
/// `configuration.json`. `configuration_form.pb` is the empty sentinel the
/// SDK CLI writes when it does not bundle a setup form.
fn connector_zip() -> Vec<u8> {
    let files: [(&str, &[u8]); 3] = [
        ("connector.py", CONNECTOR_PY.as_bytes()),
        ("requirements.txt", CONNECTOR_REQUIREMENTS.as_bytes()),
        ("configuration_form.pb", b""),
    ];
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        let offset = u32::try_from(zip.len()).expect("zip offset fits u32");
        let crc = crc32(data);
        let size = u32::try_from(data.len()).expect("zip entry fits u32");
        push_u32(&mut zip, 0x0403_4b50);
        push_u16(&mut zip, 20);
        push_u16(&mut zip, 0);
        push_u16(&mut zip, 0);
        push_u16(&mut zip, 0);
        push_u16(&mut zip, 0);
        push_u32(&mut zip, crc);
        push_u32(&mut zip, size);
        push_u32(&mut zip, size);
        push_u16(&mut zip, u16::try_from(name_bytes.len()).expect("name len"));
        push_u16(&mut zip, 0);
        zip.extend_from_slice(name_bytes);
        zip.extend_from_slice(data);

        push_u32(&mut central, 0x0201_4b50);
        push_u16(&mut central, 20);
        push_u16(&mut central, 20);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, crc);
        push_u32(&mut central, size);
        push_u32(&mut central, size);
        push_u16(
            &mut central,
            u16::try_from(name_bytes.len()).expect("name len"),
        );
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }
    let central_offset = u32::try_from(zip.len()).expect("central offset");
    let central_size = u32::try_from(central.len()).expect("central size");
    let count = u16::try_from(files.len()).expect("file count");
    zip.extend_from_slice(&central);
    push_u32(&mut zip, 0x0605_4b50);
    push_u16(&mut zip, 0);
    push_u16(&mut zip, 0);
    push_u16(&mut zip, count);
    push_u16(&mut zip, count);
    push_u32(&mut zip, central_size);
    push_u32(&mut zip, central_offset);
    push_u16(&mut zip, 0);
    zip
}

fn multipart_file(zip: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("cgfivetran{}", Uuid::new_v4().simple());
    let header = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"contractgate.zip\"\r\nContent-Type: application/zip\r\n\r\n"
    );
    let mut body = Vec::new();
    body.extend_from_slice(header.as_bytes());
    body.extend_from_slice(zip);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Replace every secret, longest first, so a shorter secret cannot leave a
/// longer one visible. Needles shorter than [`MIN_SECRET_LEN`] are ignored.
fn redact(message: &str, secrets: &[&str]) -> String {
    let mut needles: Vec<&str> = secrets
        .iter()
        .copied()
        .filter(|secret| secret.len() >= MIN_SECRET_LEN)
        .collect();
    needles.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    needles.dedup();
    let mut out = message.to_string();
    for secret in needles {
        let mut start = 0;
        while let Some(rel) = out[start..].find(secret) {
            let at = start + rel;
            out.replace_range(at..at + secret.len(), "[redacted]");
            start = at + "[redacted]".len();
        }
    }
    out
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

struct HeldSecrets {
    fivetran_key: String,
    fivetran_secret: String,
    basic: String,
    cg_key: String,
    combined: String,
}

impl HeldSecrets {
    fn needles(&self) -> Vec<&str> {
        let mut needles = vec![
            self.fivetran_key.as_str(),
            self.fivetran_secret.as_str(),
            self.basic.as_str(),
            self.combined.as_str(),
        ];
        if self.cg_key.len() >= MIN_SECRET_LEN {
            needles.push(self.cg_key.as_str());
        }
        let token = self
            .basic
            .strip_prefix("Basic ")
            .unwrap_or(self.basic.as_str());
        if token.len() >= MIN_SECRET_LEN {
            needles.push(token);
        }
        needles
    }
}

impl std::fmt::Debug for HeldSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HeldSecrets([redacted])")
    }
}

fn basic_header(key: &str, secret: &str) -> (String, String) {
    let mut combined = String::new();
    combined.push_str(key);
    combined.push(':');
    combined.push_str(secret);
    let header = format!("Basic {}", B64.encode(combined.as_bytes()));
    (header, combined)
}

fn log_upstream(status: u16, body: &str, secrets: &HeldSecrets) {
    let detail = truncate_chars(&redact(&upstream_message(body), &secrets.needles()), 180);
    tracing::warn!(status, detail = %detail, "fivetran request failed");
}

fn upstream_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string())
}

fn upstream_error(status: u16, body: &str, secrets: &HeldSecrets) -> AppError {
    log_upstream(status, body, secrets);
    if status == 401 {
        AppError::FivetranUnauthorized
    } else {
        AppError::FivetranUpstream(format!("Fivetran request failed ({status})"))
    }
}

// ---------------------------------------------------------------------------
// Request parsing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ConnectInput {
    group_id: String,
    contract_ids: Vec<Uuid>,
    /// Comma-separated `contract_id=name` pairs. Empty until defaults are filled in.
    table_names: String,
    /// Caller-chosen destination schema. `None` keeps the random default.
    destination_schema: Option<String>,
    secrets: HeldSecrets,
}

fn parse_connect_body(bytes: &[u8]) -> AppResult<ConnectInput> {
    if bytes.len() > MAX_CONNECT_BODY {
        return Err(AppError::PayloadTooLarge(format!(
            "Connect Fivetran body exceeds {MAX_CONNECT_BODY} bytes"
        )));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| AppError::BadRequest("invalid JSON body".into()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::BadRequest("body must be a JSON object".into()))?;
    let fivetran_key = required_credential(obj, "fivetran_api_key", false)?;
    let fivetran_secret = required_credential(obj, "fivetran_api_secret", true)?;
    let group_id = required_group_id(obj)?;
    let contract_ids = required_contract_ids(obj)?;
    let table_names = optional_table_names(obj, &contract_ids)?;
    let destination_schema = optional_destination_schema(obj)?;
    let (basic, combined) = basic_header(&fivetran_key, &fivetran_secret);
    Ok(ConnectInput {
        group_id,
        contract_ids,
        table_names,
        destination_schema,
        secrets: HeldSecrets {
            fivetran_key,
            fivetran_secret,
            basic,
            cg_key: String::new(),
            combined,
        },
    })
}

fn required_credential(
    obj: &serde_json::Map<String, Value>,
    field: &str,
    allow_colon: bool,
) -> AppResult<String> {
    let value = obj
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest(format!("{field} is required")))?;
    if value.len() < MIN_SECRET_LEN || value.len() > MAX_SECRET_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be {MIN_SECRET_LEN}-{MAX_SECRET_LEN} characters"
        )));
    }
    if !value.chars().all(|c| c.is_ascii_graphic()) || (!allow_colon && value.contains(':')) {
        return Err(AppError::BadRequest(format!(
            "{field} has a character that cannot be sent"
        )));
    }
    Ok(value.to_string())
}

fn required_group_id(obj: &serde_json::Map<String, Value>) -> AppResult<String> {
    let value = obj
        .get("group_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("group_id is required".into()))?;
    if value.is_empty()
        || value.len() > MAX_GROUP_ID_LEN
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "group_id must be 1-128 letters, digits, underscores, or hyphens".into(),
        ));
    }
    Ok(value.to_string())
}

fn required_contract_ids(obj: &serde_json::Map<String, Value>) -> AppResult<Vec<Uuid>> {
    let list = obj
        .get("contract_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::BadRequest("contract_ids must be an array of UUID strings".into())
        })?;
    if list.is_empty() {
        return Err(AppError::BadRequest(
            "contract_ids must include at least one contract".into(),
        ));
    }
    if list.len() > MAX_CONTRACTS {
        return Err(AppError::BadRequest(format!(
            "contract_ids exceeds {MAX_CONTRACTS}"
        )));
    }
    let mut ids = Vec::new();
    for item in list {
        let Some(text) = item.as_str() else {
            return Err(AppError::BadRequest(
                "contract_ids must be an array of UUID strings".into(),
            ));
        };
        let Ok(id) = Uuid::parse_str(text) else {
            return Err(AppError::BadRequest(
                "contract_ids contains a value that is not a UUID".into(),
            ));
        };
        if ids.contains(&id) {
            return Err(AppError::BadRequest(
                "contract_ids lists the same contract more than once".into(),
            ));
        }
        if ids.len() == MAX_CONTRACTS {
            return Err(AppError::BadRequest(format!(
                "contract_ids exceeds {MAX_CONTRACTS}"
            )));
        }
        ids.push(id);
    }
    Ok(ids)
}

fn optional_table_names(
    obj: &serde_json::Map<String, Value>,
    contract_ids: &[Uuid],
) -> AppResult<String> {
    let Some(raw) = obj.get("table_names") else {
        return Ok(String::new());
    };
    if raw.is_null() {
        return Ok(String::new());
    }
    let Some(text) = raw.as_str() else {
        return Err(AppError::BadRequest("table_names must be a string".into()));
    };
    if text.chars().count() > MAX_TABLE_NAMES_LEN {
        return Err(AppError::BadRequest(format!(
            "table_names exceeds {MAX_TABLE_NAMES_LEN} characters"
        )));
    }
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    normalize_table_names(text, contract_ids)
}

/// Default destination table: a slug of the contract name plus the first 8
/// hex digits of its id, for example `user_events_427c3304`.
///
/// The connector (`table_name_for`) would otherwise declare
/// `passed_<uuid with hyphens turned into underscores>`. Connect sends an
/// explicit `table_names` entry for every contract, so the connector uses
/// this name instead. `SOURCE_NAMING` then keeps it, including the hex suffix.
fn default_table_name(contract_name: &str, id: &Uuid) -> String {
    let suffix = id_suffix(id);
    let slug = slug_prefix(contract_name);
    if slug.is_empty() {
        format!("passed_{suffix}")
    } else {
        format!("{slug}_{suffix}")
    }
}

fn id_suffix(id: &Uuid) -> String {
    let bytes = id.as_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}

/// Lowercase letters, digits, and underscores. Leading digits and symbols
/// are dropped so the result can start a table name. The buffer is a fixed
/// size, not the length of `name`.
fn slug_prefix(name: &str) -> String {
    let mut slug = String::with_capacity(MAX_SLUG_LEN);
    let mut separator = false;
    for ch in name.chars() {
        if slug.len() >= MAX_SLUG_LEN {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            let lower = ch.to_ascii_lowercase();
            if slug.is_empty() {
                if !lower.is_ascii_lowercase() {
                    continue;
                }
            } else if separator {
                if slug.len() + 1 >= MAX_SLUG_LEN {
                    break;
                }
                slug.push('_');
            }
            separator = false;
            if slug.len() >= MAX_SLUG_LEN {
                break;
            }
            slug.push(lower);
        } else if !slug.is_empty() {
            separator = true;
        }
    }
    slug
}

/// Replace caller overrides and fill every other contract with
/// [`default_table_name`]. The string is the full list the connector reads,
/// so an omitted contract does not fall back to `passed_<uuid>`.
fn assign_table_names(explicit: &str, contracts: &[(Uuid, String)]) -> AppResult<String> {
    let overrides = explicit_table_pairs(explicit)?;
    let mut used: Vec<String> = Vec::new();
    let mut out = String::new();
    for (index, (id, name)) in contracts.iter().enumerate() {
        if index == MAX_CONTRACTS {
            break;
        }
        let table = overrides
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, table)| table.clone())
            .unwrap_or_else(|| default_table_name(name, id));
        if used.contains(&table) {
            return Err(AppError::BadRequest(
                "table_names maps two contracts to the same table".into(),
            ));
        }
        used.push(table.clone());
        if index > 0 {
            out.push(',');
        }
        out.push_str(&id.to_string());
        out.push('=');
        out.push_str(&table);
    }
    Ok(out)
}

fn explicit_table_pairs(explicit: &str) -> AppResult<Vec<(Uuid, String)>> {
    if explicit.is_empty() {
        return Ok(Vec::new());
    }
    let mut pairs = Vec::new();
    for entry in explicit.split(',') {
        let Some((raw_id, name)) = entry.split_once('=') else {
            return Err(AppError::BadRequest(
                "table_names entry must be contract_id=name".into(),
            ));
        };
        let Ok(id) = Uuid::parse_str(raw_id) else {
            return Err(AppError::BadRequest(
                "table_names contains a contract id that is not a UUID".into(),
            ));
        };
        if pairs.len() == MAX_CONTRACTS {
            return Err(AppError::BadRequest(format!(
                "table_names exceeds {MAX_CONTRACTS}"
            )));
        }
        pairs.push((id, name.to_string()));
    }
    Ok(pairs)
}

fn optional_destination_schema(obj: &serde_json::Map<String, Value>) -> AppResult<Option<String>> {
    let Some(raw) = obj.get("destination_schema") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(text) = raw.as_str() else {
        return Err(AppError::BadRequest(
            "destination_schema must be a string".into(),
        ));
    };
    if !valid_destination_schema(text) {
        return Err(AppError::BadRequest(
            "destination_schema must start with a lowercase letter and contain only lowercase letters, digits, and underscores, at most 63 characters"
                .into(),
        ));
    }
    Ok(Some(text.to_string()))
}

fn valid_destination_schema(name: &str) -> bool {
    let len = name.len();
    if len == 0 || len > 63 {
        return false;
    }
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn valid_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first == '_' => {}
        _ => return false,
    }
    !name.is_empty()
        && name.chars().count() <= MAX_TABLE_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Same pair rules as the connector: `contract_id=name`. Explicit names must
/// be unique against each other. Defaults are filled in later, once contract
/// names are known, and checked again there.
fn normalize_table_names(text: &str, contract_ids: &[Uuid]) -> AppResult<String> {
    let mut mapping: Vec<(Uuid, String)> = Vec::new();
    for part in text.split(',') {
        let entry = part.trim();
        if entry.is_empty() {
            return Err(AppError::BadRequest(
                "table_names contains an empty entry".into(),
            ));
        }
        let Some((raw_id, name)) = entry.split_once('=') else {
            return Err(AppError::BadRequest(
                "table_names entry must be contract_id=name".into(),
            ));
        };
        let name = name.trim();
        let Ok(id) = Uuid::parse_str(raw_id.trim()) else {
            return Err(AppError::BadRequest(
                "table_names contains a contract id that is not a UUID".into(),
            ));
        };
        if !contract_ids.contains(&id) {
            return Err(AppError::BadRequest(
                "table_names lists a contract id that is not in contract_ids".into(),
            ));
        }
        if mapping.iter().any(|(seen, _)| *seen == id) {
            return Err(AppError::BadRequest(
                "table_names lists the same contract more than once".into(),
            ));
        }
        if !valid_table_name(name) {
            return Err(AppError::BadRequest(
                "table name must start with a letter or underscore and use only lowercase letters, numbers, and underscores"
                    .into(),
            ));
        }
        if mapping.len() == MAX_CONTRACTS {
            return Err(AppError::BadRequest(format!(
                "table_names exceeds {MAX_CONTRACTS}"
            )));
        }
        mapping.push((id, name.to_string()));
    }
    let mut seen_names: Vec<&str> = Vec::new();
    for (_, name) in &mapping {
        if seen_names.contains(&name.as_str()) {
            return Err(AppError::BadRequest(
                "table_names maps two contracts to the same table".into(),
            ));
        }
        if seen_names.len() == MAX_CONTRACTS {
            break;
        }
        seen_names.push(name);
    }
    let mut out = String::new();
    for (i, (id, name)) in mapping.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&id.to_string());
        out.push('=');
        out.push_str(name);
    }
    Ok(out)
}

fn missing_contract(requested: &[Uuid], owned: &[Uuid]) -> Option<Uuid> {
    requested.iter().copied().find(|id| !owned.contains(id))
}

fn ensure_paid(self_hosted: bool, plan: &str) -> AppResult<()> {
    if plan::gate_paid(self_hosted, plan) {
        Ok(())
    } else {
        Err(AppError::Forbidden(plan::fivetran_connect_upgrade_message()))
    }
}

/// Hosted calls have a caller. `plan == None` is a missing org row (401).
/// An unpaid plan is 403 unless `self_hosted` (`CONTRACTGATE_SELF_HOSTED`).
/// `caller == None` is auth off: no org, same allowance as passed-event export.
fn connect_allowed(
    self_hosted: bool,
    caller: Option<&ValidatedKey>,
    plan: Option<&str>,
) -> AppResult<()> {
    match caller {
        None => Ok(()),
        Some(_) => match plan {
            Some(plan) => ensure_paid(self_hosted, plan),
            None => Err(AppError::Unauthorized),
        },
    }
}

/// Auth on and no caller is 401 before any plan check. Auth off with no
/// caller is the self-host path `connect_allowed` permits.
fn require_connect_caller(auth_configured: bool, has_caller: bool) -> AppResult<()> {
    if auth_configured && !has_caller {
        Err(AppError::Unauthorized)
    } else {
        Ok(())
    }
}

fn parse_public_base_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(trimmed).map_err(|_| origin_error())?;
    let Some(host) = url.host_str() else {
        return Err(origin_error());
    };
    let path_ok = url.path().is_empty() || url.path() == "/";
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !path_ok
    {
        return Err(origin_error());
    }
    Ok(match url.port() {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    })
}

fn origin_error() -> AppError {
    AppError::Misconfigured(
        "CANONICAL_API_ORIGIN must be an https origin with no path, query, or credentials".into(),
    )
}

const ORIGIN_UNSET: &str = "CANONICAL_API_ORIGIN is unset. Set it to this deployment's public https API origin before connecting Fivetran.";

/// `raw` is the `CANONICAL_API_ORIGIN` value, or `None` when the variable is
/// missing. Blank is missing. There is no production fallback: a staging
/// connect must not point the connector at prod.
fn public_base_url_from(raw: Option<&str>) -> AppResult<String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => parse_public_base_url(raw),
        None => Err(AppError::Misconfigured(ORIGIN_UNSET.into())),
    }
}

pub(crate) fn public_base_url() -> AppResult<String> {
    let raw = std::env::var(crate::error::CANONICAL_API_ORIGIN_ENV).ok();
    public_base_url_from(raw.as_deref())
}

// ---------------------------------------------------------------------------
// DataContractGate API key (same shape as dashboard/app/api/keys/route.ts)
// ---------------------------------------------------------------------------

fn mint_raw_key() -> String {
    let mut bytes = [0u8; 24];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    format!("cg_live_{}", hex::encode(bytes))
}

fn hash_key(raw: &str) -> String {
    B64.encode(Sha256::digest(raw.as_bytes()))
}

trait ConnectorKeys: Send + Sync {
    fn insert<'a>(
        &'a self,
        user_id: Uuid,
        org_id: Uuid,
        prefix: &'a str,
        hash: &'a str,
        contracts: &'a [Uuid],
    ) -> BoxFuture<'a, AppResult<Uuid>>;

    fn revoke<'a>(&'a self, org_id: Uuid, key_id: Uuid) -> BoxFuture<'a, AppResult<()>>;
}

struct DbKeys<'a> {
    db: &'a sqlx::PgPool,
}

impl ConnectorKeys for DbKeys<'_> {
    fn insert<'a>(
        &'a self,
        user_id: Uuid,
        org_id: Uuid,
        prefix: &'a str,
        hash: &'a str,
        contracts: &'a [Uuid],
    ) -> BoxFuture<'a, AppResult<Uuid>> {
        Box::pin(async move {
            let (id,): (Uuid,) = sqlx::query_as(INSERT_KEY_SQL)
                .bind(user_id)
                .bind(org_id)
                .bind(prefix)
                .bind(hash)
                .bind(contracts)
                .fetch_one(self.db)
                .await
                .db_op("fivetran_insert_key")?;
            Ok(id)
        })
    }

    fn revoke<'a>(&'a self, org_id: Uuid, key_id: Uuid) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL",
            )
            .bind(key_id)
            .bind(org_id)
            .execute(self.db)
            .await
            .db_op("fivetran_revoke_key")?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Fivetran HTTP
// ---------------------------------------------------------------------------

struct HttpReply {
    status: u16,
    body: String,
}

trait ConnectHttp: Send + Sync {
    fn post_multipart<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
        zip: &'a [u8],
    ) -> BoxFuture<'a, Result<HttpReply, String>>;

    fn post_json<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
        body: &'a Value,
    ) -> BoxFuture<'a, Result<HttpReply, String>>;

    fn delete<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
    ) -> BoxFuture<'a, Result<HttpReply, String>>;

    fn get<'a>(&'a self, path: &'a str, auth: &'a str) -> BoxFuture<'a, Result<HttpReply, String>>;
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

struct ReqwestFivetran {
    client: reqwest::Client,
    needles: Vec<String>,
}

impl ReqwestFivetran {
    fn needles(&self) -> Vec<&str> {
        self.needles.iter().map(String::as_str).collect()
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<HttpReply, String> {
        let response = request.send().await.map_err(|err| {
            let safe = redact(&err.to_string(), &self.needles());
            tracing::warn!(detail = %truncate_chars(&safe, 180), "fivetran transport error");
            "Fivetran request failed".to_string()
        })?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        let mut response = response;
        loop {
            let chunk = response.chunk().await.map_err(|err| {
                let safe = redact(&err.to_string(), &self.needles());
                tracing::warn!(detail = %truncate_chars(&safe, 180), "fivetran read error");
                "Fivetran request failed".to_string()
            })?;
            let Some(chunk) = chunk else {
                break;
            };
            if body.len().saturating_add(chunk.len()) > MAX_UPSTREAM_BODY {
                return Err("Fivetran request failed".to_string());
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        Ok(HttpReply { status, body: text })
    }
}

impl ConnectHttp for ReqwestFivetran {
    fn post_multipart<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
        zip: &'a [u8],
    ) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            let (content_type, body) = multipart_file(zip);
            let request = self
                .client
                .post(format!("{FIVETRAN_API}{path}"))
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .body(body);
            self.send(request).await
        })
    }

    fn post_json<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
        body: &'a Value,
    ) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            let request = self
                .client
                .post(format!("{FIVETRAN_API}{path}"))
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .json(body);
            self.send(request).await
        })
    }

    fn delete<'a>(
        &'a self,
        path: &'a str,
        auth: &'a str,
    ) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            let request = self
                .client
                .delete(format!("{FIVETRAN_API}{path}"))
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::USER_AGENT, USER_AGENT);
            self.send(request).await
        })
    }

    fn get<'a>(&'a self, path: &'a str, auth: &'a str) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            let request = self
                .client
                .get(format!("{FIVETRAN_API}{path}"))
                .header(reqwest::header::AUTHORIZATION, auth)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::USER_AGENT, USER_AGENT);
            self.send(request).await
        })
    }
}

fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|_| AppError::FivetranUpstream("Fivetran request failed".into()))
}

fn connection_schema() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("contractgate_{}", &hex[..8])
}

fn secret_entry(key: &str, value: &str) -> Value {
    json!({"key": key, "value": value})
}

fn connection_body(
    group_id: &str,
    schema: &str,
    package_id: &str,
    input: &ConnectInput,
    base_url: &str,
) -> Value {
    let contract_ids = input
        .contract_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    json!({
        "group_id": group_id,
        "service": "connector_sdk",
        "paused": true,
        "run_setup_tests": true,
        "sync_frequency": 360,
        "destination_schema_names": DESTINATION_SCHEMA_NAMES,
        "config": {
            "schema": schema,
            "package_id": package_id,
            "python_version": PYTHON_VERSION,
            "secrets_list": [
                secret_entry("api_key", &input.secrets.cg_key),
                secret_entry("contract_ids", &contract_ids),
                secret_entry("base_url", base_url),
                secret_entry("table_names", &input.table_names),
            ]
        }
    })
}

fn data_id(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let id = value.get("data")?.get("id")?.as_str()?;
    if safe_id(id) {
        Some(id.to_string())
    } else {
        None
    }
}

fn connection_status(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return "paused".to_string();
    };
    let data = value.get("data");
    let failed = data
        .and_then(|item| item.get("setup_tests"))
        .and_then(Value::as_array)
        .is_some_and(|tests| {
            tests.iter().any(|test| {
                matches!(
                    test.get("status").and_then(Value::as_str),
                    Some("FAILED" | "JOB_FAILED")
                )
            })
        });
    if failed {
        return "setup_failed".to_string();
    }
    let status = data
        .and_then(|item| item.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("paused");
    if safe_id(status) && status.len() <= 32 {
        status.to_string()
    } else {
        "paused".to_string()
    }
}

fn setup_url(connection_id: &str) -> Option<String> {
    if safe_id(connection_id) {
        Some(format!(
            "https://fivetran.com/dashboard/connections/{connection_id}/status"
        ))
    } else {
        None
    }
}

fn group_path(group_id: &str) -> String {
    format!("/v1/groups/{group_id}")
}

/// Confirms the Fivetran key and `group_id` before a DataContractGate key is minted.
async fn verify_group<H: ConnectHttp + ?Sized>(http: &H, input: &ConnectInput) -> AppResult<()> {
    let path = group_path(&input.group_id);
    let reply = http
        .get(&path, &input.secrets.basic)
        .await
        .map_err(|err| transport(err, &input.secrets))?;
    if (200..300).contains(&reply.status) {
        return Ok(());
    }
    if reply.status == 404 {
        log_upstream(reply.status, &reply.body, &input.secrets);
        return Err(AppError::BadRequest(format!(
            "Fivetran group {} was not found",
            input.group_id
        )));
    }
    Err(upstream_error(reply.status, &reply.body, &input.secrets))
}

fn transport(err: String, secrets: &HeldSecrets) -> AppError {
    let detail = truncate_chars(&redact(&err, &secrets.needles()), 180);
    tracing::warn!(detail = %detail, "fivetran request failed");
    AppError::FivetranUpstream("Fivetran request failed".into())
}

async fn upload_package<H: ConnectHttp + ?Sized>(
    http: &H,
    input: &ConnectInput,
    zip: &[u8],
) -> AppResult<String> {
    let reply = http
        .post_multipart(PACKAGE_PATH, &input.secrets.basic, zip)
        .await
        .map_err(|err| transport(err, &input.secrets))?;
    if !(200..300).contains(&reply.status) {
        return Err(upstream_error(reply.status, &reply.body, &input.secrets));
    }
    data_id(&reply.body).ok_or_else(|| {
        log_upstream(reply.status, &reply.body, &input.secrets);
        AppError::FivetranUpstream("Fivetran returned an unexpected package id".into())
    })
}

async fn create_connection<H: ConnectHttp + ?Sized>(
    http: &H,
    input: &ConnectInput,
    package_id: &str,
    base_url: &str,
) -> AppResult<(String, String)> {
    let schema = input
        .destination_schema
        .clone()
        .unwrap_or_else(connection_schema);
    let body = connection_body(&input.group_id, &schema, package_id, input, base_url);
    let first = http
        .post_json(CONNECTIONS_PATH, &input.secrets.basic, &body)
        .await
        .map_err(|err| transport(err, &input.secrets))?;
    let reply = if first.status == 404 {
        tracing::info!("fivetran POST /v1/connections returned 404; trying /v1/connectors");
        http.post_json(CONNECTORS_PATH, &input.secrets.basic, &body)
            .await
            .map_err(|err| transport(err, &input.secrets))?
    } else {
        first
    };
    if !(200..300).contains(&reply.status) {
        return Err(upstream_error(reply.status, &reply.body, &input.secrets));
    }
    let id = data_id(&reply.body).ok_or_else(|| {
        log_upstream(reply.status, &reply.body, &input.secrets);
        AppError::FivetranUpstream("Fivetran returned an unexpected connection id".into())
    })?;
    let status = connection_status(&reply.body);
    Ok((id, status))
}

async fn delete_package<H: ConnectHttp + ?Sized>(http: &H, input: &ConnectInput, package_id: &str) {
    if !safe_id(package_id) {
        return;
    }
    let path = format!("/v1/connector-sdk/packages/{package_id}");
    match http.delete(&path, &input.secrets.basic).await {
        Ok(reply) if (200..300).contains(&reply.status) => {
            tracing::info!("deleted unused fivetran package");
        }
        Ok(reply) => {
            tracing::warn!(
                status = reply.status,
                "failed to delete unused fivetran package"
            );
        }
        Err(err) => {
            let detail = truncate_chars(&redact(&err, &input.secrets.needles()), 180);
            tracing::warn!(detail = %detail, "failed to delete unused fivetran package");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_connect<H: ConnectHttp + ?Sized, K: ConnectorKeys + ?Sized>(
    http: &H,
    keys: &K,
    self_hosted: bool,
    caller: Option<&ValidatedKey>,
    plan: Option<&str>,
    owned: &[Uuid],
    mut input: ConnectInput,
    base_url: &str,
) -> AppResult<ConnectResponse> {
    connect_allowed(self_hosted, caller, plan)?;
    if let Some(id) = missing_contract(&input.contract_ids, owned) {
        return Err(AppError::ContractNotFound(id));
    }
    let base_url = public_base_url_from(Some(base_url))?;
    verify_group(http, &input).await?;
    let raw = mint_raw_key();
    // Hosted: persist a key scoped to the caller's org and the selected
    // contracts. Self-host has no user and no org. `api_keys.user_id` and
    // `org_id` are NOT NULL, and key lookup joins `orgs`, so there is no row
    // to insert. Auth is off on that instance, so `GET /passed` does not look
    // the key up. The connector still sends this key.
    let key_id = if let Some(caller) = caller {
        let prefix = &raw[..12];
        let hash = hash_key(&raw);
        Some(
            keys.insert(
                caller.user_id,
                caller.org_id,
                prefix,
                &hash,
                &input.contract_ids,
            )
            .await?,
        )
    } else {
        None
    };
    input.secrets.cg_key = raw;

    let zip = connector_zip();
    let package_id = match upload_package(http, &input, &zip).await {
        Ok(id) => id,
        Err(err) => {
            if let (Some(caller), Some(key_id)) = (caller, key_id) {
                revoke_quiet(keys, caller.org_id, key_id).await;
            }
            return Err(err);
        }
    };
    match create_connection(http, &input, &package_id, &base_url).await {
        Ok((connection_id, status)) => {
            tracing::info!(connection_id = %connection_id, "fivetran connection created");
            Ok(ConnectResponse {
                connection_id: connection_id.clone(),
                setup_url: setup_url(&connection_id),
                status,
            })
        }
        Err(err) => {
            delete_package(http, &input, &package_id).await;
            if let (Some(caller), Some(key_id)) = (caller, key_id) {
                revoke_quiet(keys, caller.org_id, key_id).await;
            }
            Err(err)
        }
    }
}

async fn revoke_quiet<K: ConnectorKeys + ?Sized>(keys: &K, org_id: Uuid, key_id: Uuid) {
    if keys.revoke(org_id, key_id).await.is_err() {
        tracing::warn!("failed to revoke unused Fivetran connector key");
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ConnectResponse {
    pub connection_id: String,
    pub setup_url: Option<String>,
    pub status: String,
}

/// `POST /v1/integrations/fivetran/connect`
pub async fn connect_fivetran_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> AppResult<Json<ConnectResponse>> {
    let caller = req.extensions().get::<ValidatedKey>().cloned();
    require_connect_caller(state.auth_configured(), caller.is_some())?;
    let bytes = axum::body::to_bytes(req.into_body(), MAX_CONNECT_BODY)
        .await
        .map_err(|_| {
            AppError::PayloadTooLarge(format!(
                "Connect Fivetran body exceeds {MAX_CONNECT_BODY} bytes"
            ))
        })?;
    let mut input = parse_connect_body(&bytes)?;
    // Resolve the public origin before a key is minted. Unset used to send
    // the connector to production.
    let base_url = public_base_url()?;
    // No caller is self-host / auth off: there is no org, so there is no plan.
    // A caller whose org row is missing stays 401. Hosted free stays 403.
    let plan = match &caller {
        Some(caller) => Some(
            storage::get_org_plan(&state.db, caller.org_id)
                .await?
                .ok_or(AppError::Unauthorized)?,
        ),
        None => None,
    };
    connect_allowed(state.self_hosted, caller.as_ref(), plan.as_deref())?;
    let org_scope = caller.as_ref().map(|caller| caller.org_id);
    let mut contracts = Vec::new();
    for id in &input.contract_ids {
        let identity = storage::get_contract_identity(&state.db, *id, org_scope).await?;
        if contracts.len() == MAX_CONTRACTS {
            return Err(AppError::BadRequest(format!(
                "contract_ids exceeds {MAX_CONTRACTS}"
            )));
        }
        contracts.push((identity.id, identity.name));
    }
    input.table_names = assign_table_names(&input.table_names, &contracts)?;
    let owned: Vec<Uuid> = contracts.iter().map(|(id, _)| *id).collect();
    let client = http_client()?;
    let http = ReqwestFivetran {
        client,
        needles: input
            .secrets
            .needles()
            .into_iter()
            .map(str::to_string)
            .collect(),
    };
    let keys = DbKeys { db: &state.db };
    let response = finish_connect(
        &http,
        &keys,
        state.self_hosted,
        caller.as_ref(),
        plan.as_deref(),
        &owned,
        input,
        &base_url,
    )
    .await?;
    Ok(Json(response))
}

pub(crate) fn openapi_operation() -> Value {
    let upgrade = plan::fivetran_connect_upgrade_message();
    json!({
        "operationId": "connect_fivetran",
        "summary": "Deploy the DataContractGate Fivetran connector into the caller's Fivetran account",
        "description": format!(
            "Paid plans only ({upgrade}). CONTRACTGATE_SELF_HOSTED=true treats the org as paid, including a free plan, and keeps auth on. The Fivetran API key and secret are used once to upload the connector package and create a paused connection, and are not stored. On a hosted org, DataContractGate mints a new API key named \"{KEY_NAME}\" for the connection and sends it as a Fivetran secret. base_url is CANONICAL_API_ORIGIN and the call fails with 500 when that variable is unset. Default table names are a slug of the contract name plus the first 8 hex digits of its id. The slug keeps ASCII letters and digits only, so Café becomes caf_<8hex>. destination_schema_names is SOURCE_NAMING so Fivetran keeps those names. An explicit table_names entry replaces the default for that contract."
        ),
        "requestBody": {
            "required": true,
            "content": {
                "application/json": {
                    "schema": {
                        "type": "object",
                        "required": ["fivetran_api_key", "fivetran_api_secret", "group_id", "contract_ids"],
                        "properties": {
                            "fivetran_api_key": {"type": "string", "minLength": MIN_SECRET_LEN, "maxLength": MAX_SECRET_LEN},
                            "fivetran_api_secret": {"type": "string", "minLength": MIN_SECRET_LEN, "maxLength": MAX_SECRET_LEN},
                            "group_id": {"type": "string", "minLength": 1, "maxLength": MAX_GROUP_ID_LEN},
                            "contract_ids": {
                                "type": "array",
                                "minItems": 1,
                                "maxItems": MAX_CONTRACTS,
                                "items": {"type": "string", "format": "uuid"}
                            },
                            "table_names": {"type": "string", "maxLength": MAX_TABLE_NAMES_LEN},
                            "destination_schema": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 63,
                                "pattern": "^[a-z][a-z0-9_]{0,62}$",
                                "description": "Destination schema. Omitted values use contractgate_<8 hex>."
                            }
                        }
                    }
                }
            }
        },
        "responses": {
            "200": {
                "description": "Connection created. It stays paused until the initial sync is started in Fivetran.",
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object",
                            "required": ["connection_id", "status"],
                            "properties": {
                                "connection_id": {"type": "string"},
                                "setup_url": {"type": ["string", "null"]},
                                "status": {"type": "string"}
                            }
                        }
                    }
                }
            },
            "400": {"description": "Invalid body, contract id, table name, or destination_schema. The error text starts with \"Invalid request: \". An unknown group is \"Invalid request: Fivetran group <id> was not found\"."},
            "413": {"description": "Body exceeds 65536 bytes (64 KiB): \"Payload too large: Connect Fivetran body exceeds 65536 bytes\". No DataContractGate key is minted."},
            "500": {"description": "CANONICAL_API_ORIGIN is unset or is not an https origin. No DataContractGate key is minted."},
            "401": {"description": "Missing DataContractGate credentials, or Fivetran rejected the API key"},
            "403": {"description": upgrade},
            "404": {"description": "A contract id is not in the caller's org"},
            "502": {"description": "Fivetran returned an error. The body does not include API keys."}
        },
        "security": [{"ApiKeyAuth": []}]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use std::sync::Mutex;

    fn caller() -> ValidatedKey {
        ValidatedKey {
            api_key_id: Uuid::nil(),
            user_id: Uuid::from_u128(1),
            org_id: Uuid::from_u128(2),
            allowed_contract_ids: None,
            rate_limit_rps: None,
            rate_limit_burst: None,
        }
    }

    fn contract(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn body(ids: &[Uuid], table_names: Option<&str>) -> Vec<u8> {
        let mut value = json!({
            "fivetran_api_key": "ftkeyTESTfivetrankeyvalue1",
            "fivetran_api_secret": "ftsecTESTfivetransecretvalue1",
            "group_id": "projected_sickle",
            "contract_ids": ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        });
        if let Some(table_names) = table_names {
            value["table_names"] = json!(table_names);
        }
        serde_json::to_vec(&value).unwrap()
    }

    struct MemKeys {
        inserted: Mutex<Vec<(String, String, Vec<Uuid>)>>,
        revoked: Mutex<Vec<Uuid>>,
    }

    impl MemKeys {
        fn new() -> Self {
            Self {
                inserted: Mutex::new(Vec::new()),
                revoked: Mutex::new(Vec::new()),
            }
        }
    }

    impl ConnectorKeys for MemKeys {
        fn insert<'a>(
            &'a self,
            _user_id: Uuid,
            _org_id: Uuid,
            prefix: &'a str,
            hash: &'a str,
            contracts: &'a [Uuid],
        ) -> BoxFuture<'a, AppResult<Uuid>> {
            Box::pin(async move {
                self.inserted.lock().unwrap().push((
                    prefix.to_string(),
                    hash.to_string(),
                    contracts.to_vec(),
                ));
                Ok(Uuid::from_u128(99))
            })
        }

        fn revoke<'a>(&'a self, _org_id: Uuid, key_id: Uuid) -> BoxFuture<'a, AppResult<()>> {
            Box::pin(async move {
                self.revoked.lock().unwrap().push(key_id);
                Ok(())
            })
        }
    }

    struct ScriptHttp {
        /// When false, any HTTP call panics.
        allow: bool,
        group_status: u16,
        group_body: String,
        upload_status: u16,
        upload_body: String,
        /// First create status. 404 triggers the legacy path.
        connections_status: u16,
        connectors_status: u16,
        create_body: String,
        calls: Mutex<Vec<String>>,
        auth: Mutex<Vec<String>>,
        created: Mutex<Vec<Value>>,
        deleted: Mutex<Vec<String>>,
    }

    impl ScriptHttp {
        fn ok() -> Self {
            Self {
                allow: true,
                group_status: 200,
                group_body: r#"{"code":"Success","data":{"id":"projected_sickle"}}"#.into(),
                upload_status: 200,
                upload_body: r#"{"code":"Success","data":{"id":"pkg_test_1"}}"#.into(),
                connections_status: 201,
                connectors_status: 201,
                create_body: r#"{"code":"Success","data":{"id":"conn_test_1","status":"paused","setup_tests":[{"title":"Configuration check","status":"PASSED"}]}}"#.into(),
                calls: Mutex::new(Vec::new()),
                auth: Mutex::new(Vec::new()),
                created: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
            }
        }
    }

    impl ConnectHttp for ScriptHttp {
        fn post_multipart<'a>(
            &'a self,
            path: &'a str,
            auth: &'a str,
            zip: &'a [u8],
        ) -> BoxFuture<'a, Result<HttpReply, String>> {
            Box::pin(async move {
                assert!(self.allow, "HTTP called");
                assert!(zip.starts_with(b"PK"));
                self.calls.lock().unwrap().push(path.to_string());
                self.auth.lock().unwrap().push(auth.to_string());
                let body = if self.upload_status == 401 {
                    format!(
                        r#"{{"message":"rejected {auth} ftkeyTESTfivetrankeyvalue1 ftsecTESTfivetransecretvalue1"}}"#
                    )
                } else {
                    self.upload_body.clone()
                };
                Ok(HttpReply {
                    status: self.upload_status,
                    body,
                })
            })
        }

        fn post_json<'a>(
            &'a self,
            path: &'a str,
            auth: &'a str,
            body: &'a Value,
        ) -> BoxFuture<'a, Result<HttpReply, String>> {
            Box::pin(async move {
                assert!(self.allow, "HTTP called");
                self.calls.lock().unwrap().push(path.to_string());
                self.auth.lock().unwrap().push(auth.to_string());
                self.created.lock().unwrap().push(body.clone());
                let status = if path == CONNECTIONS_PATH {
                    self.connections_status
                } else {
                    self.connectors_status
                };
                let reply = if status == 401 {
                    format!(
                        r#"{{"message":"rejected {auth} ftkeyTESTfivetrankeyvalue1 ftsecTESTfivetransecretvalue1"}}"#
                    )
                } else {
                    self.create_body.clone()
                };
                Ok(HttpReply {
                    status,
                    body: reply,
                })
            })
        }

        fn delete<'a>(
            &'a self,
            path: &'a str,
            auth: &'a str,
        ) -> BoxFuture<'a, Result<HttpReply, String>> {
            Box::pin(async move {
                assert!(self.allow, "HTTP called");
                self.calls.lock().unwrap().push(format!("DELETE {path}"));
                self.auth.lock().unwrap().push(auth.to_string());
                self.deleted.lock().unwrap().push(path.to_string());
                Ok(HttpReply {
                    status: 200,
                    body: r#"{"code":"Success"}"#.into(),
                })
            })
        }

        fn get<'a>(
            &'a self,
            path: &'a str,
            auth: &'a str,
        ) -> BoxFuture<'a, Result<HttpReply, String>> {
            Box::pin(async move {
                assert!(self.allow, "HTTP called");
                self.calls.lock().unwrap().push(path.to_string());
                self.auth.lock().unwrap().push(auth.to_string());
                Ok(HttpReply {
                    status: self.group_status,
                    body: self.group_body.clone(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct Mem(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Mem {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Mem {
        type Writer = Mem;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    struct LogCapture {
        mem: Mem,
        _guard: tracing::subscriber::DefaultGuard,
    }

    impl LogCapture {
        fn start() -> Self {
            let mem = Mem(Arc::new(Mutex::new(Vec::new())));
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_writer(mem.clone())
                .without_time()
                .finish();
            Self {
                mem,
                _guard: tracing::subscriber::set_default(subscriber),
            }
        }

        fn text(&self) -> String {
            String::from_utf8(self.mem.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn assert_clean(text: &str) {
        for secret in [
            "ftkeyTESTfivetrankeyvalue1",
            "ftsecTESTfivetransecretvalue1",
        ] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(!text.contains("cg_live_"), "{text}");
    }

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn vendored_zip_is_the_pinned_connector() {
        assert!(CONNECTOR_PIN.contains(CONNECTOR_COMMIT));
        assert!(CONNECTOR_PY.contains("GET /v1/contracts/{id}/passed"));
        let zip = connector_zip();
        let names = zip_names(&zip);
        assert_eq!(
            names,
            vec![
                "connector.py".to_string(),
                "requirements.txt".to_string(),
                "configuration_form.pb".to_string()
            ]
        );
        assert!(!names.iter().any(|name| name == "configuration.json"));
        let py = zip_file(&zip, "connector.py");
        assert_eq!(py, CONNECTOR_PY.as_bytes());
    }

    fn zip_names(zip: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut i = 0;
        while i + 30 <= zip.len() && zip[i..i + 4] == [0x50, 0x4b, 0x03, 0x04] {
            let name_len = u16::from_le_bytes([zip[i + 26], zip[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([zip[i + 28], zip[i + 29]]) as usize;
            let size = u32::from_le_bytes(zip[i + 18..i + 22].try_into().unwrap()) as usize;
            let name = std::str::from_utf8(&zip[i + 30..i + 30 + name_len])
                .unwrap()
                .to_string();
            names.push(name);
            i += 30 + name_len + extra_len + size;
        }
        names
    }

    fn zip_file<'a>(zip: &'a [u8], wanted: &str) -> &'a [u8] {
        let mut i = 0;
        while i + 30 <= zip.len() && zip[i..i + 4] == [0x50, 0x4b, 0x03, 0x04] {
            let name_len = u16::from_le_bytes([zip[i + 26], zip[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([zip[i + 28], zip[i + 29]]) as usize;
            let size = u32::from_le_bytes(zip[i + 18..i + 22].try_into().unwrap()) as usize;
            let name = std::str::from_utf8(&zip[i + 30..i + 30 + name_len]).unwrap();
            let start = i + 30 + name_len + extra_len;
            if name == wanted {
                return &zip[start..start + size];
            }
            i = start + size;
        }
        panic!("missing {wanted}");
    }

    #[test]
    fn insert_sql_matches_existing_key_shape() {
        assert!(INSERT_KEY_SQL.contains("'Fivetran connector'"));
        assert!(INSERT_KEY_SQL.contains("allowed_contract_ids"));
        assert!(!INSERT_KEY_SQL.to_lowercase().contains("fivetran_api"));
    }

    #[test]
    fn minted_key_uses_the_dashboard_hash() {
        let raw = "cg_live_0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(raw.len(), 56);
        assert_eq!(&raw[..12], "cg_live_0123");
        let expected = B64.encode(Sha256::digest(raw.as_bytes()));
        assert_eq!(hash_key(raw), expected);
        assert_eq!(expected.len(), 44);
    }

    #[test]
    fn public_base_url_must_be_https_without_a_path() {
        assert_eq!(
            parse_public_base_url("https://api.datacontractgate.com").unwrap(),
            "https://api.datacontractgate.com"
        );
        assert!(parse_public_base_url("http://api.datacontractgate.com").is_err());
        assert!(parse_public_base_url("https://user:pass@api.datacontractgate.com").is_err());
        assert!(parse_public_base_url("https://api.datacontractgate.com/v1").is_err());
    }

    #[test]
    fn bad_contract_ids_are_400_and_do_not_echo_the_value() {
        let cases = [
            json!({"fivetran_api_key":"ftkeyTESTfivetrankeyvalue1","fivetran_api_secret":"ftsecTESTfivetransecretvalue1","group_id":"g","contract_ids":["not-a-uuid"]}),
            json!({"fivetran_api_key":"ftkeyTESTfivetrankeyvalue1","fivetran_api_secret":"ftsecTESTfivetransecretvalue1","group_id":"g","contract_ids":[]}),
            json!({"fivetran_api_key":"ftkeyTESTfivetrankeyvalue1","fivetran_api_secret":"ftsecTESTfivetransecretvalue1","group_id":"g","contract_ids":["ftsecTESTfivetransecretvalue1"]}),
        ];
        for case in cases {
            let err = parse_connect_body(&serde_json::to_vec(&case).unwrap()).unwrap_err();
            let text = err.to_string();
            assert!(matches!(err, AppError::BadRequest(_)), "{text}");
            assert_clean(&text);
            let response = err.into_response();
            assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_contract_is_404_before_fivetran() {
        let id = contract(7);
        let http = ScriptHttp {
            allow: false,
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let err = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("growth"),
            &[],
            input,
            "https://api.datacontractgate.com",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::ContractNotFound(missing) if missing == id));
        assert!(keys.inserted.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn free_plan_is_403_for_every_unpaid_plan() {
        for plan in ["free", "aisg_free", "nope"] {
            let http = ScriptHttp {
                allow: false,
                ..ScriptHttp::ok()
            };
            let keys = MemKeys::new();
            let id = contract(3);
            let input = parse_connect_body(&body(&[id], None)).unwrap();
            let err = finish_connect(
                &http,
                &keys,
                false,
                Some(&caller()),
                Some(plan),
                &[id],
                input,
                "https://api.datacontractgate.com",
            )
            .await
            .unwrap_err();
            let response = err.into_response();
            assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let parsed: Value = serde_json::from_slice(&bytes).unwrap();
            let message = parsed["error"].as_str().unwrap();
            assert!(message.contains(&plan::fivetran_connect_upgrade_message()));
            for paid in plan::PAID_PLANS {
                assert!(message.contains(paid), "{message}");
            }
            assert!(keys.inserted.lock().unwrap().is_empty());
        }
        for plan in plan::PAID_PLANS {
            ensure_paid(false, plan).unwrap();
        }
    }

    #[test]
    fn self_hosted_free_org_is_allowed_and_hosted_free_is_403() {
        assert!(connect_allowed(false, None, None).is_ok());
        assert!(require_connect_caller(false, false).is_ok());
        assert!(matches!(
            require_connect_caller(true, false).unwrap_err(),
            AppError::Unauthorized
        ));
        assert!(matches!(
            connect_allowed(false, Some(&caller()), None).unwrap_err(),
            AppError::Unauthorized
        ));
        for plan in ["free", "aisg_free"] {
            let err = connect_allowed(false, Some(&caller()), Some(plan)).unwrap_err();
            let response = err.into_response();
            assert_eq!(
                response.status(),
                axum::http::StatusCode::FORBIDDEN,
                "{plan}"
            );
            connect_allowed(true, Some(&caller()), Some(plan)).unwrap();
        }
        for plan in plan::PAID_PLANS {
            connect_allowed(false, Some(&caller()), Some(plan)).unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn self_host_connects_and_passed_export_accepts_the_sync() {
        let id = contract(17);
        let http = ScriptHttp::ok();
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let response = finish_connect(
            &http,
            &keys,
            false,
            None,
            None,
            &[id],
            input,
            "https://contractgate.example.com",
        )
        .await
        .unwrap();
        assert_eq!(response.status, "paused");
        // No org and no user: the key is not written to api_keys.
        assert!(keys.inserted.lock().unwrap().is_empty());
        let created = http.created.lock().unwrap();
        let secrets = created[0]["config"]["secrets_list"].as_array().unwrap();
        let cg_key = secrets
            .iter()
            .find(|item| item["key"] == "api_key")
            .unwrap()["value"]
            .as_str()
            .unwrap();
        assert!(cg_key.starts_with("cg_live_"));
        assert_eq!(
            secrets
                .iter()
                .find(|item| item["key"] == "base_url")
                .unwrap()["value"],
            "https://contractgate.example.com"
        );
        // Auth off, no org: GET /passed does not look this key up and does
        // not apply the plan gate. That is the sync this connector will make.
        assert!(matches!(
            crate::passed_events::authorize_passed_export(false, None, None, true, true, false),
            Ok(None)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn self_hosted_free_org_connects_and_mints_a_key() {
        let id = contract(18);
        let http = ScriptHttp::ok();
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let response = finish_connect(
            &http,
            &keys,
            true,
            Some(&caller()),
            Some("free"),
            &[id],
            input,
            "https://contractgate.example.com",
        )
        .await
        .unwrap();
        assert_eq!(response.status, "paused");
        let (prefix, _hash, contracts) = keys.inserted.lock().unwrap()[0].clone();
        assert!(prefix.starts_with("cg_live_"));
        assert_eq!(contracts, vec![id]);
        assert_eq!(
            crate::passed_events::authorize_passed_export(
                true,
                Some(caller().org_id),
                Some("free"),
                true,
                true,
                true,
            )
            .unwrap(),
            Some(365)
        );
        assert!(matches!(
            crate::passed_events::authorize_passed_export(
                true,
                Some(caller().org_id),
                Some("free"),
                true,
                true,
                false,
            )
            .unwrap_err(),
            crate::passed_events::PassedDeny::Forbidden
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn success_uploads_package_and_creates_connection() {
        let id = contract(11);
        let http = ScriptHttp::ok();
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(
            &[id],
            Some("00000000-0000-0000-0000-00000000000b=passed_orders"),
        ))
        .unwrap();
        let logs = LogCapture::start();
        let response = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("growth"),
            &[id],
            input,
            "https://api.datacontractgate.com/",
        )
        .await
        .unwrap();
        let logs = logs.text();
        assert_eq!(response.connection_id, "conn_test_1");
        assert_eq!(
            response.setup_url.as_deref(),
            Some("https://fivetran.com/dashboard/connections/conn_test_1/status")
        );
        assert_eq!(response.status, "paused");
        assert_eq!(
            http.calls.lock().unwrap().as_slice(),
            [
                "/v1/groups/projected_sickle",
                PACKAGE_PATH,
                CONNECTIONS_PATH
            ]
        );
        let created = http.created.lock().unwrap();
        let payload = &created[0];
        assert_eq!(payload["service"], "connector_sdk");
        assert_eq!(payload["group_id"], "projected_sickle");
        assert_eq!(payload["paused"], true);
        assert_eq!(payload["config"]["python_version"], "3.12");
        assert_eq!(payload["config"]["package_id"], "pkg_test_1");
        assert_eq!(payload["destination_schema_names"], "SOURCE_NAMING");
        let schema = payload["config"]["schema"].as_str().unwrap();
        assert!(schema.starts_with("contractgate_"), "{schema}");
        assert_eq!(schema.len(), "contractgate_".len() + 8, "{schema}");
        assert!(
            schema
                .bytes()
                .skip("contractgate_".len())
                .all(|byte| byte.is_ascii_hexdigit()),
            "{schema}"
        );
        let secrets = payload["config"]["secrets_list"].as_array().unwrap();
        let cg_key = secrets
            .iter()
            .find(|item| item["key"] == "api_key")
            .unwrap()["value"]
            .as_str()
            .unwrap();
        assert!(cg_key.starts_with("cg_live_"));
        assert_ne!(cg_key, "ftkeyTESTfivetrankeyvalue1");
        assert_eq!(
            secrets
                .iter()
                .find(|item| item["key"] == "base_url")
                .unwrap()["value"],
            "https://api.datacontractgate.com"
        );
        assert_eq!(
            secrets
                .iter()
                .find(|item| item["key"] == "contract_ids")
                .unwrap()["value"],
            id.to_string()
        );
        assert_eq!(
            secrets
                .iter()
                .find(|item| item["key"] == "table_names")
                .unwrap()["value"],
            "00000000-0000-0000-0000-00000000000b=passed_orders"
        );
        let (prefix, hash, contracts) = keys.inserted.lock().unwrap()[0].clone();
        assert_eq!(prefix, &cg_key[..12]);
        assert_eq!(hash, hash_key(cg_key));
        assert_eq!(contracts, vec![id]);
        let auth = &http.auth.lock().unwrap()[0];
        assert!(auth.starts_with("Basic "));
        let payload_text = payload.to_string();
        assert!(!payload_text.contains("ftkeyTESTfivetrankeyvalue1"));
        assert!(!payload_text.contains("ftsecTESTfivetransecretvalue1"));
        assert_clean(&logs);
        assert!(keys.revoked.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connections_404_falls_back_to_legacy_connectors() {
        let id = contract(12);
        let http = ScriptHttp {
            connections_status: 404,
            connectors_status: 201,
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let response = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("aisg_pro"),
            &[id],
            input,
            "https://api.datacontractgate.com",
        )
        .await
        .unwrap();
        assert_eq!(response.connection_id, "conn_test_1");
        assert_eq!(
            http.calls.lock().unwrap().as_slice(),
            [
                "/v1/groups/projected_sickle",
                PACKAGE_PATH,
                CONNECTIONS_PATH,
                CONNECTORS_PATH
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fivetran_401_redacts_keys_in_the_error_and_the_log() {
        let id = contract(13);
        let http = ScriptHttp {
            upload_status: 401,
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let logs = LogCapture::start();
        let err = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("enterprise"),
            &[id],
            input,
            "https://api.datacontractgate.com",
        )
        .await
        .unwrap_err();
        let logs = logs.text();
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("Fivetran rejected the API key"));
        assert_clean(&text);
        assert_clean(&logs);
        assert!(logs.contains("[redacted]") || logs.contains("fivetran rejected"));
        assert_eq!(
            keys.revoked.lock().unwrap().as_slice(),
            &[Uuid::from_u128(99)]
        );
        assert!(!http
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("connections")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bad_credentials_mint_no_key() {
        let id = contract(14);
        let http = ScriptHttp {
            group_status: 401,
            group_body:
                r#"{"message":"rejected ftkeyTESTfivetrankeyvalue1 ftsecTESTfivetransecretvalue1"}"#
                    .into(),
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let logs = LogCapture::start();
        let err = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("growth"),
            &[id],
            input,
            "https://api.datacontractgate.com",
        )
        .await
        .unwrap_err();
        let logs = logs.text();
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("Fivetran rejected the API key"));
        assert_clean(&text);
        assert_clean(&logs);
        assert!(keys.inserted.lock().unwrap().is_empty());
        assert!(keys.revoked.lock().unwrap().is_empty());
        assert_eq!(
            http.calls.lock().unwrap().as_slice(),
            ["/v1/groups/projected_sickle"]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bad_group_id_is_a_clear_error_and_mints_no_key() {
        let id = contract(15);
        let http = ScriptHttp {
            group_status: 404,
            group_body:
                r#"{"message":"missing ftkeyTESTfivetrankeyvalue1 ftsecTESTfivetransecretvalue1"}"#
                    .into(),
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let logs = LogCapture::start();
        let err = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("enterprise"),
            &[id],
            input,
            "https://api.datacontractgate.com",
        )
        .await
        .unwrap_err();
        let logs = logs.text();
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("Fivetran group projected_sickle was not found"));
        assert_clean(&text);
        assert_clean(&logs);
        assert!(keys.inserted.lock().unwrap().is_empty());
        assert!(keys.revoked.lock().unwrap().is_empty());
        assert_eq!(
            http.calls.lock().unwrap().as_slice(),
            ["/v1/groups/projected_sickle"]
        );
    }

    #[test]
    fn openapi_schemas_are_non_empty() {
        let op = openapi_operation();
        let request = &op["requestBody"]["content"]["application/json"]["schema"];
        assert_eq!(request["type"], "object");
        assert!(request["properties"]["fivetran_api_key"]["type"] == "string");
        assert!(request["properties"]["contract_ids"]["items"]["format"] == "uuid");
        assert_eq!(
            request["properties"]["destination_schema"]["pattern"],
            "^[a-z][a-z0-9_]{0,62}$"
        );
        assert!(op["responses"]["500"]["description"]
            .as_str()
            .unwrap()
            .contains("CANONICAL_API_ORIGIN"));
        let bad_request = op["responses"]["400"]["description"].as_str().unwrap();
        assert!(bad_request.contains("Invalid request: Fivetran group <id> was not found"));
        assert!(op["responses"]["413"]["description"]
            .as_str()
            .unwrap()
            .contains("65536"));
        assert!(op["description"]
            .as_str()
            .unwrap()
            .contains("Café becomes caf_<8hex>"));
        let ok = &op["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(ok["type"], "object");
        for field in ["connection_id", "setup_url", "status"] {
            assert!(ok["properties"][field].is_object(), "{field}");
        }
        assert!(!request.as_object().unwrap().is_empty());
        assert!(!ok.as_object().unwrap().is_empty());
    }

    fn example_id() -> Uuid {
        Uuid::parse_str("427c3304-ead1-4ba7-b7e0-123456789abc").unwrap()
    }

    #[test]
    fn default_table_name_is_a_stable_slug_plus_short_id() {
        let id = example_id();
        assert_eq!(
            default_table_name("User Events", &id),
            "user_events_427c3304"
        );
        assert_eq!(default_table_name("123 Orders", &id), "orders_427c3304");
        assert_eq!(default_table_name("Café", &id), "caf_427c3304");
        assert_eq!(default_table_name("!!!", &id), "passed_427c3304");
        assert!(!default_table_name("User Events", &id).contains("_427_c_"));
        let long = "User Events ".repeat(20);
        let table = default_table_name(&long, &id);
        assert!(table.len() <= MAX_TABLE_NAME_LEN, "{table}");
        assert!(table.ends_with("_427c3304"), "{table}");
        assert!(valid_table_name(&table), "{table}");
    }

    #[test]
    fn connect_body_over_64_kib_is_413() {
        let bytes = vec![b'x'; MAX_CONNECT_BODY + 1];
        let err = parse_connect_body(&bytes).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Payload too large: Connect Fivetran body exceeds 65536 bytes"
        );
        assert_eq!(MAX_CONNECT_BODY, 64 * 1024);
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn explicit_table_names_win_and_other_contracts_keep_the_slug() {
        let named = example_id();
        let other = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        let explicit = format!("{named}=passed_orders");
        let input = parse_connect_body(&body(&[named, other], Some(&explicit))).unwrap();
        let assigned = assign_table_names(
            &input.table_names,
            &[(named, "User Events".into()), (other, "Billing".into())],
        )
        .unwrap();
        assert_eq!(
            assigned,
            format!("{named}=passed_orders,{other}=billing_aaaaaaaa")
        );

        let collided = format!("{named}=billing_aaaaaaaa");
        let err = assign_table_names(
            &collided,
            &[(named, "User Events".into()), (other, "Billing".into())],
        )
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        assert!(err.to_string().contains("same table"));
    }

    #[test]
    fn destination_schema_is_optional_and_validated() {
        let id = example_id();
        let ok = body(&[id], None);
        let parsed = parse_connect_body(&ok).unwrap();
        assert!(parsed.destination_schema.is_none());

        let mut value: Value = serde_json::from_slice(&ok).unwrap();
        value["destination_schema"] = Value::Null;
        assert!(parse_connect_body(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .destination_schema
            .is_none());

        value["destination_schema"] = json!("analytics_01");
        let parsed = parse_connect_body(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(parsed.destination_schema.as_deref(), Some("analytics_01"));

        let sixty_three = "a".repeat(63);
        value["destination_schema"] = json!(sixty_three.clone());
        assert_eq!(
            parse_connect_body(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .destination_schema
                .as_deref(),
            Some(sixty_three.as_str())
        );

        let too_long = "a".repeat(64);
        for bad in [
            "",
            "Analytics",
            "1analytics",
            "_analytics",
            "analytics-db",
            "has space",
            too_long.as_str(),
        ] {
            value["destination_schema"] = json!(bad);
            let err = parse_connect_body(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            let response = err.into_response();
            assert_eq!(
                response.status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{bad}"
            );
        }

        value["destination_schema"] = json!(1);
        let err = parse_connect_body(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn connection_body_preserves_schema_and_table_names() {
        let id = example_id();
        let mut input =
            parse_connect_body(&body(&[id], Some(&format!("{id}=passed_orders")))).unwrap();
        input.destination_schema = Some("analytics".into());
        input.table_names =
            assign_table_names(&input.table_names, &[(id, "User Events".into())]).unwrap();
        let payload = connection_body("group", "analytics", "pkg", &input, "https://example.com");
        assert_eq!(payload["destination_schema_names"], "SOURCE_NAMING");
        assert_eq!(payload["config"]["schema"], "analytics");
        let names = payload["config"]["secrets_list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == "table_names")
            .unwrap();
        assert_eq!(names["value"], format!("{id}=passed_orders"));
    }

    #[test]
    fn public_origin_fails_closed_when_unset_and_returns_the_configured_value() {
        for unset in [None, Some(""), Some("   ")] {
            let err = public_base_url_from(unset).unwrap_err();
            let text = err.to_string();
            assert!(matches!(err, AppError::Misconfigured(_)), "{text}");
            assert!(text.contains("CANONICAL_API_ORIGIN is unset"), "{text}");
            assert!(!text.contains("https://api.datacontractgate.com"), "{text}");
            let response = err.into_response();
            assert_eq!(
                response.status(),
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            );
        }
        assert_eq!(
            public_base_url_from(Some("https://api.datacontractgate.com")).unwrap(),
            "https://api.datacontractgate.com"
        );
        assert_eq!(
            public_base_url_from(Some("https://contractgate-api-staging.fly.dev/")).unwrap(),
            "https://contractgate-api-staging.fly.dev"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_public_origin_mints_no_key() {
        let id = contract(16);
        let http = ScriptHttp {
            allow: false,
            ..ScriptHttp::ok()
        };
        let keys = MemKeys::new();
        let input = parse_connect_body(&body(&[id], None)).unwrap();
        let err = finish_connect(
            &http,
            &keys,
            false,
            Some(&caller()),
            Some("growth"),
            &[id],
            input,
            "",
        )
        .await
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("CANONICAL_API_ORIGIN is unset"), "{text}");
        let response = err.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("CANONICAL_API_ORIGIN is unset"));
        assert!(keys.inserted.lock().unwrap().is_empty());
    }
}
