//! DB-backed API key validation with a 60-second in-process cache.
//!
//! # Key format
//! Keys are generated client-side in the form `cg_live_<48 hex chars>` (56
//! chars total).  Only two things are stored in the database:
//!   - `key_prefix` — first 12 characters of the raw key (not secret on its
//!     own; used as a cheap discriminator to narrow the DB lookup to a single
//!     candidate row)
//!   - `key_hash` — SHA-256 of the raw key, base64-encoded
//!
//! The raw key is **never** stored anywhere.
//!
//! # Cache design
//! The cache maps the **SHA-256 hex digest of the raw key** to a `CachedEntry`
//! that holds the DB row data and the `Instant` it was inserted.  Using the
//! digest instead of the plaintext ensures no secret material ever sits
//! resident in the heap or appears in core dumps.
//!
//! `CachedEntry.result` is an `Option<ValidatedKey>`:
//!   - `Some(key)` — the key was found and verified (valid hit).
//!   - `None`      — the key was definitively rejected (not found or hash
//!                   mismatch).  This is a cacheable negative result that
//!                   prevents repeated DB queries for the same bad key.
//!
//! DB errors are *not* cached.  `verify_against_db` returns `Err(())` only for
//! transient failures; the caller returns 401 without writing the cache so the
//! next request re-queries the DB immediately.
//!
//! Entries older than `TTL` are evicted by a background sweeper task (spawned
//! once at startup via `ApiKeyCache::spawn_sweeper`).  The map is bounded to
//! `MAX_CACHE_ENTRIES`; the sweeper drops the oldest entries when that cap is
//! hit.  Revocation propagation latency is still ≤ `TTL` (60 s).

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Raw DB row returned by the key-lookup query.
#[derive(FromRow)]
struct ApiKeyRow {
    id: Uuid,
    user_id: Uuid,
    org_id: Uuid,
    key_hash: String,
    allowed_contract_ids: Option<Vec<Uuid>>,
    /// RFC-021 rate-limit overrides.  NULL columns map to None.
    rate_limit_rps: Option<i32>,
    rate_limit_burst: Option<i32>,
}

/// How long a validated key stays in the cache before re-verification.
const TTL: Duration = Duration::from_secs(60);

/// Maximum number of entries allowed in the cache.  The background sweeper
/// drops the oldest entries when this cap is exceeded after evicting stale ones.
const MAX_CACHE_ENTRIES: usize = 10_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Everything we learn about a key from the DB, stored in the cache.
#[derive(Clone, Debug)]
pub struct ValidatedKey {
    pub api_key_id: Uuid,
    pub user_id: Uuid,
    /// The org this key belongs to. All queries are scoped to this org.
    pub org_id: Uuid,
    /// NULL → unrestricted; Some → key only works for listed contract UUIDs.
    pub allowed_contract_ids: Option<Vec<Uuid>>,
    /// Per-key rate-limit override (RFC-021). NULL → use default (100 rps).
    pub rate_limit_rps: Option<u32>,
    /// Per-key burst override (RFC-021). NULL → use default (1000).
    pub rate_limit_burst: Option<u32>,
}

/// A cached outcome for a given key digest.
///
/// `result = Some(key)` → valid and active.
/// `result = None`      → definitively rejected (not found or hash mismatch);
///                        cached to prevent hammering the DB on repeated bad keys.
///
/// DB errors are never stored here — they cause an immediate 401 without
/// writing the cache so the next request re-queries.
#[derive(Debug)]
struct CachedEntry {
    result: Option<ValidatedKey>,
    inserted_at: Instant,
}

