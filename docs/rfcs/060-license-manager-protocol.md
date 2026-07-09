# RFC-060 — LicenseManager Protocol + SaaS Validation

**Status:** Draft — **on the shelf**
**Date:** 2026-05-27
**Branch:** n/a — planning document
**Addresses:** [RFC-059](059-open-core-split.md) §"One protocol, two implementations"
**Depends on:** RFC-059 sign-off

> Wire protocol + offline-signing format consumed by both the Rust
> LicenseManager (RFC-061) and the Java LicenseManager (RFC-063). Also
> specifies the SaaS validation service that issues and refreshes
> licenses.

---

## ⚠️ Build trigger — do not implement yet

Same trigger as [RFC-059](059-open-core-split.md). This protocol is
inert without a paid feature to gate. Standing up the SaaS validation
service, the second Supabase project, and the Ed25519 key custody flow
is real ops cost — don't pay it until a customer has paid for what it
gates.

---

## Decision summary

- **Phone-home primary.** Enterprise builds call
  `POST https://license.datacontractgate.com/v1/validate` on startup and
  every 24h thereafter.
- **Signed-file fallback with 90-day hard expiry.** When phone-home is
  unreachable (air-gapped clusters, network outage), an Ed25519-signed
  offline token is used. The token expires 90 days after **issuance**,
  not 90 days after first use — customers must refresh by calling home
  at least quarterly. This tells us a license is in active use without
  blocking legitimate offline operation.
- **Single key, both products.** One license key validates the Rust
  enterprise server and the Java enterprise SMT independently. Each
  product reports its own `product_code` so usage attribution stays
  clean.

---

## Wire protocol

### Validation request

```
POST /v1/validate
Content-Type: application/json

{
  "license_key": "cgent_live_8f3a...",            // customer-issued key
  "product_code": "contractgate-server" | "kafka-connect-contractgate",
  "product_version": "1.4.2",
  "install_id": "9b2d6e3f-...",                   // stable UUID per install
  "cluster_id": "production-east-1" | null,       // customer-supplied tag
  "hostname": "kafka-connect-01.internal" | null, // best-effort
  "now": "2026-05-27T14:32:11Z"                   // client clock, for audit
}
```

`install_id` is generated on first run and persisted to disk
(`$XDG_DATA_HOME/contractgate/install_id` on Rust;
`$KAFKA_DATA_DIR/contractgate/install_id` on Java). It is **not** a
secret — it lets the SaaS backend distinguish "ten clusters using one
key" from "one cluster restarted ten times" for compliance/audit, not
for enforcement.

### Validation response (success)

```
HTTP/1.1 200 OK
Content-Type: application/json

{
  "status": "valid",
  "license_id": "lic_9f8e7d6c",
  "customer": "Acme Corp",
  "issued_at": "2026-04-15T00:00:00Z",
  "expires_at": "2027-04-15T00:00:00Z",
  "features": ["sso_saml", "audit_export", "dynamic_reload", "dlq_routing"],
  "max_installs": 10,
  "current_installs": 3,
  "cache_until": "2026-05-28T14:32:11Z",          // 24h from now()
  "offline_token": "eyJhbGciOiJFZERTQSI..."       // signed Ed25519 JWT, 90d
}
```

The client persists `offline_token` to disk on every successful response.
That token becomes the fallback if the next phone-home fails.

### Validation response (revoked / expired)

```
HTTP/1.1 200 OK

{
  "status": "revoked" | "expired",
  "license_id": "lic_9f8e7d6c",
  "reason": "Payment 60 days overdue.",
  "grace_until": "2026-06-15T00:00:00Z"           // optional soft window
}
```

Within `grace_until`, the client logs a WARN every hour but keeps
enterprise features enabled. After `grace_until`, features disable on
next startup or on the next 24h re-check, whichever comes first.

### Network failure

If the request times out (5s connect, 10s total) or returns 5xx, the
client falls back to the locally cached offline token. If that token is
expired or absent, the client logs ERROR and disables enterprise
features.

The client **never** crashes the host process on license failure. The
Rust server still serves community traffic; the Java SMT still validates
records — they just stop honoring enterprise-only config keys and log a
clear "Enterprise license required for X" message when one is hit.

