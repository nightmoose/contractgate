# RFC-013: Multi-Tenant SaaS

| Status        | **Deferred entirely** (2026-04-27) — no tenants, no signup flow needed pre-customer |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 08 — Multi-Tenant SaaS                                       |
| Depends on    | **RFC-001 (org-scoped tenancy) — must be signed before work begins**   |

## Summary

Make the managed plane real. Highest blast radius of any chunk. Six
deliverables, sequenced:

1. **Multi-tenant namespace isolation** (per-org data plane separation).
2. **Usage metering service** — track API calls, contracts stored,
   connector events.
3. **Self-serve org signup + onboarding** — email verification, first
   contract wizard.
4. **API rate limiting + quota enforcement per plan tier**.
5. **Terraform provider** — manage contracts, policies, connectors as IaC.
6. **Kubernetes Operator** with CRDs (`ContractGateInstance`,
   `ContractPolicy`).

Items 5 and 6 are the largest. They depend on 1–4 stabilizing first.

## Hard prerequisites

- **RFC-001 must be signed.** Per `project_tenancy_model.md`, org-scoped
  (Option B) is decided but not signed. **No work starts until signed.**
- **sqlx 0.7 → 0.8 upgrade likely triggered.** Per `MAINTENANCE_LOG.md`'s
  deferred item, multi-tenant migrations touch every call site in
  `src/storage.rs`. RFC the upgrade as part of step 1 if not already done.

## Goals

1. Every existing data path is re-keyed onto `org_id` without breaking
   history. Audit log retains original `contract_id`; new `org_id`
   column is backfilled and indexed.
2. Per-org API-call quotas enforced with token-bucket; enforcement happens
   in middleware, not handlers.
3. Self-serve signup ships behind a feature flag (`SAAS_SIGNUP_ENABLED`)
   so on-prem deploys never expose it.
4. Terraform provider and K8s Operator are released after the data plane
   is stable on at least one staging org.
5. Validation engine p99 budget (<15ms) unaffected by tenant routing.

## Non-goals

- Database-per-tenant. v1 is row-level isolation with `org_id` everywhere.
- Cross-org contract sharing (other than templates from RFC-012).
- Enterprise SSO. v1 ships email/password + magic link only; SAML/OIDC
  is a follow-up RFC.
- Stripe billing integration. Metering ships; conversion-to-invoice
  is a follow-up.
- Region failover / multi-region. Single-region at launch.

## Decisions (recommended — flag any to override; each item below is
substantive enough to be split into its own sub-RFC if useful)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Isolation model | **Row-level `org_id` everywhere.** Single DB, single schema, indexes lead with `org_id`. Confirm RFC-001 specifies this. |
| Q2 | sqlx upgrade timing | **Step 1.** Don't migrate every call site twice. RFC the upgrade first. |
| Q3 | Metering granularity | **Per-event row in `usage_events`, hourly rollup view for billing.** |
| Q4 | Rate limit dimension | **Both org and API key.** Token bucket per (org, key) with burst = 2× steady-state. |
| Q5 | Signup auth | **Email/password + magic-link verification** in v1. SSO deferred. |
| Q6 | Org slug uniqueness | **Globally unique slugs**, reserved word list. |
| Q7 | Tier model | **Three tiers in v1**: Free (5 contracts, 100k events/mo), Team ($, 50 contracts, 5M events/mo), Business ($$, unlimited contracts, soft-cap events). |
| Q8 | Terraform provider scope | **Contracts + API keys + alert rules in v1.** Templates and orgs deferred. |
| Q9 | K8s Operator CRDs | **Two CRDs**: `ContractGateInstance` (deploys the gateway via Helm wrapper) and `ContractPolicy` (declares contracts as cluster-native objects). |
| Q10 | Operator bootstrap | **Wraps the Helm chart from RFC-010.** No reinvention. |

## Current state

- Per-contract scoping today (per `project_scoping.md`): salts, quotas,
  audit are scoped per contract.
- API keys carry `owner_id` but no `org_id` concept.
- No signup flow; keys are seeded by hand.
- No metering. `audit_log` rows are the closest analog and are
  per-contract.
- Helm chart from RFC-010 (assumed landed by the time this work starts).

## Design

### Schema migration

```sql
CREATE TABLE orgs (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        text UNIQUE NOT NULL,
    display_name text NOT NULL,
    plan        text NOT NULL DEFAULT 'free',     -- free | team | business
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE org_members (
    org_id   uuid REFERENCES orgs(id) ON DELETE CASCADE,
    user_id  uuid NOT NULL,
    role     text NOT NULL,                       -- admin | editor | viewer
    PRIMARY KEY (org_id, user_id)
);

ALTER TABLE contracts          ADD COLUMN org_id uuid REFERENCES orgs(id);
ALTER TABLE api_keys           ADD COLUMN org_id uuid REFERENCES orgs(id);
ALTER TABLE audit_log          ADD COLUMN org_id uuid;
ALTER TABLE quarantine_events  ADD COLUMN org_id uuid;
ALTER TABLE alert_rules        ADD COLUMN org_id uuid REFERENCES orgs(id);

-- backfill: synth one org per existing API key owner_id
-- (one-time data migration script)

-- enforce post-backfill
ALTER TABLE contracts          ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE api_keys           ALTER COLUMN org_id SET NOT NULL;
-- audit_log + quarantine_events left nullable for historical rows

CREATE INDEX ON contracts (org_id, name);
CREATE INDEX ON api_keys (org_id);
CREATE INDEX ON audit_log (org_id, created_at DESC);
CREATE INDEX ON quarantine_events (org_id, created_at DESC);
```

