# ContractGate — Security Overview

**Last updated:** 2026-07-14
**Audience:** prospective customers (security questionnaires), design partners, acquirer diligence.
**Scope:** hosted ContractGate (Rust validation gateway + Next.js dashboard + Supabase Postgres).

This is a one-page summary. Each section links to the authoritative reference,
RFC, migration, or test that implements the control. Where a control is a
deliberate design trade-off rather than a gap, that is stated explicitly.

---

## 1. Architecture in one line

Producer → (API key / JWT resolves **org**) → per-org **contract** → **validate**
(sub-millisecond, panic-free Rust hot path) → **audit** + **quarantine** on
failure. Bad events are stopped *before* the warehouse.

- Validation engine: Rust (Axum), compile-once / validate-many. Target **<15 ms
  p99**; measured p99 ~31 µs (2026-05-24 perf test). The request path is
  panic-free by policy (RFC-067 converted latent `unwrap`/`expect` to clean 4xx).
- Backend hosting: Fly.io (`primary_region = iad`, US East).
- Data store: Supabase Postgres (project `contract gate`, `us-east-2`, PG 17).
- Dashboard: Next.js on Vercel.

---

## 2. Authentication

Two mechanisms for the management API; the validation hot path adds a per-key
contract scope on top. Full detail: [`docs/auth-reference.md`](./auth-reference.md).

| Mechanism | Header | Org source |
|---|---|---|
| Supabase Bearer JWT (RS256) | `Authorization: Bearer <token>` | user's primary org membership, verified against project JWKS |
| DB-backed API key | `x-api-key: cg_live_<48 hex>` | owning `org_id` on the key row |

- **API-key issuance is server-side only** (RFC-056): the Next.js server
  generates the raw key with a CSPRNG, hashes it server-side, and returns the
  raw value exactly once. It is never stored and cannot be retrieved again.
  Key hashing algorithm/length are guarded by DB triggers (migrations 024/025)
  and an immutability guard. See [`docs/key-management-reference.md`](./key-management-reference.md).
- **No env-var master key.** The legacy `API_KEY` master credential was removed
  (RFC-066). The only way to disable auth is `CONTRACTGATE_DEV_NO_AUTH=1`, which
  defaults off and must never be set in production.
- **Client-supplied `x-org-id` is not trusted** (RFC-048). Org context comes only
  from a verified JWT or DB key. The header is honored solely in dev-no-auth mode.

---

## 3. Tenant isolation (multi-org)

The backend connects as the Supabase **service role** (bypassing RLS), so
isolation is enforced at the **application layer** and, for browser/PostgREST
access, by **row-level security**.

- **By-ID routes** (`GET/PATCH/DELETE /contracts/{id}/…`, version routes) are
  org-scoped in the app layer (RFC-047). A caller whose org does not own the
  target gets **404** (never 403 — UUID existence is not revealed). No resolvable
  org on a prod deployment → **401**.
- **Hot-path routes** (`/ingest`, `/v1/ingest`, `/egress`) are additionally bounded
  by the key's **`allowed_contract_ids`** list, enforced on every hot path
  (RFC-065). A latent cross-tenant write (handlers resolved `org_id` but passed
  `None` to storage) was found by inspection and fixed (RFC-074).
- **RLS** for browser/PostgREST access uses the `public.get_my_org_ids()` helper
  (SECURITY DEFINER) to avoid the Postgres 42P17 policy-recursion class. Org
  policies call it as the querying user.
- **Isolation is tested in CI**: self-seeding cross-org DB tests run on every PR
  in the `migrations-check` lane (RFC-068), extended to the write-side
  version-mutation surface (RFC-070).

**Known limitation (in progress):** the compose-smoke isolation lane ran under
`CONTRACTGATE_DEV_NO_AUTH=1` and was a false green; it is disabled pending an
**auth-on** end-to-end isolation lane with a no-key→401 sanity gate
([RFC-075](./rfcs/075-auth-on-isolation-test-lane.md), Draft). The DB-level
isolation tests above remain active and authoritative in the meantime.

---

## 4. Data protection — PII

