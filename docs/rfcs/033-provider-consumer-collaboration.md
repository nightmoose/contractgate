# RFC-033: Provider-Consumer Collaboration

**Status:** Accepted
**Date:** 2026-05-14
**Author:** Alex Suarez
**Implemented:** 2026-05-15

---

## Problem

Two parties — a provider and a consumer — frequently need to work on the
*same* contract together. The provider knows what it can emit; the
consumer knows what it needs; the contract is where they meet.

Today the only way to collaborate is to **invite the other party into
your org**. That is a blunt instrument:

- It grants the outside party access to *everything* in the org —
  every other contract, every audit log, every quarantine event — when
  all they should touch is one shared contract.
- There is no notion of *role*. An invited member can do anything a
  member can do.
- There is no structured way to discuss a field or propose a change —
  it falls back to email, exactly the problem RFC-032 set out to kill.

RFC-032 gave us one-directional distribution (publish → import). This
RFC adds the missing piece: **scoped, cross-org, role-based
collaboration on a single shared contract**, with the provider and
consumer each staying in their own org.

## Proposed Solution

A **contract collaborator** model. A contract owned by one org can grant
*other orgs* a scoped role on that contract alone — no org invite, no
access to anything else.

### Roles

| Role       | On the shared contract — can… |
|------------|-------------------------------|
| `owner`    | Everything. The authoring org. Implicit, not granted. |
| `editor`   | Propose changes, comment, edit draft versions. Cannot publish or change collaborators. |
| `reviewer` | Comment and approve/reject change proposals. Cannot edit. |
| `viewer`   | Read the contract definition only. (This is what RFC-032 `org`-visibility import grants.) |

Crucially, a collaborator role scopes to **the contract definition and
its review surface** — never the owner org's runtime data. A
collaborator cannot see the owner's `audit_log`, `quarantine_events`,
or `contracts.pii_salt`. (A consumer seeing *its own* provider
scorecard is a separate, narrower grant — see Interactions.)

### Review surface

So collaboration does not fall back to email, two lightweight surfaces:

- **Comments** — threaded notes attached to a contract, optionally
  pinned to a specific field. Resolvable.
- **Change proposals** — an `editor` proposes a contract change; a
  `reviewer` or `owner` approves or rejects. An approved proposal is
  applied by the owner. Collaborator edits never land directly on a
  stable version — they always flow through proposal → approval.

### Schema (proposed)

```sql
-- migration 021_contract_collaboration.sql

CREATE TABLE contract_collaborators (
  contract_name text NOT NULL,
  org_id        uuid NOT NULL REFERENCES orgs (id),
  role          text NOT NULL CHECK (role IN ('editor', 'reviewer', 'viewer')),
  granted_by    uuid NOT NULL,
  granted_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (contract_name, org_id)
);

CREATE TABLE contract_comments (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  contract_name text NOT NULL,
  field         text,                       -- null = whole-contract comment
  org_id        uuid NOT NULL,              -- commenter's org
  author        text NOT NULL,
  body          text NOT NULL,
  resolved      boolean NOT NULL DEFAULT false,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE contract_change_proposals (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  contract_name text NOT NULL,
  proposed_by   uuid NOT NULL,              -- proposing org
  proposed_yaml text NOT NULL,
  status        text NOT NULL DEFAULT 'open'
                CHECK (status IN ('open', 'approved', 'rejected', 'applied')),
  decided_by    uuid,
  created_at    timestamptz NOT NULL DEFAULT now()
);
```

### RLS — must use `get_my_org_ids()`

This RFC widens contract visibility from "my org owns it" to "my org
owns it **or** my org is a collaborator." That predicate is RLS, and
RLS on org membership in this codebase **must** route through the
`get_my_org_ids()` helper. Inline subqueries against `org_memberships`
re-trigger the PG `42P17` infinite-recursion failure that migration 013
was written to fix.

```sql
-- contracts visibility: owner org OR collaborator org
CREATE POLICY contracts_collaborator_read ON contracts
  FOR SELECT USING (
    owner_org_id = ANY (get_my_org_ids())
    OR name IN (
      SELECT contract_name FROM contract_collaborators
      WHERE org_id = ANY (get_my_org_ids())
    )
  );
```

Same pattern for `contract_comments` and `contract_change_proposals`.
Every new policy is helper-routed; none introduce an inline
`org_memberships` subquery.

### API

- `GET / POST /contracts/{name}/collaborators` — list / grant a role.
- `PATCH / DELETE /contracts/{name}/collaborators/{org_id}` — change /
  revoke a role.
- `GET / POST /contracts/{name}/comments` — list / add comments;
  `POST /contracts/{name}/comments/{id}/resolve`.
