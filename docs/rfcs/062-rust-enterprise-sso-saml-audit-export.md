# RFC-062 — Rust Enterprise: SSO/SAML + Audit-Log Export

**Status:** Deferred
**Date:** 2026-05-27
**Branch:** n/a — design only
**Addresses:** [RFC-059](059-open-core-split.md) §"What is paid → server-enterprise"
**Depends on:** RFC-060 (LicenseManager protocol), RFC-061 (`enterprise` feature scaffold)

> First two real revenue-bearing features in
> `contractgate-server-enterprise`. SAML 2.0 SP for the dashboard API
> (alternative to Supabase JWT) and configurable audit-log export to a
> webhook or S3 bucket. Both gated by `LicenseManager`.

---

## ⚠️ Deferred — build inventory risk

SAML is a 2-week tarpit (samael wiring, IdP metadata storage, SP entity
setup, JIT user creation, dashboard SSO settings page). Audit export is
another 2 weeks (Vault wiring for S3 creds, retry/backoff state
machine, hourly S3 buffering, HMAC webhook signing). That's a month of
focused work for features no one has asked for yet.

**Implement when** [RFC-059's build trigger](059-open-core-split.md#️-build-trigger--do-not-implement-yet)
fires AND the design partner specifically named SSO/SAML or audit
export as the feature they're paying for. If they ask for something
else (RBAC, fleet management, compliance report generator), this RFC
gets superseded.

When the trigger fires, implement [RFC-060](060-license-manager-protocol.md)
+ [RFC-061](061-rust-enterprise-feature-flag.md) + this RFC together in
one or two nightlies.

---

## Why these two features first

Both come up in every enterprise sales conversation. SSO is table stakes
for any company over ~200 employees with a security review process.
Audit-log export is table stakes for anything with a compliance officer
(SOC 2, ISO 27001, HIPAA). Shipping both unlocks the first paid tier
without needing a third feature to justify the price.

Dynamic contract reload was considered for this RFC but moved to
RFC-064 (Java side) — it's a much more natural Connect-side feature
since Connect tasks are the long-running processes that benefit most
from hot reload.

---

## Feature 1: SSO / SAML 2.0

### Scope

- **In:** SAML 2.0 SP (Service Provider) for the dashboard API. JIT
  user provisioning. Per-org IdP configuration. Existing Supabase JWT
  path stays as the community auth method.
- **Out:** SCIM (separate RFC if a customer asks). OIDC IdPs other
  than SAML (most enterprise IdPs speak both; SAML covers the
  certifications). Per-user role assertions (orgs start with all SSO
  users as `member`; admin promotion is a separate manual step).

### Library choice

Use `samael` (Rust SAML library, MIT, actively maintained, used by
Outline and a few other open-source apps). Rejected alternatives:

- `saml2` (the one I tentatively listed in RFC-061): less active,
  thinner XML signature support. Update Cargo.toml in this RFC to use
  `samael` instead.
- Roll our own: SAML is a tarpit. Don't.

### Auth flow

```
1. User visits dashboard → /login
2. Dashboard detects the org's domain has SSO enabled (lookup against
   public /v1/sso/discover?email=...).
3. Dashboard redirects to backend POST /v1/sso/login?org_slug=...
4. Backend builds a SAML AuthnRequest, signs it, redirects to the IdP.
5. IdP authenticates user, POSTs SAML Response back to
   /v1/sso/callback.
6. Backend verifies signature, extracts NameID + attributes, JIT-creates
   the user + org_membership if absent, mints a Supabase JWT (using
   SUPABASE_JWT_SECRET like jwt_auth.rs does today), and redirects to
   the dashboard with the token.
7. Dashboard proceeds as if the user had logged in via Supabase
   email/password — same Authorization: Bearer flow afterward.
```

This lets us reuse all of the existing `jwt_auth.rs` machinery and the
`require_auth` middleware unchanged. SSO is "an alternative way to
obtain a JWT," not a parallel auth model.

### Per-org IdP config

New table:

```sql
-- migration 028 (next free after 027_api_keys_server_side_issuance.sql)
create table idp_configs (
  id uuid primary key default gen_random_uuid(),
  org_id uuid not null references organizations(id) on delete cascade,
  domain text not null,                    -- email domain for discovery
  idp_metadata_xml text not null,          -- SAML IdP metadata
  sp_entity_id text not null,              -- per-org SP entity ID
  attribute_email text not null default 'email',
  attribute_name text,
  default_role text not null default 'member',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  created_by uuid references auth.users(id),
  unique (org_id),                         -- one IdP per org for now
  unique (domain)
);

create index on idp_configs (domain);

alter table idp_configs enable row level security;

create policy idp_configs_select on idp_configs for select
  using (org_id = any (get_my_org_ids()));     -- per memory: use helper
create policy idp_configs_insert on idp_configs for insert
  with check (org_id = any (get_my_org_ids()));
create policy idp_configs_update on idp_configs for update
  using (org_id = any (get_my_org_ids()));
create policy idp_configs_delete on idp_configs for delete
  using (org_id = any (get_my_org_ids()));
```

RLS pattern uses `get_my_org_ids()` per memory entry
`feedback_rls_helper_required.md`. The IDP metadata XML is **not** a
secret (it's the IdP's public config); storing it in plain Postgres is
fine.

### Backend modules

```
src/enterprise/saml/
├── mod.rs              feature gate + route registration
├── sp.rs               samael SP wrapper (AuthnRequest, Response verify)
├── routes.rs           POST /v1/sso/login, /v1/sso/callback, GET /v1/sso/discover
├── jit.rs              JIT user + org_membership creation
└── admin.rs            CRUD on idp_configs (admin auth required)
```

License gate: `routes::register(router, license)` checks
`license.has("sso_saml")` and either mounts the routes or installs a
410-Gone handler with the "Enterprise license required" message. Same
pattern for audit-export below.

### Dashboard changes

- New page `/settings/sso` (admin-only, behind `<PlanGate
  required="enterprise" />` from RFC-045) — upload IdP metadata, copy
  SP entity ID / ACS URL, test login button.
- `/login` page detects SSO via the discover endpoint and adds a
  "Continue with SSO" button when the user's email domain matches an
  active IdP config.

These dashboard changes are Apache 2.0 (dashboard stays community-
licensed). Pointing the UI at an enterprise-only backend feature is
fine — the UI just shows "upgrade to Enterprise" if the backend returns
410.

### Testing

- Use `samltest.id` as a public test IdP for manual QA.
- Integration tests use a local mock IdP (`samael` ships test helpers)
  to exercise the round-trip without external dependencies.
- License gate test: with `sso_saml` absent from `features`, SSO
  endpoints return 410 with the documented error string.

---

## Feature 2: Audit-Log Export

### Scope

- **In:** Configurable real-time export of `audit_log` rows to (a) an
  HTTP webhook (POST JSON) or (b) an S3 bucket (newline-delimited JSON,
  one file per hour). Per-org config. At-least-once delivery.
- **Out:** Kafka topic destination (a customer can already consume the
  webhook into Kafka via Connect; saves us a destination to maintain).
  Splunk HEC, Datadog, etc. (each one's a separate destination; webhook
  covers them all via customer-side adapter). Historical backfill
  (export-from-now-on only; we'll add a backfill CLI in a follow-up if
  asked).

### Why two destinations, not one

- Webhook covers SIEM ingestion (Splunk, Datadog, Sumo, custom).
- S3 covers compliance archival (long-term retention, low cost).
- Most enterprise customers want one or both. A single destination
  forces them to build a fan-out themselves.

### Config schema

New table:

```sql
-- migration 029
create table audit_exports (
  id uuid primary key default gen_random_uuid(),
  org_id uuid not null references organizations(id) on delete cascade,
  destination_type text not null check (destination_type in ('webhook', 's3')),
  webhook_url text,                        -- nullable; required if type='webhook'
  webhook_secret text,                     -- HMAC signing key, optional
  s3_bucket text,                          -- nullable; required if type='s3'
  s3_prefix text not null default '',
  s3_region text,
  s3_access_key_id text,                   -- customer-supplied IAM user
  s3_secret_access_key text,               -- encrypted at rest (Supabase Vault)
  enabled boolean not null default true,
  last_exported_at timestamptz,
  last_error text,
  last_error_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (org_id, destination_type)         -- one webhook + one S3 per org
);

create index on audit_exports (org_id, enabled);

-- RLS same pattern as idp_configs above
```

S3 credentials are sensitive — use Supabase Vault for `s3_secret_access_key`
(or, if Vault isn't already wired, encrypt with a server-side key from
env and document the rotation procedure). Defer to whichever is already
in use elsewhere in the codebase; check `src/` for existing precedent
before implementing.

### Export pipeline

New module: `src/enterprise/audit_export/`

```
audit_export/
├── mod.rs              feature gate, public types
├── tail.rs             Postgres LISTEN/NOTIFY tail of audit_log inserts
├── webhook.rs          HTTP delivery worker
├── s3.rs               S3 buffered delivery worker
├── retry.rs            exponential backoff, dead-letter handling
└── admin.rs            CRUD on audit_exports
```

**Delivery model:** single tokio task per org per destination, fed by a
broadcast channel. The audit_log insert path (existing
`src/audit.rs`) gains a single line:

```rust
#[cfg(feature = "enterprise")]
let _ = state.audit_export_tx.send(audit_entry.clone());
```

The export task subscribes. If it falls behind by more than 10k events,
it drops oldest with a WARN log and a metric (`audit_export_dropped`).
This is acceptable because:

- Audit_log itself is the source of truth — exports are a convenience,
  not durability.
- Customers who need true at-least-once across restarts can do periodic
  reconciliation via the existing `/v1/audit/list` API. Document this
  in the audit-export reference doc.

**Webhook delivery:** POST JSON, HMAC-SHA256 in `X-CG-Signature` header
(format: `t=<unix>,v1=<hex>`). Retry on 5xx with exponential backoff
(1s, 5s, 30s, 5m, 30m, 2h, then dead-letter to local file + log ERROR).
Headers, body format, signature scheme documented in
`docs/audit-export-reference.md` (new doc per project rules — this is
user-facing).

**S3 delivery:** buffer in memory up to 5 MB or 1 hour, whichever first;
flush to
`s3://<bucket>/<prefix>year=YYYY/month=MM/day=DD/hour=HH/<install_id>_<seq>.jsonl.gz`.
Gzip-compressed NDJSON. Hive-style partitioning so Athena/Glue/Trino
work out of the box.

### License gate

`audit_export` module's `register(router, license, db)` is a no-op when
`license.has("audit_export") == false`. The export task is not spawned;
the admin endpoints return 410.

### Dashboard

`/settings/audit-export` page (admin-only, `<PlanGate
required="enterprise" />`): forms for webhook URL/secret and S3
credentials. Status panel shows `last_exported_at` and recent errors.

---

## Docs (per project rules)

Two new files in `docs/`:

- `docs/sso-reference.md` — SAML metadata download, attribute mapping,
  test-login procedure, troubleshooting.
- `docs/audit-export-reference.md` — webhook payload schema, HMAC
  verification example, S3 directory layout, retry semantics.

Both must exist before this RFC is "Accepted" — adds the user-facing
documentation requirement spelled out in CLAUDE.md.

---

## Migrations

- `028_idp_configs.sql`
- `029_audit_exports.sql`

Verified against `supabase/migrations/` on 2026-05-27 — last shipped is
`027_api_keys_server_side_issuance.sql`. If another migration lands
between this RFC and implementation, bump.

---

## Tests

- Integration: full SAML round-trip against `samael`'s mock IdP, asserts
  JIT user + membership created, JWT minted.
- Integration: webhook delivery with HMAC verified by a test receiver.
- Integration: S3 delivery against `minio` in docker-compose (or LocalStack).
- Integration: license gate — without `sso_saml`/`audit_export`,
  endpoints return 410 with documented error string.
- Unit: HMAC signing, retry/backoff state machine, S3 path templating.
- RLS test: a user in org A cannot read or modify org B's `idp_configs`
  or `audit_exports`.

---

## Out of scope

- OIDC IdPs (separate RFC if asked).
- SCIM provisioning (separate RFC if asked).
- Kafka/Datadog/Splunk-HEC destinations (webhook + customer-side
  adapter covers).
- Per-user role assertions from SAML (everyone JIT'd as `member`).
- Self-serve SSO billing (manual license issuance for now).

---

## Acceptance Criteria

1. `cargo build --features enterprise` succeeds; `samael` and `aws-sdk-s3`
   compile cleanly.
2. SAML round-trip works against `samltest.id` end-to-end in a manual QA
   pass.
3. Webhook export delivers an audit event to a netcat listener within
   1s of the originating ingest call, with a valid HMAC.
4. S3 export writes a gzipped NDJSON file to MinIO with the documented
   path layout.
5. Without the matching license feature, both endpoints return 410 with
   the documented error string and no DB side effects.
6. RLS tests pass.
7. `docs/sso-reference.md` and `docs/audit-export-reference.md` exist
   and are linked from the README's Enterprise section (the Enterprise
   section itself is a separate small README change).
8. `cargo test --features enterprise` passes.
9. Migrations 028 and 029 apply cleanly to a fresh DB.

**Cannot test locally:** Alex runs `cargo build --features enterprise`,
`cargo test --features enterprise`, and the manual SAML + S3 QA passes.
