# API Key Management Reference

**Added:** RFC-056 (2026-05-24)  
**Status:** Stable

---

## Overview

ContractGate API keys authenticate Kafka connectors, CLI tools, and any
server-to-server caller against the validation backend (`src/api_key_auth.rs`).

As of RFC-056, key issuance and revocation are **server-side only**.  The
browser never generates or hashes a raw key.  All write operations go through
the Next.js route handler at `dashboard/app/api/keys/route.ts`, which uses the
Supabase service role and enforces org scoping from the session.

---

## Key format

```
cg_live_<48 lowercase hex chars>   (56 chars total)
```

- **24 random bytes** from Node.js `crypto.randomBytes` → 48 hex chars → 192 bits of entropy.
- **`key_prefix`** = first 12 characters (e.g. `cg_live_ab12`).  Stored in plain text; used as an O(1) discriminator for the DB lookup.
- **`key_hash`** = `base64( sha256(raw_key_bytes) )` — standard base64, no line breaks.  This is the value stored in `api_keys.key_hash` and verified by `api_key_auth.rs`.

The raw key is **never** stored anywhere.  It is returned exactly once in the `POST /api/keys` response.

---

## Dashboard routes

Both routes require:
- A valid Supabase session cookie.
- A same-origin `Origin` or `Referer` header (CSRF guard).
- The `org_id` is resolved server-side from `org_memberships`; no client-supplied org is trusted.

### `POST /api/keys`

Issue a new API key.

**Request body**

```json
{ "name": "Production S3 connector" }
```

| Field  | Type   | Required | Notes                         |
|--------|--------|----------|-------------------------------|
| `name` | string | yes      | Human label; max 80 chars     |

**Response `200 OK`**

```json
{
  "id":         "<uuid>",
  "name":       "Production S3 connector",
  "key_prefix": "cg_live_ab12",
  "created_at": "2026-05-24T00:00:00Z",
  "raw_key":    "cg_live_ab12cd34ef56..."
}
```

`raw_key` is returned **exactly once**.  Store it in a secrets manager
immediately; it cannot be retrieved again.

**Error responses**

| Status | Meaning                                          |
|--------|--------------------------------------------------|
| 400    | Missing or invalid `name`                        |
| 401    | No valid session                                 |
| 403    | Cross-origin request rejected (CSRF)             |
| 404    | No org found for the session user                |
| 500    | DB insert failed                                 |

---

### `DELETE /api/keys`

Revoke an existing key by UUID.

**Request body**

```json
{ "id": "<api_key uuid>" }
```

**Response `204 No Content`** on success.

Revocation is immediate in the database.  The in-process cache in
`api_key_auth.rs` has a 60-second TTL, so a revoked key may continue to
authenticate for up to 60 seconds after the `DELETE` returns.

**Error responses**

| Status | Meaning                                          |
|--------|--------------------------------------------------|
| 400    | Missing or invalid `id`                          |
| 401    | No valid session                                 |
| 403    | Cross-origin request rejected (CSRF)             |
| 404    | Key not found or belongs to a different org      |
| 409    | Key already revoked                              |
| 500    | DB update failed                                 |

---

## Using a key

Pass the raw key in the `x-api-key` header on every request to the Rust backend:

```
x-api-key: cg_live_<48 hex chars>
```

The backend verifies via `api_key_auth.rs`:
1. Sanity-checks length and `cg_` prefix.
2. Looks up the single candidate row by `key_prefix` (indexed; fast).
3. Computes `base64(sha256(raw_key))` and compares to `key_hash`.
4. Returns `ValidatedKey { org_id, allowed_contract_ids, rate_limit_rps, … }`.

Validated keys are cached for 60 seconds (keyed by `sha256(raw_key)` hex, not the plaintext).

---

## RLS posture (after migration 027)

| Role              | SELECT metadata | SELECT `key_hash` | INSERT | UPDATE (`revoked_at`, `last_used_at`) |
|-------------------|-----------------|-------------------|--------|---------------------------------------|
| `authenticated`   | ✓ (own org)     | ✓ (column exposed, but dashboard query omits it) | ✗ | ✗ |
| `service_role`    | ✓ (all)         | ✓ (all)           | ✓      | ✓                                     |

The dashboard `SELECT` query explicitly omits `key_hash` so it is never sent to
the browser.  Column-level GRANT revocation is deferred to a future
security-hardening migration (noted in migration 027).

---

## Database table

`public.api_keys` — key columns:

| Column                | Type          | Notes                                              |
|-----------------------|---------------|----------------------------------------------------|
| `id`                  | uuid (PK)     | Returned on creation                               |
| `user_id`             | uuid (FK)     | Auth user who created the key                      |
| `org_id`              | uuid (FK)     | Org scoping; resolved server-side from session     |
| `name`                | text          | Human label                                        |
| `key_prefix`          | varchar(12)   | First 12 chars of raw key; plain text              |
| `key_hash`            | text          | `base64(sha256(raw_key))`; 44 chars (CHECK enforced) |
| `allowed_contract_ids`| uuid[]        | NULL = unrestricted; non-null = contract allowlist |
| `rate_limit_rps`      | integer       | NULL = default (100)                               |
| `rate_limit_burst`    | integer       | NULL = default (1000)                              |
| `created_at`          | timestamptz   |                                                    |
| `last_used_at`        | timestamptz   | Updated async on each validated request            |
| `revoked_at`          | timestamptz   | NULL = active                                      |
| `deleted_at`          | timestamptz   | Soft delete                                        |
| `is_active`           | boolean (gen) | `revoked_at IS NULL`                               |
