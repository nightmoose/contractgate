# Plan Gating — Reference

**RFC:** 045  
**Status:** Accepted  
**Added:** nightly-maintenance-2026-05-17-rfc045-plan-gating; doc added 2026-05-24

---

## Overview

ContractGate has three billing tiers with a tiered feature set. Plan gating is
enforced in the dashboard UI by the `<PlanGate>` component — users below the
required tier see an upsell card instead of the gated feature. Backend
enforcement of per-tier event quotas and audit retention is separate (handled
at the API/DB layer; not covered here).

---

## Tiers

| Key | Display name | Monthly price |
|---|---|---|
| `free` | Cloud Free | $0 |
| `growth` | Growth | $299/mo |
| `enterprise` | Enterprise | Custom |

Tier ordering: `free < growth < enterprise`. A Growth user has access to all
Free features; an Enterprise user has access to all Growth and Free features.

Self-hosted deployments have no `organizations` row and are not subject to plan
gating.

---

## Feature tier matrix

### Free (all authenticated users)

- Ingest API
- Playground (test without saving)
- Contracts CRUD — up to **3 contracts**, **3 versions each**
- Audit log — 7-day retention window (backend-enforced)
- Live stream demo
- Open data catalog — browse and fork
- Import from ref — snapshot mode only
- API key management

### Growth+ (`minTier="growth"`)

- Visual Builder tab
- Generate from Sample (JSON → YAML inference)
- CSV → YAML inference (From CSV tab)
- Quarantine replay
- GitHub sync
- Publish to catalog
- Semantic versioning + promotion controls (Versions tab)
- Collaborate tab (AI contract proposals)
- Provider Scorecard page
- Scaffold page (brownfield scaffolder + PII detection)
- Egress Validator (Catalog page)
- Import from ref — subscribe mode
- Kafka / Kinesis integration tabs
- Batch ingest (backend limit; UI surfaced in docs)
- Multi-tenancy, PII transform rules (backend)
- Team invites and roles

### Enterprise only (`minTier="enterprise"`)

- SSO / SAML
- Audit log export (S3 / GCS)
- Custom SLA
- Dedicated deployment
- Priority support and SRE on-call
- Custom contract templates

---

## Schema

Migration `026_plan_tier.sql` adds:

```sql
CREATE TYPE public.plan_tier AS ENUM ('free', 'growth', 'enterprise');

ALTER TABLE public.organizations
  ADD COLUMN plan public.plan_tier NOT NULL DEFAULT 'free';
```

All existing orgs were migrated to `free`. Admins set `growth` or `enterprise`
via the Supabase dashboard or a future admin API.

---

## Frontend: `useOrg()` hook

The `plan` field is exposed through the `useOrg()` hook:

```ts
import { useOrg } from "@/lib/org";

const { org } = useOrg();
// org.plan is "free" | "growth" | "enterprise"
```

`OrgInfo` shape:

```ts
interface OrgInfo {
  org_id: string;
  org_name: string;
  slug: string;
  role: "owner" | "admin" | "member";
  plan: "free" | "growth" | "enterprise";
}
```

The `planAtLeast(actual, required)` utility returns `true` when `actual` meets
or exceeds `required` in the tier order.

---

## Frontend: `<PlanGate>` component

```tsx
import PlanGate from "@/components/PlanGate";

// Tab/section-level gate — shows a compact lock card:
<PlanGate minTier="growth" feature="Visual Builder">
  <VisualBuilder />
</PlanGate>

// Full-page gate — shows an illustrated preview with upgrade CTA:
<PlanGate minTier="growth" feature="Scorecard" previewKey="scorecard">
  <ScorecardContent />
</PlanGate>
```

Props:

| Prop | Type | Required | Description |
|---|---|---|---|
| `minTier` | `"growth"` \| `"enterprise"` | Yes | Minimum plan required. |
| `feature` | string | Yes | Human-readable feature name shown in the upsell card. |
| `previewKey` | string | No | Key into the `PREVIEWS` registry. When supplied, shows an illustrated preview (same as the logged-out AuthGate view) with an upgrade CTA instead of the sign-in buttons. Use this for full pages. |
| `children` | ReactNode | Yes | Content shown to users on or above `minTier`. |

The component renders nothing while the org is still loading, to avoid a flash
of the upsell.

---

## Frontend: `<FreeLimitBanner>` component

Shows a non-blocking amber banner when a Free org is at a hard limit:

```tsx
import { FreeLimitBanner } from "@/components/PlanGate";

<FreeLimitBanner current={contractCount} max={3} resource="contracts" />
```

The banner is shown only when `org.plan === "free"` and `current >= max`.
It links to `/pricing` with an "Upgrade to Growth" CTA.

---

## Free-tier hard limits (backend-enforced)

| Resource | Free limit |
|---|---|
| Contracts per org | 3 |
| Versions per contract | 3 |
| Audit log retention | 7 days |

These are enforced at the API layer. The dashboard shows limit banners as a UX
hint before the hard limit is hit.

---

## Admin: setting a plan

Until a self-serve upgrade flow is available, plan changes are made directly in
the Supabase dashboard:

```sql
UPDATE organizations SET plan = 'growth' WHERE id = 'your-org-id';
```

Or use the Supabase Table Editor on the `organizations` table.

---

## Related

- [RFC-045](rfcs/045-plan-gating.md) — design rationale and acceptance criteria.
- [Pricing page](https://app.datacontractgate.com/pricing) — full feature comparison.