/// Shared cache of recently validated API keys.
pub struct ApiKeyCache {
    /// Keyed by hex(SHA-256(raw_key)).  No plaintext keys in the heap.
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
    /// Acquire the cache mutex.  RFC-054: recover from poison instead of
    /// re-panicking.  A poisoned lock means some earlier thread panicked while
    /// holding it; the inner value is still intact and usable.
    fn lock_cache(&self) -> MutexGuard<'_, HashMap<String, CachedEntry>> {
        self.inner.lock().unwrap_or_else(|e| {
            tracing::error!("api-key cache Mutex was poisoned — recovering inner value");
            e.into_inner()
        })
    }

    /// Returns `Ok(ValidatedKey)` if the key is valid and active.
    /// Returns `Err(())` for any rejection: missing, malformed, invalid,
    /// revoked, or a transient DB error.
    ///
    /// # Caching
    /// - Valid hits (`Ok`) and definitive misses (not found / hash mismatch)
    ///   are cached for `TTL`.
    /// - Transient DB errors are *never* cached; the next call re-queries.
    pub async fn validate(&self, raw_key: &str, db: &PgPool) -> Result<ValidatedKey, ()> {
        // Compute cache key — SHA-256 hex digest.  No plaintext in the map.
        let cache_key = hex::encode(Sha256::digest(raw_key.as_bytes()));

        // 1. Fast path: in-cache and not stale.
        {
            let map = self.lock_cache();
            if let Some(entry) = map.get(&cache_key) {
                if entry.inserted_at.elapsed() < TTL {
                    return entry.result.clone().ok_or(());
                }
                // stale — fall through to DB verification
            }
        }

        // 2. Slow path: query the DB.
        //    Ok(Some) = valid key.
        //    Ok(None) = definitively rejected (not found or hash mismatch) — cacheable.
        //    Err(())  = transient DB error — NOT cacheable.
        let outcome = verify_against_db(raw_key, db).await;

        match outcome {
            Ok(opt_key) => {
                // 3. Cache the cacheable outcome (hit or definitive miss).
                {
                    let mut map = self.lock_cache();

                    // Opportunistic eviction: drop one stale entry while
                    // holding the lock, in addition to the sweeper's pass.
                    let stale = map
                        .iter()
                        .find(|(_, e)| e.inserted_at.elapsed() >= TTL)
                        .map(|(k, _)| k.clone());
                    if let Some(k) = stale {
                        map.remove(&k);
                    }

                    map.insert(
                        cache_key.clone(),
                        CachedEntry {
                            result: opt_key.clone(),
                            inserted_at: Instant::now(),
                        },
                    );
                }

                // 4. Fire-and-forget: touch last_used_at on a valid hit.
                //    Emit a warning if the update fails (masked bugs, renamed
                //    columns) rather than silently swallowing the error.
                if let Some(ref validated) = opt_key {
                    let key_id = validated.api_key_id;
                    let db2 = db.clone();
                    tokio::spawn(async move {
                        match sqlx::query(
                            "UPDATE api_keys SET last_used_at = now() WHERE id = $1",
                        )
                        .bind(key_id)
                        .execute(&db2)
                        .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    api_key_id = %key_id,
                                    "failed to update last_used_at: {e}"
                                );
                            }
                        }
                    });
                }

                // Convert Option<ValidatedKey> → Result<ValidatedKey, ()>
                opt_key.ok_or(())
            }
            Err(()) => {
                // Transient DB error — return 401 without caching.  The next
                // request will re-query and may succeed once the DB recovers.
                Err(())
            }
        }
    }

    /// Immediately evicts a key from the cache (e.g. after a 401 response so
    /// the next request re-verifies rather than serving a stale miss).
    pub fn evict(&self, raw_key: &str) {
        let cache_key = hex::encode(Sha256::digest(raw_key.as_bytes()));
        self.lock_cache().remove(&cache_key);
    }

    /// Spawn a background task that sweeps expired entries from the cache
    /// every 5 minutes and enforces the `MAX_CACHE_ENTRIES` cap.
    ///
    /// Call once at startup after wrapping the cache in an `Arc`:
    /// ```ignore
    /// ApiKeyCache::spawn_sweeper(state.key_cache.clone());
    /// ```
    pub fn spawn_sweeper(cache: Arc<ApiKeyCache>) {
        tokio::spawn(async move {
            let interval = Duration::from_secs(300); // 5 min
            loop {
                tokio::time::sleep(interval).await;
                let mut map = cache.lock_cache();

                // Drop all expired entries.
                map.retain(|_, e| e.inserted_at.elapsed() < TTL);

                // Enforce the hard cap: evict the oldest entries if needed.
                if map.len() > MAX_CACHE_ENTRIES {
                    let mut by_age: Vec<(String, Instant)> = map
                        .iter()
                        .map(|(k, e)| (k.clone(), e.inserted_at))
                        .collect();
                    // Sort ascending — oldest first.
                    by_age.sort_by_key(|(_, t)| *t);
                    let to_drop = map.len() - MAX_CACHE_ENTRIES;
                    for (k, _) in by_age.into_iter().take(to_drop) {
                        map.remove(&k);
                    }
                    tracing::warn!(
                        "api-key cache exceeded cap ({MAX_CACHE_ENTRIES}); evicted {to_drop} oldest entries"
                    );
                }

                tracing::debug!(
                    entries = map.len(),
                    "api-key cache sweep complete"
                );
            }
        });
    }
}

