# RFC-045 — Plan-Based Feature Gating

**Status:** Accepted  
**Date:** 2026-05-17  
**Branch:** `nightly-maintenance-2026-05-17-rfc045-plan-gating`

---

## Problem

ContractGate has four published pricing tiers (Self-Hosted Free, Cloud Free, Growth, Enterprise) with a clear feature matrix on `/pricing`. However, the application currently gates features only by authentication — any logged-in user on any tier can access every feature, including Growth-only capabilities like the Visual Builder, Quarantine replay, GitHub sync, Scorecard, Scaffold, Egress Validator, and the Collaborate tab.

Additionally, several features shipped since the pricing page was written (Scaffold, Scorecard, Egress Validator, Collaborate tab, CSV inference, Kafka/Kinesis tabs, subscribe-mode import, open-data fork) have no tier placement at all.

---

## Goals

1. Add a `plan` field to the `organizations` table.
2. Expose `plan` through the `useOrg()` hook.
3. Build a `<PlanGate>` component that wraps restricted UI surfaces and renders an upsell card for users below the required tier.
4. Apply gates to all Growth+ features in the dashboard.
5. Show Free-tier limit warnings (contract count, version count) as banners — not hard blocks, since enforcement is a backend concern.
6. Update the pricing page feature table to include features added since the original authoring.

---

## Non-Goals

- Backend enforcement of per-tier limits (event quotas, audit retention). That is enforced at the API/DB layer and is out of scope here.
- Billing integration (Stripe, etc.). Plan is set by admins for now; self-serve upgrade is a future RFC.
- Org switcher / multi-org UX.

---

## Schema Change — Migration 026

```sql
-- Add plan enum and column to organizations table
create type public.plan_tier as enum ('free', 'growth', 'enterprise');

alter table public.organizations
  add column plan public.plan_tier not null default 'free';
```

All existing orgs default to `free`. Admins set `growth` or `enterprise` via Supabase dashboard or a future admin API.

---

## Tier Definitions

| Key | Display name | Monthly price |
|---|---|---|
| `free` | Cloud Free | $0 |
| `growth` | Growth | $299/mo |
| `enterprise` | Enterprise | Custom |

Self-hosted is not a cloud plan — it has no `organizations` row.

---

## Feature Tier Placement

### Free (all logged-in users)
- Ingest API (all tiers)
- Playground (test without saving)
- Contracts CRUD — up to **3 contracts**, **3 versions each**
- Audit log — 7-day window (backend-enforced)
- Stream demo
- Open data catalog — **browse and fork** (good acquisition hook)
- Import from ref — **snapshot mode only**
- API key management

### Growth+ (`minTier="growth"`)
- Visual Builder tab
- Generate from Sample (JSON → YAML)
- CSV → YAML inference (From CSV tab)
- Quarantine replay
- GitHub sync
- Publish to catalog
- Semantic versioning + promotion (Versions tab — fork/promote/deprecate controls)
- Collaborate tab (AI contract proposals)
- Scorecard page
- Scaffold page (brownfield scaffolder + PII detection)
- Egress Validator (Catalog page)
- Import from ref — **subscribe mode** (snapshot stays Free)
- Kafka / Kinesis integration tabs
- Batch ingest (backend-enforced; UI surfaced in docs)
- Multi-tenancy, PII transform rules (backend)
- Team invites & roles

### Enterprise only (`minTier="enterprise"`)
- SSO / SAML
- Audit log export (S3 / GCS)
- Custom SLA
- Dedicated deployment
- Priority support + SRE on-call
- Custom contract templates

---

## `useOrg()` Change

Add `plan` to `OrgInfo`:

```ts
export interface OrgInfo {
  org_id: string;
  org_name: string;
  slug: string;
  role: "owner" | "admin" | "member";
  plan: "free" | "growth" | "enterprise";   // ← new
}
```

The membership query gains a join on `orgs(name, slug, plan)`.

---

## `<PlanGate>` Component

```tsx
<PlanGate minTier="growth" feature="Visual Builder">
  <VisualBuilder ... />
</PlanGate>
```

Renders children when `org.plan` meets `minTier`. Otherwise renders an `<UpsellCard>` showing the feature name, the required tier, and a "Upgrade to Growth →" CTA linking to `/pricing`.

Tier ordering: `free < growth < enterprise`.

---

## Free-Tier Limit Banners

When `org.plan === "free"` and the user has ≥ 3 contracts, show a non-blocking amber banner at the top of the contracts list:

> "You're using 3/3 contracts on the Free plan. Upgrade to Growth for unlimited contracts."

Similarly, inside the contract modal, show a banner when ≥ 3 versions exist.

These are UX hints only. The backend already rejects creates beyond the limit (to be added in a follow-up).

---

## Pricing Page Updates

Add rows for features missing from the current `FEATURES` array:

| Feature | Free | Growth | Enterprise |
|---|---|---|---|
| Scaffold (brownfield → YAML) | ✗ | ✓ | ✓ |
| Provider Scorecard | ✗ | ✓ | ✓ |
| Egress Validator | ✗ | ✓ | ✓ |
| Collaborate tab (AI proposals) | ✗ | ✓ | ✓ |
| Kafka / Kinesis integration | ✗ | ✓ | ✓ |
| CSV → YAML inference | ✗ | ✓ | ✓ |
| Open data catalog (browse + fork) | ✓ | ✓ | ✓ |
| Import from ref — snapshot | ✓ | ✓ | ✓ |
| Import from ref — subscribe | ✗ | ✓ | ✓ |

---

## Rollout

1. Migration 026 — add `plan` column (all existing orgs → `free`)
2. `useOrg()` — surface `plan`
3. `PlanGate` + `UpsellCard` components
4. Apply gates to all Growth+ surfaces
5. Free-tier banners on contracts page
6. Update pricing page feature table
