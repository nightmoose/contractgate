# RFC-051 — API-key cache hardening

**Status:** Draft  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc051  
**Addresses:** REVIEW-2026-05-22-launch-readiness H1, M2  
**Severity:** P1 — high

---

## Problem

`src/api_key_auth.rs` has three correctness and hygiene defects in the
60-second key cache:

1. **DB errors are cached as hard failures.** `verify_against_db` maps any
   `sqlx::Error` to `Err(())`, and `validate` (step 3) caches that `Err` for
   the full TTL. A transient DB blip during one validation locks a *valid*
   key out for 60 seconds. "Key not found" and "DB unreachable" must be
   distinguished — only the former is cacheable.
2. **Cache keyed by raw plaintext key.** `HashMap<String, CachedEntry>` holds
   full API keys in the heap for their lifetime. A core dump or memory
   snapshot leaks every active key. `String` zeroes nothing on drop.
3. **No background eviction.** Eviction is one-stale-entry-per-miss. Under key
   churn (CI keys, revoked keys never retried) the map grows unbounded.
4. **`last_used_at` update errors silently swallowed.** `let _ = sqlx::query
   (...)` discards failures, so a renamed column or vanished row makes "Last
   used" silently stale and masks a real bug.

---

## Fix

1. **Three-state validation result.** Change `verify_against_db` to return
   `Result<Option<ValidatedKey>, ()>` — `Ok(Some)` valid, `Ok(None)` key not
   found / hash mismatch (cacheable), `Err(())` DB error (NOT cacheable).
   `validate` caches `Ok(Some)` and `Ok(None)`; on `Err` it returns a `401`
   without writing the cache, so the next request re-checks immediately.
2. **Key the cache by SHA-256 of the raw key.** Same `Sha256` digest already
   computed for hash verification — store the hash, not the plaintext. O(1)
   lookup, no secrets resident in the heap.
3. **Background sweeper.** Spawn one task at startup that scans the map every
   5 minutes and drops entries older than TTL. Bound the map (LRU at ~10 000)
   as belt-and-braces.
4. **Log `last_used_at` failures.** Replace `let _ =` with a match that emits
   `tracing::warn!` on error.

---

## Testing

- A simulated DB error returns `401` and leaves the cache untouched (next call
  re-queries); a "key not found" result is cached.
- Cache map keys contain no substring of a raw `cg_` key.
- Sweeper unit test: entries past TTL are gone after one sweep.

## What does NOT change

- 60-second TTL and the revocation-propagation budget.
- Wire behaviour for callers — a valid key still validates identically.

## Rollout

Application-only, no migration. Independent — ship standalone.
