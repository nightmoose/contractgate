# Collaboration Reference (RFC-033)

Provider-consumer collaboration lets an org that owns a contract grant scoped
roles to **other orgs** on that contract alone — without inviting them into your
org and without exposing runtime data (audit logs, quarantine events, PII salt).

---

## Role model

| Role       | Can…                                                                |
|------------|---------------------------------------------------------------------|
| `owner`    | Everything. The authoring org. Implicit — never stored in the grant table. |
| `editor`   | Open change proposals. Comment. Read. Cannot grant roles or approve proposals. |
| `reviewer` | Approve or reject open proposals. Comment. Read. Cannot grant roles or apply. |
| `viewer`   | Read the contract definition and comments/proposals. Cannot write anything. |

Roles are **strictly ordered**: owner > reviewer > editor > viewer.

A collaborator role scopes to the contract definition and its review surface only.
Collaborators **cannot** access the owner org's `audit_log`, `quarantine_events`,
or `contracts.pii_salt` — these remain owner-org-scoped by Postgres RLS.

---

## Authentication

All collaboration endpoints require `x-api-key` (the standard ContractGate header).
The org bound to the API key is the caller's org for all role checks.

---

## Endpoints

### Collaborators

#### `GET /contracts/{name}/collaborators`

List all collaborator grants on a contract.

**Minimum role:** viewer (or owner)

**Response** `200 OK`

```json
[
  {
    "contract_name": "user_events",
    "org_id": "11111111-…",
    "role": "editor",
    "granted_by": "00000000-…",
    "granted_at": "2026-05-15T12:00:00Z"
  }
]
```

---

#### `POST /contracts/{name}/collaborators`

Grant a collaborator role to another org.

**Minimum role:** owner

**Request body**

```json
{
  "org_id": "11111111-1111-1111-1111-111111111111",
  "role": "editor"
}
```

`role` must be `"editor"`, `"reviewer"`, or `"viewer"`. Posting `"owner"` is an
error — the owner role is determined by `contracts.org_id`, not this table.

If a grant already exists for the org, it is updated to the new role (upsert).

**Response** `201 Created` — the new/updated `CollaboratorRow`.

---

#### `PATCH /contracts/{name}/collaborators/{org_id}`

Change an existing collaborator's role.

**Minimum role:** owner

**Request body**

```json
{ "role": "reviewer" }
```

**Response** `200 OK` — the updated `CollaboratorRow`.

---

#### `DELETE /contracts/{name}/collaborators/{org_id}`

Revoke a collaborator grant entirely.

**Minimum role:** owner

**Response** `204 No Content`

---

### Comments

Comments are flat, optionally anchored to a specific field, and resolvable.

#### `GET /contracts/{name}/comments`

List all comments on a contract, oldest first.

**Minimum role:** viewer

**Response** `200 OK`

```json
[
  {
    "id": "aaaa…",
    "contract_name": "user_events",
    "field": "amount",
    "org_id": "11111111-…",
    "author": "alice@example.com",
    "body": "Should this allow negative values?",
    "resolved": false,
    "created_at": "2026-05-15T12:00:00Z"
  }
]
```

`field` is `null` for whole-contract comments.

---

#### `POST /contracts/{name}/comments`

Add a comment to a contract.

**Minimum role:** viewer

**Request body**

```json
{
  "field": "amount",
  "author": "alice@example.com",
  "body": "Should this allow negative values?"
}
```

`field` is optional — omit it for a whole-contract comment.

**Response** `201 Created` — the new `CommentRow`.

---

#### `POST /contracts/{name}/comments/{id}/resolve`

Mark a comment as resolved.

**Minimum role:** viewer

**Response** `200 OK` — the updated `CommentRow` with `"resolved": true`.

---

### Change Proposals

Editors propose YAML changes; reviewers or the owner approve or reject; the owner applies.
A collaborator's edit **never** lands on a stable version directly.

#### `GET /contracts/{name}/proposals`

List all change proposals for a contract, newest first.

**Minimum role:** viewer

**Response** `200 OK`

```json
[
  {
    "id": "bbbb…",
    "contract_name": "user_events",
    "proposed_by": "11111111-…",
    "proposed_yaml": "version: \"1.0\"\nname: user_events\n…",
    "status": "open",
    "decided_by": null,
    "created_at": "2026-05-15T12:00:00Z"
  }
]
```

`status` values: `"open"` → `"approved"` or `"rejected"` → `"applied"`.

---

#### `POST /contracts/{name}/proposals`

Open a new change proposal.

**Minimum role:** editor

**Request body**

```json
{
  "proposed_yaml": "version: \"1.0\"\nname: user_events\n…full contract YAML…"
}
```

**Response** `201 Created` — the new `ProposalRow` with `"status": "open"`.

---

#### `POST /contracts/{name}/proposals/{id}/decide`

Approve or reject an open proposal.

**Minimum role:** reviewer

A proposal must be in `"open"` status to be decided. Attempting to decide an
already-decided proposal returns `400`.

**Request body**

```json
{ "decision": "approved" }
```

`decision` must be `"approved"` or `"rejected"`.

**Response** `200 OK` — the updated `ProposalRow`.

---

#### `POST /contracts/{name}/proposals/{id}/apply`

Mark an approved proposal as applied.

**Minimum role:** owner

The proposal must be in `"approved"` status. The response includes `proposed_yaml` —
use this content to create a new contract version (via `POST /contracts/{name}/versions`
or `POST /contracts/deploy`).

**Response** `200 OK` — the updated `ProposalRow` with `"status": "applied"`.

---

## RFC-032 integration: `org`-visibility publications

When a contract is published with `visibility: "org"` (RFC-032) and another org
imports it via `POST /contracts/import-published`, the importer's org is
automatically granted a `viewer` collaborator role on that contract. This means
the importer can immediately use the collaboration surface (read comments, open
proposals once upgraded to `editor`) without a separate invite from the owner.

---

## Error responses

All errors follow the standard ContractGate error envelope:

```json
{ "error": "…message…" }
```

| Status | When |
|--------|------|
| `401 Unauthorized` | Missing API key, or caller has no role on this contract. |
| `400 Bad Request`  | Invalid role string, empty body, attempting to decide a non-open proposal, etc. |
| `404 Not Found`    | Contract name not found (reported as 401 to avoid enumeration). |