Two-sided PII guarantee using one transform engine (`src/transform.rs`) and a
per-contract `pii_salt`. Detail: [`docs/pii-masking-reference.md`](./pii-masking-reference.md).

- **Ingest (RFC-004):** declared PII is transformed so raw PII never lands in
  durable storage (audit/quarantine).
- **Egress (RFC-030):** raw PII and undeclared internal fields never leave the API.
- Deterministic hashing: a value hashed on ingest matches on egress, so
  downstream joins on hashed keys stay consistent.

---

## 5. Network / input hardening

- **SSRF (RFC-049):** URL-based contract inference blocks redirect-based SSRF
  bypass (private-range and redirect-to-internal). Treated as a P0 launch blocker
  and closed before public launch.
- **CORS (RFC-050):** authenticated surface returns `Access-Control-Allow-Origin`
  only for origins in the `DASHBOARD_ORIGIN` allowlist (Fly secret, not committed).
  Public, tenant-free routes (`/health`, `/metrics`, `/openapi.json`, `/catalog`,
  `/published/*`) use wildcard by design.
- **Idempotency:** replayed ingest is deduped via an idempotency key with a TTL
  sweep (Supabase scheduled function); prevents double-processing without
  unbounded key growth.

---

## 6. Supabase security posture

- Migration `031_security_advisor_fixes.sql` (applied to prod 2026-07-11) closed
  all ERROR-level advisor items: `security_invoker=true` on views that would
  otherwise leak cross-org rows, dropped a blanket `authenticated` policy on
  `provider_field_baseline`, revoked PUBLIC execute on trigger/helper functions,
  and pinned `search_path` on eight trigger functions.
- **Residual advisor items are intentional or a console toggle** (re-checked
  2026-07-14, zero ERRORs):
  - `get_my_org_ids()` remains executable by `authenticated` **by design** — org
    RLS policies depend on it. Revoking it would break tenancy.
  - `early_access` public-waitlist INSERT policy is intentional.
  - Four service-role-only tables (`idempotency_keys`, `public_contracts`,
    `stripe_processed_events`, `stripe_failed_events`) have RLS enabled with no
    anon/authenticated policy — deny-by-default is the intended posture
    (documented via `COMMENT ON TABLE`).
  - Auth "leaked password protection" (HaveIBeenPwned check) is a paid-plan Auth
    feature; not available on the current Supabase plan. Not an open action.

---

## 7. Supply chain & SDLC

From [`SECURITY.md`](../SECURITY.md):

- GitHub secret scanning + push protection; CodeQL static analysis.
- `cargo audit` + `cargo deny` and Dependabot in CI.
- Signed commits required on `main`; branch protection enabled.
- Coverage ratchet gate (RFC-071) and a migration-drift check
  ([`docs/migration-drift-check-reference.md`](./migration-drift-check-reference.md)).

---

## 8. Data retention & residency

- **Residency:** application data in Supabase Postgres `us-east-2`; compute on
  Fly.io `iad`. US-only today.
- **Classes:** contracts/versions, audit log, quarantined events, API-key hashes
  (never raw keys), Stripe event-idempotency records.
- **Retention:** idempotency keys expire via TTL sweep. Audit and quarantine data
  are operator-controlled today — **no automated blanket TTL/purge job exists
  yet**. A documented retention policy is a recommended pre-enterprise-sale item
  (owner: Alex).

---

## 9. Vulnerability reporting

Preferred: GitHub private vulnerability reporting. Alternative:
`security@datacontractgate.com`. Acknowledgement target 48 h; fix-timeline target
7 days; coordinated disclosure (public after fix or 90 days). Full policy:
[`SECURITY.md`](../SECURITY.md).

---

## 10. Not yet in place (honest gaps)

- SOC 2: not started. No audit or Type I/II report today. (Recommend a
  "security posture / SOC2-readiness" note rather than a committed timeline.)
- SSO / SAML: not built (RFC-062, deferred until a named deal requires it).
- Auth-on end-to-end isolation lane: RFC-075 Draft (§3).
- Automated data-retention/purge policy: to be defined (§8).

---

*Maintainers: keep this current when landing RFC-075, enabling leaked-password
protection, or changing the auth/isolation model. Link new controls to their RFC
or migration.*
