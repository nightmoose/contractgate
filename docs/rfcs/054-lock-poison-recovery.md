# RFC-054 — Recover from poisoned locks instead of crashing the process

**Status:** Accepted  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc054  
**Addresses:** REVIEW-2026-05-22-launch-readiness H4  
**Severity:** P1 — high

---

## Problem

Three shared structures `.expect()` on lock poison:

| Lock | File |
|---|---|
| Contract cache `RwLock` | `src/main.rs:146-156` (`cache_read` / `cache_write`) |
| API-key cache `Mutex` | `src/api_key_auth.rs:97-101` (`lock_cache`) |
| Rate-limit bucket `Mutex` | `src/rate_limit.rs:141` |

A `Mutex`/`RwLock` is poisoned when a thread panics while holding it. The
`.expect()` then propagates that panic to **every subsequent caller** — so one
panicked request takes down the whole process and, for a multi-tenant gateway,
every customer with it. The current single-tenant rationale ("we've never seen
this") does not hold once untrusted public tenants share the process.

A poisoned lock does not mean corrupt data — it means *some* writer panicked.
The inner value is still readable via `PoisonError::into_inner()`.

---

## Fix

Recover instead of re-panicking:

```rust
fn cache_read(&self) -> RwLockReadGuard<'_, _> {
    self.contract_cache.read().unwrap_or_else(|e| {
        tracing::error!("contract cache RwLock was poisoned — recovering");
        e.into_inner()
    })
}
```

Apply the same `unwrap_or_else(|e| e.into_inner())` + `tracing::error!`
pattern to all six accessors (`cache_read`, `cache_write`, `lock_cache`, and
the rate-limit bucket lock).

Then **prevent** poisoning at the source: audit the critical sections so no
code between lock acquisition and guard drop can panic (indexing, `unwrap`,
arithmetic). The hot paths are tiny map ops; keep them allocation- and
panic-free.

---

## Testing

- Unit: deliberately poison each lock (panic in a spawned thread holding the
  guard), then assert the next access recovers and returns the data instead of
  panicking.
- `cargo test`, `cargo clippy -- -D warnings`.

## What does NOT change

- Cache semantics, TTLs, rate-limit algorithm — untouched.

## Rollout

Application-only, no migration. Independent — ship standalone.