---

## Offline token format

Standard JWT structure with Ed25519 signature (`EdDSA` alg). Public key
is baked into the Rust and Java LicenseManager binaries at build time.

Header:
```
{"alg":"EdDSA","typ":"JWT","kid":"cg-license-2026"}
```

Payload:
```
{
  "iss": "license.datacontractgate.com",
  "sub": "lic_9f8e7d6c",
  "aud": "contractgate-enterprise",
  "iat": 1715731200,
  "exp": 1723507200,                  // iat + 90 days
  "customer": "Acme Corp",
  "license_key_fingerprint": "sha256:7c4f...",
  "features": ["sso_saml", "audit_export", "dynamic_reload", "dlq_routing"],
  "license_expires_at": "2027-04-15T00:00:00Z"
}
```

Two distinct expiries:

- `exp` (90 days) — when the offline token itself stops being trusted.
  Forces a phone-home refresh.
- `license_expires_at` — when the underlying paid license ends. Used by
  the client to compute "how long until I should warn the customer."

`license_key_fingerprint` lets the client verify the offline token
matches the license key it was started with (catches accidental
key/token mismatch when an admin replaces the key).

### Key rotation

`kid` (key ID) in the JWT header lets us rotate signing keys without
breaking outstanding tokens. Clients ship with an array of trusted
public keys keyed by `kid`. Rolling a new key is a client release;
old tokens keep verifying until they expire.

---

## SaaS validation service

**Where it runs:** Fly.io, Rust (Axum), Postgres on Supabase. Same
infrastructure as the rest of ContractGate; no new ops surface. Domain:
`license.datacontractgate.com`.

**Why not just embed in the existing Rust server:** the validation
service must be reachable from customer Kafka clusters that have *no*
access to the customer's own ContractGate gateway. It is a public,
multi-tenant service with a different threat model than the
customer-deployed server.

### Endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/v1/validate` | none (license_key in body) | Validate + refresh offline token |
| GET | `/v1/health` | none | Liveness for monitoring |
| POST | `/admin/licenses` | admin JWT | Issue a new license (sales-led) |
| POST | `/admin/licenses/:id/revoke` | admin JWT | Revoke (non-payment, breach) |
| GET | `/admin/licenses/:id/usage` | admin JWT | Recent install pings, for sales |

`/admin/*` endpoints are not public; they sit behind the existing
Supabase JWT auth path used by the dashboard, gated to a single internal
org.

### Schema (Supabase migration, new table set)

```sql
create table licenses (
  id uuid primary key default gen_random_uuid(),
  license_key_hash text not null unique,         -- sha256 of the raw key
  customer_name text not null,
  customer_email text not null,
  features text[] not null default '{}',
  max_installs int not null default 10,
  issued_at timestamptz not null default now(),
  expires_at timestamptz not null,
  revoked_at timestamptz,
  revoke_reason text,
  grace_until timestamptz,
  notes text,
  created_at timestamptz not null default now()
);

create table license_pings (
  id bigserial primary key,
  license_id uuid not null references licenses(id),
  install_id uuid not null,
  product_code text not null,
  product_version text,
  cluster_id text,
  hostname text,
  client_ip inet,
  client_now timestamptz,
  server_now timestamptz not null default now()
);

create index on license_pings (license_id, server_now desc);
create index on license_pings (install_id, server_now desc);
```

`license_key_hash` (not the raw key) is stored so the DB is useless if
exfiltrated. Validation lookups hash the incoming key and compare.

### Issuance flow

Sales-led. An admin POSTs to `/admin/licenses` with customer info; the
service generates a 32-byte random key (`cgent_live_<base64url>`),
stores the SHA-256 hash, and returns the raw key once. Lost keys are
not recoverable — they get reissued and the old one revoked.

License key format: `cgent_live_` prefix (so accidental leaks in logs
are greppable) + base64url(32 random bytes). Total length 54 chars.

---

## Cache semantics

