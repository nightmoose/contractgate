//! Idempotency-Key support for POST /v1/ingest (RFC-021).
//!
//! ### Semantics
//! - No header → no idempotency; request processed normally.
//! - First request with key K → process normally, store result.
//! - Repeat: same key + same body → return cached (status, response).
//! - Repeat: same key + different body → 422 conflict.
//! - `dry_run=true` → never stored (confirmed by Alex 2026-05-01).
//! - TTL sweep is handled by a Supabase scheduled function; no in-process
//!   background task.
//!
//! ### Body hash
//! SHA-256 of the raw request bytes (hex-encoded).  Computed once before
//! parsing so it covers the exact bytes the client sent — content-type
//! agnostic and proof against re-serialisation differences.
//!
//! ### sqlx API
//! Uses the dynamic `sqlx::query` / `query_as` API (not the `query!` macro)
//! so no compile-time DB connection or `.sqlx` cache is required — consistent
//! with the rest of the codebase (see `api_key_auth.rs`, `storage.rs`).

use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of looking up an idempotency key before processing a request.
pub enum IdempotencyLookup {
    /// Key not found (or expired) — process the request normally.
    Miss,
    /// Key found, body hash matches — return this cached response.
    Hit {
        status_code: u16,
        response: serde_json::Value,
    },
    /// Key found but body hash differs — caller must return 422.
    Conflict,
}

/// DB row shape for the SELECT query.
#[derive(FromRow)]
struct IdempotencyRow {
    body_hash: String,
    status_code: i16,
    response: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Compute SHA-256 of raw bytes, return lowercase hex string.
pub fn body_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

/// Look up `key` in the `idempotency_keys` table.
///
/// Returns `Miss` when the key is absent *or* when the row has expired
/// (`expires_at < now()`); the caller treats both cases identically.
pub async fn lookup(
    db: &PgPool,
    key: &str,
    contract_id: Uuid,
    hash: &str,
) -> Result<IdempotencyLookup, sqlx::Error> {
    let row = sqlx::query_as::<_, IdempotencyRow>(
        r#"
        SELECT body_hash, status_code, response
        FROM   idempotency_keys
        WHERE  key         = $1
          AND  contract_id = $2
          AND  expires_at  > now()
        "#,
    )
    .bind(key)
    .bind(contract_id)
    .fetch_optional(db)
    .await?;

    match row {
        None => Ok(IdempotencyLookup::Miss),
        Some(r) if r.body_hash == hash => Ok(IdempotencyLookup::Hit {
            status_code: r.status_code as u16,
            response: r.response,
        }),
        Some(_) => Ok(IdempotencyLookup::Conflict),
    }
}

/// Store a completed response in `idempotency_keys`.
///
/// Uses `INSERT … ON CONFLICT (key) DO UPDATE` so a concurrent second request
/// that raced past the initial `lookup` (Miss) safely overwrites with the
/// authoritative result rather than failing.
///
/// Not called when `dry_run = true`.
pub async fn store(
    db: &PgPool,
    key: &str,
    contract_id: Uuid,
    hash: &str,
    status_code: u16,
    response: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO idempotency_keys
            (key, contract_id, body_hash, status_code, response, expires_at)
        VALUES
            ($1, $2, $3, $4, $5, now() + INTERVAL '24 hours')
        ON CONFLICT (key) DO UPDATE
            SET body_hash   = EXCLUDED.body_hash,
                status_code = EXCLUDED.status_code,
                response    = EXCLUDED.response,
                expires_at  = EXCLUDED.expires_at
        "#,
    )
    .bind(key)
    .bind(contract_id)
    .bind(hash)
    .bind(status_code as i16)
    .bind(response)
    .execute(db)
    .await?;

    Ok(())
}