Every query that reads any of these tables grows an `AND org_id = $`
clause, sourced from the request's `org_id` (resolved from API key in
middleware).

### Metering

```sql
CREATE TABLE usage_events (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      uuid NOT NULL,
    kind        text NOT NULL,                    -- api_call | event_validated | contract_stored
    quantity    bigint NOT NULL DEFAULT 1,
    occurred_at timestamptz NOT NULL DEFAULT now()
);

CREATE MATERIALIZED VIEW usage_events_hourly AS
SELECT org_id, kind,
       date_trunc('hour', occurred_at) AS hour,
       sum(quantity) AS total
FROM usage_events
GROUP BY 1, 2, 3;
```

`usage_events_hourly` refreshes every 5 minutes (same pattern as
RFC-008's impact materialized view). Billing reports read this view.

### Rate limiting

In-memory token bucket per `(org_id, api_key_id)`, configured from the
org's plan. Limits stored in `orgs.plan` → looked up at process start +
on plan change events.

```rust
// src/rate_limit.rs
pub struct Limits {
    pub steady_per_sec: u32,
    pub burst: u32,                 // 2× steady
}

impl From<Plan> for Limits {
    fn from(p: Plan) -> Self {
        match p {
            Plan::Free     => Limits { steady_per_sec:   10, burst:   20 },
            Plan::Team     => Limits { steady_per_sec:  100, burst:  200 },
            Plan::Business => Limits { steady_per_sec: 1000, burst: 2000 },
        }
    }
}
```

429 on exhaustion, with `Retry-After` header.

### Signup flow

`dashboard/app/(public)/signup/page.tsx`:

1. Email + password + org slug.
2. POST `/auth/signup` → row in `users`, row in `orgs`, magic-link email
   via RFC-009's email transport.
3. Click verifies; first-contract wizard runs.
4. Wizard: paste a sample event → calls `POST /contracts/infer` (RFC-006)
   → previews YAML → user accepts → contract created in their org.

Feature-flagged behind `SAAS_SIGNUP_ENABLED=true`. Off in self-host.

### Terraform provider

Repo `terraform-provider-contractgate`. Resources in v1:

- `contractgate_contract`
- `contractgate_api_key`
- `contractgate_alert_rule` (RFC-009)

Implementation: HashiCorp's plugin framework, talks to the gateway via
the Go SDK (RFC-011).

### K8s Operator

Repo `contractgate-operator`. CRDs:

```yaml
apiVersion: contractgate.dev/v1
kind: ContractGateInstance
metadata: { name: prod }
spec:
  version: 0.5.0
  replicas: 3
  postgres: { externalUrl: "..." }
  ingress: { host: "gw.example.com" }

---
apiVersion: contractgate.dev/v1
kind: ContractPolicy
metadata: { name: user-events }
spec:
  contractYaml: |
    version: "1.0"
    name: user_events
    ...
  enforcement: strict      # strict | permissive | dry-run
```

Operator wraps the Helm chart from RFC-010 for `ContractGateInstance`
and pushes contracts via the gateway API for `ContractPolicy`.

## Test plan

- Migration: snapshot DB, run migration, run backfill, assert all rows
  have `org_id`, all queries pass.
- `tests/tenancy_isolation.rs` — two orgs share contract names; assert
  no cross-org reads.
- `tests/rate_limit.rs` — exceed Free tier, assert 429 + Retry-After.
- Signup flow: Playwright happy-path including magic-link click.
- Terraform: acceptance tests against a staging gateway.
- Operator: kuttl tests on a kind cluster.

## Rollout

1. **Sign off RFC-001.**
2. RFC + execute sqlx 0.7 → 0.8 upgrade.
3. `orgs`, `org_members`, `org_id` columns + backfill.
4. Middleware: resolve `org_id` from API key on every request.
5. Re-key every query in `src/storage.rs` to filter by `org_id`.
6. Soak in staging on at least one synthetic org for 1 week.
7. Metering (`usage_events`, hourly view).
8. Rate limiting middleware.
9. Signup flow + email verification + first-contract wizard.
10. Terraform provider (separate repo, depends on Go SDK from RFC-011).
11. K8s Operator (separate repo, depends on Helm chart from RFC-010).
12. `cargo check && cargo test`; full integration soak.
13. Update `MAINTENANCE_LOG.md`.

## Deferred

- Stripe billing integration (separate RFC).
- SAML / OIDC SSO (separate RFC).
- Multi-region.
- Database-per-tenant for white-glove enterprise.