| Event | Cache state after |
|---|---|
| Successful phone-home | Offline token written to disk; in-memory `cache_until` = response value; in-memory feature list = response value. |
| Phone-home failure, valid offline token on disk | In-memory `cache_until` = offline token `exp`; features = offline token `features`. |
| Phone-home failure, expired or missing offline token | In-memory state = "no license"; enterprise features disabled; ERROR logged. |
| Startup | Read offline token from disk, set in-memory state from it, **then** phone home in background. Don't block startup on phone-home — community traffic must serve immediately. |
| Every 24h while running | Phone home, refresh offline token on success. |
| Operator runs `contractgate license refresh` (Rust) or sends a Connect REST API call (Java) | Force immediate phone-home. |

**Critical:** the 24h cache applies to *successful* validations. A failed
or revoked response is honored immediately, not cached for 24h. Otherwise
revoking a license takes a day to take effect.

---

## Failure-mode logging

Every state change emits a single, clear, greppable log line at the
right level. Examples:

```
INFO  contractgate::license: License validated for Acme Corp; expires 2027-04-15; features=[sso_saml,audit_export]
WARN  contractgate::license: Phone-home failed (timeout); falling back to offline token (valid until 2026-08-25)
WARN  contractgate::license: License in grace period; expires 2026-06-15; renew now to avoid downtime
ERROR contractgate::license: No valid license; enterprise features disabled. Set CONTRACTGATE_LICENSE_KEY or see https://datacontractgate.com/license
ERROR contractgate::license: Enterprise feature 'sso_saml' requested but not licensed; rejecting config
```

These strings are part of the public contract — they're greppable in
customer monitoring. Changing them is a breaking change, called out in
release notes.

---

## Client configuration

Both clients accept the same two inputs:

| Source | Rust env var / config | Java env var / config |
|---|---|---|
| License key | `CONTRACTGATE_LICENSE_KEY` env, or `license.key` in `config.toml` | `CONTRACTGATE_LICENSE_KEY` env, or `license.key` in the Connect SMT config |
| Cluster tag (optional) | `CONTRACTGATE_CLUSTER_ID` env | `CONTRACTGATE_CLUSTER_ID` env |
| Validation endpoint override (testing) | `CONTRACTGATE_LICENSE_URL` env | `CONTRACTGATE_LICENSE_URL` env |
| Offline token cache path (testing) | `CONTRACTGATE_LICENSE_CACHE_DIR` env | `CONTRACTGATE_LICENSE_CACHE_DIR` env |

Defaults: license URL = `https://license.datacontractgate.com`, cache
dir = OS-appropriate data dir.

Community builds **must not** read these env vars or config keys — the
LicenseManager doesn't exist in those builds. Setting them in a
community build is a silent no-op (env vars are just env vars), which
is the correct behavior.

---

## Security considerations

- **No customer secrets in pings.** Hostnames are best-effort; admins
  who object can set them to anonymous strings via env var.
- **TLS required.** Clients refuse to use a non-HTTPS endpoint unless
  `CONTRACTGATE_LICENSE_URL` overrides it (test/dev only).
- **No PII in logs.** Customer name is logged once on startup; subsequent
  logs use `license_id`.
- **Signing key compromise plan:** rotate `kid`, ship a client patch
  release within 24h, invalidate all outstanding offline tokens with the
  compromised `kid` server-side, force phone-home for all clients via
  shortened `cache_until`. Documented in the security runbook (separate
  doc, not blocking this RFC).

---

## Acceptance Criteria

This RFC is accepted when:

1. SaaS validation service deployed to staging on Fly.io.
2. Postgres schema migrated on the license-service Supabase project
   (separate project from the main app, isolation requirement).
3. `/v1/validate` returns valid responses for a manually-issued test
   license; revoked + expired paths verified.
4. Ed25519 keypair generated (signing key in 1Password ops vault; public
   key checked into repo at `crypto/license-signing-key-2026.pub`).
5. Integration test: a Rust binary stub and a Java binary stub each
   validate a real license end-to-end (test runs in CI against staging).
6. Rust LicenseManager (RFC-061) and Java LicenseManager (RFC-063) are
   unblocked.

Production cutover happens with RFC-062 when the first paying feature
ships.
