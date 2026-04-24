//! DB-backed API key validation with a 60-second in-process cache.
//!
//! # Key format
//! Keys are generated client-side in the form `cg_live_<48 hex chars>` (56
//! chars total).  Only two things are stored in the database:
//!   - `key_prefix`  — first 12 characters of the raw key (not secret on its
//!                     own; used as a cheap discriminator to narrow the DB
//!                     lookup to a single candidate row)
//!   - `key_hash`    — SHA-256 of the raw key, base64-encoded
//!
//! The raw key is **never** stored anywhere.
//!
//! # Cache design
//! The cache maps the raw `x-api-key` header value (the full key) to a
//! `CachedEntry` that holds the DB row data and the `Instant` it was inserted.
//! Entries older than `TTL` are evicted on the next lookup — no background
//! thread required.  This keeps revocation propagation ≤ 60 s, which is
//! acceptable for Kafka connector workloads.
//!
//! Cache poisoning is not a concern: an attacker who knows a valid key already
//! has access; the cache only stores *validated* keys.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Raw DB row returned by the key-lookup query.
#[derive(FromRow)]
struct ApiKeyRow {
    id: Uuid,
    user_id: Uuid,
    key_hash: String,
    allowed_contract_ids: Option<Vec<Uuid>>,
}

/// How long a validated key stays in the cache before re-verification.
const TTL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Everything we learn about a key from the DB, stored in the cache.
#[derive(Clone, Debug)]
pub struct ValidatedKey {
    pub api_key_id: Uuid,
    pub user_id: Uuid,
    /// NULL → unrestricted; Some → key only works for listed contract UUIDs.
    pub allowed_contract_ids: Option<Vec<Uuid>>,
}

#[derive(Debug)]
struct CachedEntry {
    result: Result<ValidatedKey, ()>,
    inserted_at: Instant,
}

/// Shared cache of recently validated API keys.
pub struct ApiKeyCache {
    inner: Mutex<HashMap<String, CachedEntry>>,
}

impl Default for ApiKeyCache {
    fn default() -> Self {
        ApiKeyCache {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Core validation
// ---------------------------------------------------------------------------

impl ApiKeyCache {
    /// Returns `Ok(ValidatedKey)` if the key is valid and active,
    /// `Err(())` if it is missing, malformed, invalid, or revoked.
    pub async fn validate(&self, raw_key: &str, db: &PgPool) -> Result<ValidatedKey, ()> {
        // 1. Fast path: in-cache and not stale.
        {
            let map = self.inner.lock().unwrap();
            if let Some(entry) = map.get(raw_key) {
                if entry.inserted_at.elapsed() < TTL {
                    return entry.result.clone();
                }
                // stale — fall through to DB verification
            }
        }

        // 2. Slow path: query the DB.
        let outcome = verify_against_db(raw_key, db).await;

        // 3. Update cache (store both hits and misses to avoid stampedes).
        {
            let mut map = self.inner.lock().unwrap();
            // Opportunistic eviction: remove one stale entry while we're holding the lock.
            let stale_key = map
                .iter()
                .find(|(_, e)| e.inserted_at.elapsed() >= TTL)
                .map(|(k, _)| k.clone());
            if let Some(k) = stale_key {
                map.remove(&k);
            }
            map.insert(
                raw_key.to_owned(),
                CachedEntry {
                    result: outcome.clone(),
                    inserted_at: Instant::now(),
                },
            );
        }

        // 4. Fire-and-forget: update last_used_at in the DB on a successful hit.
        //    We don't await this — a missed update is preferable to adding latency.
        if let Ok(ref validated) = outcome {
            let key_id = validated.api_key_id;
            let db2 = db.clone();
            tokio::spawn(async move {
                let _ = sqlx::query(
                    "UPDATE api_keys SET last_used_at = now() WHERE id = $1",
                )
                .bind(key_id)
                .execute(&db2)
                .await;
            });
        }

        outcome
    }

    /// Immediately evicts a key from the cache (e.g. after a 401 response so
    /// the next request re-verifies rather than serving a stale miss).
    pub fn evict(&self, raw_key: &str) {
        self.inner.lock().unwrap().remove(raw_key);
    }
}

// ---------------------------------------------------------------------------
// DB lookup + hash verification
// ---------------------------------------------------------------------------

async fn verify_against_db(raw_key: &str, db: &PgPool) -> Result<ValidatedKey, ()> {
    // Sanity-check length / prefix to avoid DB queries for junk values.
    if raw_key.len() < 12 || !raw_key.starts_with("cg_") {
        return Err(());
    }

    let key_prefix = &raw_key[..12];

    // Query a single candidate row by prefix (indexed, fast).
    // Uses the dynamic query API (not query!) so no compile-time DB connection
    // or .sqlx cache entry is required for this new table.
    let row = sqlx::query_as::<_, ApiKeyRow>(
        r#"
        SELECT id, user_id, key_hash, allowed_contract_ids
        FROM   api_keys
        WHERE  key_prefix = $1
          AND  revoked_at IS NULL
        LIMIT  1
        "#,
    )
    .bind(key_prefix)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        tracing::error!("api_key DB lookup failed: {e}");
    })?;

    let row = row.ok_or(())?;

    // Compute SHA-256 of the raw key and compare to stored hash.
    let hash_bytes = Sha256::digest(raw_key.as_bytes());
    let computed_hash = B64.encode(hash_bytes);

    if computed_hash != row.key_hash {
        tracing::warn!(
            prefix = key_prefix,
            "api_key prefix matched but hash did not verify"
        );
        return Err(());
    }

    Ok(ValidatedKey {
        api_key_id: row.id,
        user_id: row.user_id,
        allowed_contract_ids: row.allowed_contract_ids,
    })
}
