# RFC-048 — Remove the trusted `x-org-id` header fallback

**Status:** Accepted  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc047-048  
**Addresses:** REVIEW-2026-05-22-launch-readiness B2  
**Severity:** P0 — launch blocker

---

## Problem

`org_id_from_req` (`src/main.rs:287-297`) resolves the caller's org in two
steps:

1. `ValidatedKey` from request extensions (authoritative — DB-backed key or
   verified JWT).
2. **Fallback:** the client-supplied `x-org-id` request header.

The legacy env-var `API_KEY` path injects **no** `ValidatedKey`. So any client
holding the shared env-var key reaches step 2 and can set `x-org-id` to any
org UUID it wants. On every org-scoped route (`list_contracts`, `audit`,
`stats`, `create_contract`, `deploy`, and — once RFC-047 lands — every by-ID
route) that is full tenant impersonation.

The header exists because the dashboard historically sent it alongside the
env-var key. Now that RFC-039 ships Bearer-JWT auth, the dashboard has an
authoritative `org_id` from the verified token and no longer needs the header.

---

## Fix

1. **Delete the header fallback.** `org_id_from_req` returns `org_id` only
   from a `ValidatedKey`. No `ValidatedKey` ⇒ `None`.
2. **Reject unscoped authenticated requests.** When `API_KEY` auth is
   configured and a request reaches an org-scoped handler with `org_id =
   None`, return `401` — never run an unscoped query.
3. **Dev mode unchanged.** When `API_KEY` is empty (local dev, no auth),
   `org_id = None` continues to mean "single-tenant, unscoped" — the existing
   `make demo` behaviour.
4. **Dashboard.** Remove `setApiOrgId` / the `x-org-id` header from
   `dashboard/lib/api.ts`; the Bearer JWT already carries org context.
5. **Connectors / CLI / SDKs.** These already use DB-backed keys, which carry
   `org_id` in the row — no change.

---

## Migration concern

The legacy env-var `API_KEY` branch (`src/main.rs:834-836`) survives for
zero-downtime connector migration but can no longer write org-scoped data
(it has no org). Audit usage: if no connector still relies on it, drop the
branch entirely in this RFC. If one does, that connector must be reissued a
DB-backed key first — track as a checklist item, not a blocker.

---

## Testing

- A request with the env-var key + a forged `x-org-id` header gets `401` on
  every org-scoped route (previously: served that org's data).
- JWT and DB-backed key paths are unaffected — `org_id` still resolves.
- Dev mode (`API_KEY` unset) still serves unscoped.

## Rollout

Application-only. Ship in the same branch/PR as RFC-047 so org scoping is
introduced and the spoofable input is removed atomically.
