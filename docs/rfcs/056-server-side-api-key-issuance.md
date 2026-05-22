# RFC-056 — Server-side API-key issuance

**Status:** Draft  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc056  
**Addresses:** REVIEW-2026-05-22-launch-readiness M1  
**Severity:** P2 — medium

---

## Problem

The dashboard's account page creates API keys by calling
`supabase.from("api_keys").insert(...)` **directly from the browser**. There is
no `dashboard/app/api/keys/` route handler. Consequences:

- The browser is trusted to compute `key_hash` correctly. A modified client
  could insert a hash matching a *different* known-cleartext key.
- The hash algorithm is whatever the browser implements — it cannot be
  changed or strengthened server-side.
- There is no server-side audit trail of key issuance.
- The raw key is generated client-side, so its entropy source is the browser.

The keys themselves are high-entropy (`cg_live_` + 48 hex), so this is not an
active break — it is an integrity and auditability gap that should close
before a public launch invites untrusted clients.

---

## Fix

1. **New Next.js route handler** `dashboard/app/api/keys/route.ts`:
   - `POST` — validates the Supabase session, generates the raw key
     **server-side** with a CSPRNG, computes `key_prefix` + `key_hash`
     server-side, inserts via the service role scoped to the session user's
     org, and returns the raw key **exactly once** in the response body.
   - `DELETE` — revokes (stamps `revoked_at`) a key the session user owns.
2. **Lock down the table.** Once issuance is server-only, tighten the
   `api_keys` RLS so `authenticated` can `SELECT` their org's key metadata
   (never `key_hash`) but cannot `INSERT`/`UPDATE` directly. Issuance and
   revocation go through the service-role route only.
3. **Client.** The account page calls the new route; it no longer imports the
   Supabase client for key writes. The raw key is shown once in a copy-once
   modal.
4. **CSRF.** The new mutating route needs the same `Origin`/`Referer`
   same-origin check applied to the other `dashboard/app/api/*` mutating
   routes (org members, github config, invites).

---

## Testing

- A key created via the route verifies against `api_key_auth.rs` (round-trip).
- Direct browser `INSERT` into `api_keys` is rejected by RLS after the
  tightening.
- Revocation propagates within the 60 s cache TTL.

## Rollout

Dashboard + one RLS migration (tighten `api_keys` policies). Apply the
migration after the route ships so issuance is never broken in between.
