# RFC-040 — Fix RLS on contract_versions, contract_name_history, quarantine_events

**Status:** Accepted  
**Date:** 2026-05-16  
**Branch:** dev/p02-rls-contract-versions  
**Addresses:** REVIEW-2026-05-16-saas-readiness P0-2

---

## Problem

Migrations 002 and 003 created `auth_all` policies (`FOR ALL TO authenticated
USING (TRUE)`) on three tables that were never cleaned up:

| Table | Migration | Policy name |
|---|---|---|
| `quarantine_events` | 002 | `auth_all` |
| `contract_versions` | 003 | `auth_all` |
| `contract_name_history` | 003 | `auth_all` |

Migration 007 dropped `auth_all` on `contracts`, `audit_log`, and
`forwarded_events` and replaced them with org-scoped policies, but never
touched these three tables.

**Impact:** Any authenticated Supabase user can SELECT, INSERT, UPDATE, and
DELETE the YAML, version ladder, and quarantined raw payloads of every other
tenant via the REST API. The `active_contracts_public` view (RFC-028) and the
`provider_field_health` view (RFC-031) read from these tables and inherit the
same exposure from the browser.

---

## Fix

A single migration (023) that mirrors the pattern from migrations 008 and 013:

1. **Drop** `auth_all` on all three tables.
2. **Recreate** org-scoped policies using `public.get_my_org_ids()` — the
   SECURITY DEFINER helper that avoids the PG-42P17 recursion that inline
   subqueries on `org_memberships` cause.

### Policy shape (same as contracts, audit_log, forwarded_events)

**`contract_versions`** — needs SELECT, INSERT, UPDATE, DELETE scoped by the
parent contract's `org_id`. The table has no direct `org_id` column; join
through `contracts`:

```sql
-- SELECT / UPDATE / DELETE: version's contract must belong to caller's org
USING (
  contract_id IN (
    SELECT id FROM contracts
    WHERE org_id IN (SELECT public.get_my_org_ids())
      AND deleted_at IS NULL
  )
)
-- INSERT: same check for the contract being written to
WITH CHECK (
  contract_id IN (
    SELECT id FROM contracts
    WHERE org_id IN (SELECT public.get_my_org_ids())
      AND deleted_at IS NULL
  )
)
```

**`contract_name_history`** — same join pattern through `contracts`.

**`quarantine_events`** — has a direct `contract_id` FK; same join pattern.

### Views

`active_contracts_public` and `provider_field_health` are plain (non-SECURITY
DEFINER) views. Once the underlying table policies are org-scoped, the views
automatically show only the caller's org data — no view changes needed.

---

## What does NOT change

- `service_all` policies on all three tables are left in place (they are
  no-ops since the service role bypasses RLS unconditionally, but removing them
  is a separate P2-8 cleanup task).
- The Rust backend is unaffected — it connects as the service role and bypasses
  RLS entirely.
- No application code changes needed.

---

## Rollout

Apply migration 023 via `supabase db push` (or the Supabase dashboard SQL
editor). No downtime required — `DROP POLICY` + `CREATE POLICY` on live tables
is a metadata-only operation in Postgres.
