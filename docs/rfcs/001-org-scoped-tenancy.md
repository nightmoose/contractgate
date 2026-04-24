# RFC-001 — Org-Scoped Tenancy

**Status:** Draft — awaiting sign-off  
**Date:** 2026-04-24  
**Author:** Alex Suarez  

---

## Problem

Contracts are currently visible and writable across all user accounts. There is no ownership boundary — any authenticated user can read or overwrite any contract. This breaks collaborative safety: teams need to share access without sharing credentials, and one user's writes must not clobber another's.

---

## Decision

Adopt **Option B — org-scoped tenancy**. Every resource (contracts, API keys, audit log entries) belongs to an org, not to an individual user. Users join orgs and operate within them.

Option C (per-contract ACLs for enterprise) is explicitly deferred. When an enterprise client is in the pipeline, ContractGate will likely ship as a standalone instance for that customer; the ACL layer and any other enterprise-specific concerns will be designed at that time.

---

## Data Model

### New tables

```sql
-- Orgs
CREATE TABLE orgs (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name        text NOT NULL,
  slug        text NOT NULL UNIQUE,          -- url-safe identifier
  plan        text NOT NULL DEFAULT 'free',  -- 'free' | 'pro' | 'enterprise'
  created_at  timestamptz NOT NULL DEFAULT now()
);

-- Org membership
CREATE TABLE org_memberships (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id     uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id    uuid NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
  role       text NOT NULL DEFAULT 'member',  -- 'owner' | 'admin' | 'member'
  invited_by uuid REFERENCES auth.users(id),
  joined_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, user_id)
);
```

### Modified tables

```sql
-- contracts gets org_id (replaces any implicit per-user scoping)
ALTER TABLE contracts ADD COLUMN org_id uuid REFERENCES orgs(id) ON DELETE CASCADE;
CREATE INDEX ON contracts(org_id);

-- api_keys gets org_id (keys are scoped to the org they were created in)
ALTER TABLE api_keys ADD COLUMN org_id uuid REFERENCES orgs(id) ON DELETE CASCADE;

-- audit_log gets org_id (denormalised for fast per-org queries)
ALTER TABLE audit_log ADD COLUMN org_id uuid REFERENCES orgs(id);
```

### Auto-provisioning rule

When a user signs up, a personal org is created automatically (slug derived from their email prefix, e.g. `alexsuarez`) and the user is inserted as `owner`. This keeps the zero-friction solo use case intact — a new user lands in the dashboard and everything just works, with no "create an org first" friction.

---

## Auth & Access Rules

### Dashboard (Next.js)

Every authenticated request already has a Supabase session. The active org is resolved from the session user's `org_memberships` row (for now: one active org per user; multi-org switching is a future UI concern).

All Supabase queries from the dashboard are filtered by `org_id` via Row Level Security:

```sql
-- Contracts: org members can read/write
CREATE POLICY "org members can access contracts"
  ON contracts FOR ALL
  USING (
    org_id IN (
      SELECT org_id FROM org_memberships WHERE user_id = auth.uid()
    )
  );
```

Similar policies on `api_keys` and `audit_log`.

### Rust API (ingest / validate endpoints)

The `x-api-key` header is validated by `ApiKeyCache`. On success, `ValidatedKey` currently carries `user_id` and `allowed_contract_ids`. We add `org_id` to `ValidatedKey`:

```rust
pub struct ValidatedKey {
    pub api_key_id: Uuid,
    pub user_id:    Uuid,
    pub org_id:     Uuid,   // ← new
    pub allowed_contract_ids: Option<Vec<Uuid>>,
}
```

Every DB query in the Rust service that touches `contracts` or `audit_log` gains a `WHERE org_id = $org_id` clause. This is the enforcement layer for machine clients (Kafka connectors, direct API calls).

---

## Invite Flow (MVP)

1. Org owner goes to `/account` → "Invite a team member" → enters email.
2. Supabase sends a magic-link email with a signed `?invite_token=<uuid>` param.
3. Recipient clicks link → signs up (or logs in) → is inserted into `org_memberships` with role `member`.
4. Invite tokens stored in a new `org_invites` table with a 7-day TTL.

```sql
CREATE TABLE org_invites (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id     uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  email      text NOT NULL,
  role       text NOT NULL DEFAULT 'member',
  invited_by uuid NOT NULL REFERENCES auth.users(id),
  token      uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  expires_at timestamptz NOT NULL DEFAULT now() + interval '7 days',
  accepted_at timestamptz
);
```

---

## Multi-org (deferred)

A user can technically have rows in `org_memberships` for more than one org, but the UI will not expose org-switching in the MVP. The data model supports it from day one so there is no migration cost when we add it later.

---

## Enterprise (deferred)

When an enterprise client arrives, the likely path is a dedicated ContractGate instance (own DB, own Rust service, own dashboard). Per-contract ACLs (Option C), SSO/SAML, audit export APIs, and custom SLAs are all deferred to that engagement. This RFC does not need to pre-solve any of those concerns.

---

## Implementation Order

1. **Migration** — `orgs`, `org_memberships`, `org_invites` tables; add `org_id` to `contracts`, `api_keys`, `audit_log`; write auto-provisioning trigger; write RLS policies.
2. **Rust** — add `org_id` to `ValidatedKey`; add `org_id` filter to all contract + audit queries.
3. **Dashboard** — resolve `org_id` from session in a shared hook; thread it into all Supabase queries; add "Members" section to `/account`; add invite UI.
4. **Data migration** — assign all existing contracts/keys to a default org for existing users.
5. **Verification** — confirm Account A cannot see Account B's contracts in staging.

---

## Open Questions

None blocking. Deferred questions (multi-org UI, enterprise ACLs) are noted above.