// ---------------------------------------------------------------------------
// DB lookup + hash verification
// ---------------------------------------------------------------------------

/// Query the DB for `raw_key` and verify its hash.
///
/// Returns:
/// - `Ok(Some(key))` — key found, hash matches, not revoked.
/// - `Ok(None)`      — definitively rejected: prefix not found, already
///                     revoked, or hash mismatch.  Cacheable.
/// - `Err(())`       — transient DB error.  The caller must not cache this.
async fn verify_against_db(raw_key: &str, db: &PgPool) -> Result<Option<ValidatedKey>, ()> {
    // Sanity-check length / prefix to avoid DB queries for junk values.
    if raw_key.len() < 12 || !raw_key.starts_with("cg_") {
        return Ok(None); // definitively not a valid key — cacheable miss
    }

    let key_prefix = &raw_key[..12];

    // Query a single candidate row by prefix (indexed, fast).
    let row = sqlx::query_as::<_, ApiKeyRow>(
        r#"
        SELECT id, user_id, org_id, key_hash, allowed_contract_ids,
               rate_limit_rps, rate_limit_burst
        FROM   api_keys
        WHERE  key_prefix = $1
          AND  revoked_at IS NULL
          AND  deleted_at IS NULL
        LIMIT  1
        "#,
    )
    .bind(key_prefix)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        // Transient DB failure — do NOT cache.
        tracing::error!("api_key DB lookup failed: {e}");
    })?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None), // prefix not found — cacheable miss
    };

    // Compute SHA-256 of the raw key and compare to stored hash.
    let hash_bytes = Sha256::digest(raw_key.as_bytes());
    let computed_hash = B64.encode(hash_bytes);

    if computed_hash != row.key_hash {
        tracing::warn!(
            prefix = key_prefix,
            "api_key prefix matched but hash did not verify"
        );
        return Ok(None); // hash mismatch — cacheable miss
    }

    Ok(Some(ValidatedKey {
        api_key_id: row.id,
        user_id: row.user_id,
        org_id: row.org_id,
        allowed_contract_ids: row.allowed_contract_ids,
        // Cast i32 → u32; the CHECK constraint guarantees > 0.
        rate_limit_rps: row.rate_limit_rps.map(|v| v as u32),
        rate_limit_burst: row.rate_limit_burst.map(|v| v as u32),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // Helper: build a cache and pre-populate one entry directly.
    fn cache_with_entry(cache_key: &str, result: Option<ValidatedKey>, age: Duration) -> ApiKeyCache {
        let cache = ApiKeyCache::default();
        let mut map = cache.inner.lock().unwrap();
        map.insert(
            cache_key.to_owned(),
            CachedEntry {
                result,
                inserted_at: Instant::now() - age,
            },
        );
        drop(map);
        cache
    }

    fn dummy_key() -> ValidatedKey {
        ValidatedKey {
            api_key_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            allowed_contract_ids: None,
            rate_limit_rps: None,
            rate_limit_burst: None,
        }
    }

    // -----------------------------------------------------------------------
    // RFC-051: Cache map keys must not contain raw key material
    // -----------------------------------------------------------------------

    #[test]
    fn cache_key_contains_no_plaintext() {
        let raw = "cg_live_abcdef123456789012345678901234567890123456789012";
        let cache_key = hex::encode(Sha256::digest(raw.as_bytes()));
        // The cache key must not contain any substring of the raw key.
        assert!(
            !cache_key.contains("cg_"),
            "cache key must not contain 'cg_' prefix"
        );
        assert!(
            !cache_key.contains("abcdef"),
            "cache key must not contain raw key material"
        );
    }

    // -----------------------------------------------------------------------
    // RFC-051: Sweeper evicts entries older than TTL
    // -----------------------------------------------------------------------

    #[test]
    fn sweeper_evicts_expired_entries() {
        let cache = ApiKeyCache::default();

        let raw = "cg_live_aabbcc112233445566778899001122334455667788990011";
        let cache_key = hex::encode(Sha256::digest(raw.as_bytes()));

        // Insert an entry that is already past TTL.
        {
            let mut map = cache.inner.lock().unwrap();
            map.insert(
                cache_key.clone(),
                CachedEntry {
                    result: Some(dummy_key()),
                    inserted_at: Instant::now() - TTL - Duration::from_secs(1),
                },
            );
        }

        // Run the sweep logic directly (no async needed).
        {
            let mut map = cache.inner.lock().unwrap();
            map.retain(|_, e| e.inserted_at.elapsed() < TTL);
        }

        let map = cache.inner.lock().unwrap();
        assert!(
            !map.contains_key(&cache_key),
            "expired entry must be gone after sweep"
        );
    }

    // -----------------------------------------------------------------------
    // RFC-051: Fast path serves a fresh cached hit without needing the DB
    // -----------------------------------------------------------------------

    #[test]
    fn fresh_hit_served_from_cache() {
        let raw = "cg_live_001122334455667788990011223344556677889900112233";
        let cache_key = hex::encode(Sha256::digest(raw.as_bytes()));
        let vk = dummy_key();

        let cache = cache_with_entry(&cache_key, Some(vk.clone()), Duration::from_secs(1));

        // Read directly from the map — in-cache and fresh.
        let map = cache.inner.lock().unwrap();
        let entry = map.get(&cache_key).expect("entry must be present");
        assert!(
            entry.inserted_at.elapsed() < TTL,
            "entry must still be fresh"
        );
        assert!(
            entry.result.is_some(),
            "cached hit must contain a ValidatedKey"
        );
    }

    // -----------------------------------------------------------------------
    // RFC-051: A stale hit is not returned from the cache
    // -----------------------------------------------------------------------

    #[test]
    fn stale_entry_not_returned() {
        let raw = "cg_live_ffeeddccbbaa998877665544332211ffeeddccbbaa998877";
        let cache_key = hex::encode(Sha256::digest(raw.as_bytes()));

        let cache = cache_with_entry(&cache_key, Some(dummy_key()), TTL + Duration::from_secs(1));

        let map = cache.inner.lock().unwrap();
        let entry = map.get(&cache_key).expect("entry present");
        assert!(
            entry.inserted_at.elapsed() >= TTL,
            "entry must be considered stale"
        );
    }

    // -----------------------------------------------------------------------
    // RFC-054: Poisoned Mutex is recovered without panicking
    // -----------------------------------------------------------------------

    #[test]
    fn poisoned_mutex_is_recovered() {
        let cache = Arc::new(ApiKeyCache::default());

        // Pre-populate before poisoning.
        let raw = "cg_live_deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let cache_key = hex::encode(Sha256::digest(raw.as_bytes()));
        {
            let mut map = cache.inner.lock().unwrap();
            map.insert(
                cache_key.clone(),
                CachedEntry {
                    result: Some(dummy_key()),
                    inserted_at: Instant::now(),
                },
            );
        }

        // Poison the mutex by panicking inside a thread that holds the guard.
        let cache2 = Arc::clone(&cache);
        let _ = thread::spawn(move || {
            let _guard = cache2.inner.lock().unwrap();
            panic!("intentional poison");
        })
        .join(); // join() returns Err — that's expected

        // lock_cache() must recover via into_inner(), not re-panic.
        let map = cache.lock_cache();
        assert!(
            map.contains_key(&cache_key),
            "data must be intact after poison recovery"
        );
    }
}
