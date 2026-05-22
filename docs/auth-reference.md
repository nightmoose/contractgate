# Authentication Reference

**Last updated:** 2026-05-22 (RFC-047 / RFC-048 / RFC-050)

ContractGate's Rust backend supports two authentication mechanisms for the
management API.  The validation hot path (`/ingest`, `/v1/ingest`) follows the
same rules but its scope is additionally bounded by the key's
`allowed_contract_ids` list.

---

## Mechanisms

### Bearer JWT (recommended for dashboard / browser sessions)

Supabase issues an RS256 session token after sign-in.  Send it as:

```
Authorization: Bearer <token>
```

The backend verifies the token against the project's JWKS endpoint, looks up
the user's primary org membership, and treats that org as the request's
authoritative `org_id`.  No additional header is needed.

### DB-backed API key

Issue keys through the dashboard or the `/api-keys` endpoint.  Send as:

```
x-api-key: cgk_<hex>
```

Each key row stores its owning `org_id`.  All management API calls are
automatically scoped to that org.

### Legacy env-var key (`API_KEY`)

The shared env-var key is accepted for zero-downtime connector migration but
**carries no org context**.  On any org-scoped management route the backend
returns `401 Unauthorized` when this key is used without a DB-backed
`ValidatedKey` extension.  Migrate connectors to DB-backed keys before launch.

---

## Org scoping — by-ID routes (RFC-047)

Every `GET / PATCH / DELETE /contracts/{id}/…` and version route is now
org-scoped at the application layer.  The backend connects as the Supabase
service role (bypassing RLS), so the application enforces isolation itself:

- A request with a valid token whose org does not own the target contract
  receives **404 Not Found** (never 403 — UUID existence is not revealed).
- A request with no resolvable org (e.g. legacy env-var key) on a prod
  deployment receives **401 Unauthorized**.
- In dev mode (`API_KEY` unset) org scoping is disabled — all contracts are
  visible regardless of org, preserving `make demo` behaviour.

Routes that are intentionally unscoped (use their own scoping mechanism):

| Route | Scoping |
|---|---|
| `POST /ingest/{id}` | key's `allowed_contract_ids` |
| `POST /v1/ingest/{id}` | key's `allowed_contract_ids` |
| `POST /egress/{id}` | key's `allowed_contract_ids` |
| `GET /public-contracts/*` | public — no auth |
| `GET /published/{ref}` | public or link-token |
| `GET /catalog` | public |

---

## Breaking change — `x-org-id` header removed (RFC-048)

**Prior to 2026-05-22**, the backend accepted a client-supplied `x-org-id`
header as a fallback when no `ValidatedKey` was present.  This header is
**no longer accepted or trusted**.  Any client that was sending `x-org-id`
must now rely on the Bearer JWT or a DB-backed API key for org context.

| Before | After |
|---|---|
| `x-api-key: <env-var>` + `x-org-id: <uuid>` → org-scoped | `x-api-key: <env-var>` + `x-org-id: <uuid>` → **401** (org not resolved) |
| `Authorization: Bearer <jwt>` + `x-org-id: <uuid>` → org from JWT | `Authorization: Bearer <jwt>` → org from JWT ✓ (header ignored) |
| `x-api-key: <db-key>` + `x-org-id: <uuid>` → org from DB key | `x-api-key: <db-key>` → org from DB key ✓ (header ignored) |

The dashboard's `OrgProvider` has been updated accordingly — it no longer
calls `setApiOrgId` or sends `x-org-id`.

---

## CORS policy — `DASHBOARD_ORIGIN` (RFC-050)

ContractGate uses two CORS layers with different scopes:

**Authenticated surface** (`/contracts/*`, `/ingest/*`, `/egress/*`, `/v1/*`,
`/audit`, `/stats`, `/playground/*`, `/contracts/infer/*`): only origins listed
in `DASHBOARD_ORIGIN` receive an `Access-Control-Allow-Origin` header.  Requests
from any other origin receive no CORS header and are rejected by the browser.

Allowed methods: `GET, POST, PATCH, DELETE, OPTIONS`.  
Allowed headers: `Authorization, Content-Type`.

**Public surface** (`/health`, `/metrics`, `/openapi.json`, `/demo/*`,
`/public-contracts`, `/catalog`, `/published/*`): wildcard `*` — these routes
expose no tenant data and are safe to embed from any origin.

### `DASHBOARD_ORIGIN` environment variable

| Value | Behaviour |
|---|---|
| Unset | Startup warning; falls back to `http://localhost:3000` (dev only). |
| `https://app.datacontractgate.com` | Single origin allowed. |
| `https://app.example.com,https://staging.example.com` | Multiple origins, comma-separated. |

Set in production via Fly secrets — **do not commit the value to source control**:

```bash
fly secrets set DASHBOARD_ORIGIN=https://app.datacontractgate.com
```

For local development with `docker compose`, leave `DASHBOARD_ORIGIN` unset
(the fallback `http://localhost:3000` matches the default dashboard port).

---

## Summary — resolution order

1. **DB-backed API key** (`x-api-key`) — org from the key row (authoritative).
2. **Bearer JWT** (`Authorization: Bearer`) — org from the Supabase user's
   primary org membership (authoritative).
3. **Legacy env-var key** — no org; org-scoped routes return 401 in prod,
   unscoped in dev mode.
