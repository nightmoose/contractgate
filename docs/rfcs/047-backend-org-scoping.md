# RFC-047 — Org-scope every by-ID contract and version route in the backend

**Status:** Accepted  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc047-048  
**Addresses:** REVIEW-2026-05-22-launch-readiness B1  
**Severity:** P0 — launch blocker

---

## Problem

The Rust backend connects to Postgres with `DATABASE_URL`, which is the
Supabase **service role**. The service role bypasses RLS unconditionally.
RFC-040 added org-scoped RLS policies, but those only protect the Supabase
REST path — every request that goes through the Rust API is unaffected.

The org-scoped routes (`list_contracts`, `audit`, `stats`) filter by
`org_id_from_req`. The **by-ID routes do not**:

| Route | Handler | Storage fn | org filter? |
|---|---|---|---|
| `GET /contracts/{id}` | `get_contract_handler` | `get_contract_identity` | none |
| `PATCH /contracts/{id}` | `patch_contract_handler` | `patch_contract_identity` | none |
| `DELETE /contracts/{id}` | `delete_contract_handler` | `delete_contract` | none |
| `GET/POST /contracts/{id}/versions` | version handlers | `get_version`, `create_version` | none |
| `PATCH/DELETE .../versions/{v}` | version handlers | `patch_version_yaml`, `delete_version` | none |
| `POST .../promote`, `.../deprecate` | version handlers | `promote_version`, `deprecate_version` | none |
| `.../name-history`, `.../export`, `.../odcs-conformance` | various | various | none |

**Impact:** any holder of any valid JWT or API key can read, edit, promote,
deprecate, or soft-delete any other tenant's contract or version by
enumerating UUIDs. This is a Broken-Object-Level-Authorization (BOLA / IDOR)
hole across the entire single-resource surface — the single largest barrier
to a public launch.

---

## Fix

Thread `org_id` into every by-ID storage function and add an `AND org_id = $N`
predicate (directly on `contracts`, or via the parent contract for
`contract_versions`).

1. **Storage layer.** Add an `org_id: Uuid` parameter to each function in the
   table above. For `contracts`-rooted queries add `AND org_id = $N`. For
   `contract_versions` queries add `AND contract_id IN (SELECT id FROM
   contracts WHERE id = $contract_id AND org_id = $N)`, mirroring the RLS
   policy shape from migration 023 so DB and app logic agree.
2. **Handlers.** Each handler resolves `org_id` via `org_id_from_req` and
   passes it down. A request with no resolvable `org_id` (and not in dev mode)
   is rejected `401` rather than allowed through unscoped.
3. **404 vs 403.** A row that exists but belongs to another org returns `404`
   (`ContractNotFound`) — never `403` — so UUID existence is not leaked.
4. **Service-role / deploy paths.** `deploy_contract_handler` already resolves
   `org_id`; confirm `deploy_contract_version` writes and reads stay within it.

This must land together with RFC-048 (drop the `x-org-id` header trust) so the
`org_id` the backend scopes by is always authoritative.

---

## Testing

- Per-route test: org A creates a contract, org B's key gets `404` on GET,
  PATCH, DELETE, version create, promote, and export of that contract.
- Regression: same-org access still succeeds; dev-mode (no `API_KEY`) still
  works with `org_id = None` short-circuiting to unscoped.
- `cargo test`, `cargo clippy -- -D warnings`, `cargo sqlx prepare`.

---

## What does NOT change

- RLS policies (RFC-040 / migration 023) stay as defense-in-depth.
- The validation hot path (`/ingest`, `/v1/ingest`) is unaffected — it
  resolves contracts by the caller's own key scope already.

## Rollout

Application-only change; no migration. Ship behind the same nightly branch as
RFC-048. Verify with a two-org integration test before merge.
