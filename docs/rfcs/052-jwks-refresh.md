# RFC-052 — Periodic Supabase JWKS refresh

**Status:** Draft  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc052  
**Addresses:** REVIEW-2026-05-22-launch-readiness H2  
**Severity:** P1 — high

---

## Problem

`main()` fetches the Supabase JWKS exactly once at startup
(`src/main.rs:1192-1214`) and stores it as `Option<Arc<JwkSet>>` on
`AppState`. The key set is never refreshed.

When Supabase rotates its JWT signing keys — which it does, and which the
project cannot control or schedule — every dashboard session token starts
failing `verify_supabase_jwt` with `NoMatchingKey`. The only recovery is a
manual backend restart. For a public launch that is an unacceptable
single-point auth outage.

---

## Fix

1. **Make the JWKS swappable.** Change `AppState.supabase_jwks` to
   `Arc<ArcSwap<Option<JwkSet>>>` (or `Arc<RwLock<...>>`) so it can be
   replaced at runtime without restarting.
2. **Periodic refresh task.** Spawn a background task at startup that re-fetches
   the JWKS every ~10 minutes and swaps it in on success. A failed fetch logs
   a warning and keeps the previous key set — never blanks it.
3. **Refresh-on-unknown-`kid`.** When `verify_supabase_jwt` hits
   `NoMatchingKey`, trigger an out-of-band refresh (debounced, e.g. at most
   once per 60 s) and retry the verification once. This makes rotation
   near-instant instead of waiting for the next interval.
4. **Startup resilience.** If the initial fetch fails, still boot with JWT
   auth disabled (current behaviour) but let the refresh task recover it once
   the network is back — today a startup failure is permanent until restart.

---

## Testing

- Unit: swapping the `JwkSet` is observed by the next `verify_supabase_jwt`
  call without a restart.
- Refresh task: a failing fetch leaves the prior key set intact.
- Debounce: repeated unknown-`kid` hits trigger at most one refresh per window.

## What does NOT change

- JWKS URL derivation (`jwks_url_from_database_url`, `SUPABASE_URL`).
- Token verification logic and the `Uuid::nil()` session sentinel.

## Rollout

Application-only, no migration. Adds the `arc-swap` crate if chosen. Independent
— ship standalone.