- `GET / POST /contracts/{name}/proposals` — list / open a proposal;
  `POST /contracts/{name}/proposals/{id}/decide` — approve / reject
  (reviewer/owner); `POST .../apply` (owner).

### What this unlocks

- **Collaboration without over-sharing.** An outside org touches one
  contract, nothing else. The org-invite hack goes away.
- **Structured review.** Field-level comments and change proposals
  replace the email thread.
- **RFC-032 `org` visibility becomes real.** An `org`-visibility
  publication is implemented as a `viewer` collaborator grant — the
  two RFCs meet here.
- **A provider can co-own its contract.** The PMS vendor and Findigs
  jointly maintain the contract that governs the data between them,
  each from their own org.

### Integration points

- **Tenancy:** builds directly on RFC-001 (`orgs`, `org_memberships`,
  `get_my_org_ids()`).
- **Distribution:** RFC-032 `org` visibility resolves to a `viewer`
  grant in `contract_collaborators`.
- **New module:** `src/collaboration.rs` — collaborator grants,
  comments, proposals, the approve/apply flow.
- **Routing:** new routes nested under `/contracts/{name}/...`.
- **Dashboard:** a "Collaborators" panel on the contract page; a
  comment thread; a proposals inbox.

## Out of Scope (this RFC)

- **Real-time co-editing** (cursors, live presence). Proposal-based
  async review only.
- **Notifications** (email/webhook on a new comment or proposal).
  Surfaced through the API in v1; pushing them is later.
- **Collaborator access to runtime data.** Audit logs, quarantine
  events, and the PII salt stay owner-org-scoped, full stop. The one
  narrow exception is scoped scorecard access — see Interactions.
- **The publication mechanism itself** — RFC-032.

## Interactions

- **RFC-032:** `org`-visibility publications are implemented as
  `viewer` rows in `contract_collaborators`. RFC-032 ships `public` +
  `link`; `org` lands here.
- **RFC-031:** a consumer may be granted read access to *its own*
  provider scorecard (the data about the feed it sends/receives). That
  is a separate, narrow grant keyed on `source` — it is **not** implied
  by any `contract_collaborators` role and should be designed as its
  own small surface, not folded into the role table.

## Open Questions

1. **Role count.** Four roles (`owner`/`editor`/`reviewer`/`viewer`),
   or collapse to three (`owner`/`editor`/`viewer`, where `editor`
   also approves)? Recommendation: keep `reviewer` — separating "can
   propose" from "can approve" is the whole point of a review flow.
2. **Can an `editor` ever edit directly?** Or is every collaborator
   edit a proposal? Recommendation: every collaborator edit is a
   proposal; only the `owner` edits a stable version directly. Keeps
   "the owner org is accountable for what ships" true.
3. **Comment threading depth.** Flat list with a `field` anchor, or
   true threaded replies? Recommendation: flat + `field` anchor +
   `resolved` in v1; threading later if it is actually needed.
4. **Granting target.** Grant a role to an *org* only, or also to an
   individual email? Recommendation: org only — individual scoping
   reintroduces the membership-management problem RFC-001 already
   owns.
5. **Self-service vs owner-grant.** Can an org *request* collaborator
   access on a `public`/`link` contract, or must the owner always
   initiate? Recommendation: owner-initiated only in v1; a request
   flow is a clean follow-up.

## Acceptance Criteria

- [x] Migration `021_contract_collaboration.sql` adds
      `contract_collaborators`, `contract_comments`,
      `contract_change_proposals`
- [x] All new RLS policies route org membership through
      `get_my_org_ids()` — no inline `org_memberships` subqueries
      (no `42P17` regression)
- [x] A collaborator org can read a shared contract but **cannot** read
      the owner org's `audit_log`, `quarantine_events`, or
      `contracts.pii_salt` — asserted by test
- [x] `editor` / `reviewer` / `viewer` permissions behave per the role
      table
- [x] Collaborator edits flow through proposal → approve → apply; a
      collaborator cannot mutate a stable version directly
- [x] Field-anchored comments can be created and resolved
- [x] RFC-032 `org`-visibility import resolves to a `viewer` row in
      `contract_collaborators`
- [x] `cargo test` / `cargo check` pass; existing RFC-001 tenancy and
      RLS behavior unchanged
- [x] `docs/collaboration-reference.md` added — new user-facing
      collaborator / comment / proposal endpoints

---

## Dependency Chain

Depends on **RFC-001** (org tenancy + `get_my_org_ids()`) and
**RFC-032** (publication / `org` visibility). Last in the egress +
sharing series; ships after RFC-032.
