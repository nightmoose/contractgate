# RFC-096 — AISchemaGate Frontend Repo + DNS + CORS Allowlist

**Status:** Draft
**Date:** 2026-10-11
**Depends on:** RFC-039 (Supabase-JWT dashboard auth), RFC-050 (CORS origin allowlist), RFC-056 (server-side API key issuance), RFC-093 (egress as first-class door), RFC-094 (LLMGate SDK + aischemagate alias package), RFC-095 (aischemagate.dev docs portal)
**Companion RFCs:** RFC-094 (SDK), RFC-095 (docs)

---

## Problem

The AISchemaGate dev brief (2026-10-11) requires a second "front door"
alongside DataContractGate. Specifically:

- A new marketing site at `aischemagate.com`.
- A developer-facing docs portal at `aischemagate.dev` (RFC-095 owns the
  content; this RFC owns the deploy).
- The brief constrains both to share the DataContractGate backend, tenant
  service, and API-key issuance — no new auth path.

What this RFC must answer concretely:

1. **Where does the frontend live?** A new sibling repo
   (`nightmoose/aischemagate-www`), a new path in this repo
   (`frontend/aischemagate/`), or a new workspace under `dashboard/`? Each
   has consequences for ownership, deploy pipeline, and CI boundaries.
2. **What stack does it use?** Dashboard is Next.js 15 + Tailwind. The
   brief says "clone the primary UI frontend template," which points at
   `dashboard/` as the template. But `dashboard/` is a logged-in product;
   `aischemagate.com` is public marketing + lead capture. Different concerns.
3. **How do the two domains share tenant / API keys?** `datacontractgate.com`
   signs users in via Supabase JWT (RFC-039). API keys are `cg_live_...` from
   RFC-056. Can an aischemagate.com user hit the same key issuance path?
4. **CORS.** The gateway's protected routes read `DASHBOARD_ORIGIN`
   (comma-separated) from env (RFC-050, `src/main.rs:1258`). Adding
   `https://aischemagate.com` and `https://aischemagate.dev` is a one-env
   change — zero code. But that is still a config / infra deliverable that
   this RFC must enumerate.
5. **DNS and TLS**. Both domains need registration (if not already) and
   TLS termination at whatever edge the frontend uses.
6. **The dashboard experience.** When a user lands on
   `aischemagate.com/dashboard` after signup, do they go to a separate
   AISchemaGate-branded dashboard, or do they 301 to
   `app.datacontractgate.com` (the existing dashboard)? The brief implies
   shared dashboard; a reskin inside the shared dashboard is deferred.

RFC-096 answers all six and sequences the deploy so RFC-094 and RFC-095
can light up `aischemagate.com` and `aischemagate.dev` without touching
the shared backend's auth, tenancy, or data plane.

## Goal

Stand up the AISchemaGate frontend as its own repo, deployed to
`aischemagate.com` and `aischemagate.dev`, serving marketing + the docs
portal (RFC-095) and routing authenticated users to the shared
`app.datacontractgate.com` dashboard.

**Done means:**

1. A new sibling repo `nightmoose/aischemagate-www` exists, Next.js 15 +
   Tailwind, cloned-and-reskinned from the `dashboard/` template at a
   known commit, with its own `README.md` naming this RFC as the design.
2. `aischemagate.com` serves the marketing site, with hero / feature-grid
   copy taken verbatim from RFC-095 Appendix A.
3. `aischemagate.dev` serves the docs portal, with MDX rendering of
   `docs/llm-output-validation.md` from this repo (per RFC-095 §2 Option
   B). `aischemagate.com/docs` 301s to `aischemagate.dev/docs`.
4. The "Sign in" and "Dashboard" links on `aischemagate.com` deep-link to
   `app.datacontractgate.com` for the existing Supabase sign-in flow
   (RFC-039). No new auth code.
5. Gateway `DASHBOARD_ORIGIN` env var is extended to include
   `https://aischemagate.com` and `https://aischemagate.dev`. A smoke test
   proves a request from aischemagate.com with the user's `cg_live_...`
   key reaches `/egress/{contract_id}` without a CORS failure.
6. Lead-capture forms (if any — see Q3) submit to the existing lead-intake
   path (RFC-084's Slack bot) with `source = 'aischemagate.com'` tagged.
7. No shared-code duplication beyond the Tailwind config and shared
   components; `aischemagate-www` owns its own layout, pages, and copy.

## Non-goals

- **Modifying the DataContractGate dashboard brand.** The shared dashboard
  stays `app.datacontractgate.com` with existing branding. Dashboard reskin
  conditional on tenant (so an AISchemaGate signup sees different nav /
  branding inside the dashboard) is a follow-up RFC, not this one.
- **New tenant model.** Shared Supabase auth; same `cg_live_...` keys. A
  user who signs up at aischemagate.com and a user who signs up at
  datacontractgate.com land in the same tenant table. Marketing attribution
  is a `source` column, not a separate tenant pool.
- **Rebranding `contractgate` core packages.** PyPI `contractgate` and npm
  `@nightmoose/contractgate-sdk` keep their names. `aischemagate` is a
  sister alias (RFC-094).
- **Moving existing docs under aischemagate.dev.** `app.datacontractgate.com/llm-integration.md`
  (RFC-089) stays where it is; aischemagate.dev adds the output-validation
  playbook (RFC-095) and does not scoop up the ingest playbook.
- **Mirroring the AISchemaGate frontend inside this repo.** The frontend
  lives in a sibling repo. This repo only ships: the Appendix A copy
  table in RFC-095, the drift gate for the docs it exports, and the
  `DASHBOARD_ORIGIN` env change in deploy configuration (if that
  configuration is tracked here).
- **A separate deploy of the Rust gateway for AISchemaGate.** One backend,
  one gateway, two frontends.
- **DNS registration automation.** The two domains are purchased /
  configured by the human; this RFC states what names and records are
  needed, but does not script DNS.
- **A new quarantine / audit UI inside aischemagate.com.** Users reach
  those through the shared dashboard. If AISchemaGate wants its own
  quarantine UX later, that is a follow-up.

## Design

### 1. Repo layout — new sibling repo

```
nightmoose/aischemagate-www/
├── README.md                   # names RFC-096 as the design
├── package.json                # next, tailwindcss, react; same majors as dashboard/
├── next.config.js
├── tailwind.config.js
├── src/
│   ├── app/
│   │   ├── (marketing)/        # landing, pricing-redirect, lead capture
│   │   │   ├── page.tsx        # hero + feature grid (RFC-095 Appendix A)
│   │   │   └── layout.tsx
│   │   └── docs/
│   │       └── [...slug]/      # MDX render of docs/llm-output-validation.md
│   ├── components/             # <CodeTabs>, <ContractExample>, <SignInLink>
│   └── lib/
│       ├── mdx.ts              # MDX loader
│       └── docs-source.ts      # pulls doc content from contractgate repo
├── scripts/
│   └── sync-docs.ts            # fetches docs/llm-output-validation.md
│                               # from a contractgate tag; writes under content/
├── content/
│   └── llm-output-validation.md    # the synced doc (checked in)
├── public/
│   └── _redirects              # 301 /docs → aischemagate.dev/docs
└── .github/workflows/
    ├── sync-docs.yml           # weekly cron + manual trigger
    └── deploy.yml              # Vercel or Netlify; whichever matches dashboard
```

Why a new repo, not a path in this repo:

- `dashboard/` is a logged-in product with 300+ components. Mixing a
  public marketing site into the same build inflates bundle size and
  confuses CI gates (dashboard's RFC-091 "What's this?" help mode would
  pull into the public site unless we aggressively tree-shake).
- Marketing / docs changes merge faster than engine changes. Keeping them
  in a separate repo lets the marketing team ship copy without touching
  the Rust / dashboard CI matrix.
- The docs sync is a one-way pull from contractgate; the frontend never
  pushes back. Separate repo makes that boundary clear.

Why "clone the dashboard template" nonetheless: Tailwind config, color
palette, typography scale, and the dashboard's existing components for
code blocks and callouts carry over. Clone-and-strip, not import-and-fork.

### 2. Docs sync (RFC-095 §Design 2 Option B)

- `contractgate` repo tags a release (`v0.3.0`) that includes
  `docs/llm-output-validation.md`.
- `aischemagate-www/scripts/sync-docs.ts` runs on a weekly cron and on
  manual dispatch. It fetches the tagged doc, writes it under
  `content/llm-output-validation.md`, and commits the delta if any.
- The commit is PR-reviewed in `aischemagate-www`. No auto-merge; a human
  verifies the rendered output before deploying.
- Rejected alternative: git submodule. Submodules are a pain in CI, and
  the content is small enough that checked-in snapshots are simpler.

### 3. Auth and tenant sharing

- The "Sign in" button on aischemagate.com links to
  `https://app.datacontractgate.com/signin?from=aischemagate` (query param
  for marketing attribution).
- The existing Supabase signin flow (RFC-039) runs at
  `app.datacontractgate.com`. No change.
- After signin, the dashboard lands users at
  `app.datacontractgate.com/dashboard`. There is no AISchemaGate-branded
  dashboard view in v1.
- The `from=aischemagate` query param is logged into the user row on
  first signin as `signup_source`. New column:
  `ALTER TABLE org_members ADD COLUMN signup_source TEXT;` on the
  dashboard-schema migration track. (Draft; migration ownership belongs to
  the dashboard team, not this RFC — flag in Q4.)

### 4. CORS allowlist change

Gateway reads `DASHBOARD_ORIGIN` (comma-separated) and builds an
`AllowOrigin::list` layer (RFC-050, `src/main.rs:1258`). Operations change
only:

```
# before
DASHBOARD_ORIGIN=https://app.datacontractgate.com

# after
DASHBOARD_ORIGIN=https://app.datacontractgate.com,https://aischemagate.com,https://aischemagate.dev
```

Deploy order:

1. Add the two new origins to `DASHBOARD_ORIGIN` in staging.
2. Smoke-test from an `aischemagate.com` deploy preview: a
   `fetch('https://app.datacontractgate.com/egress/{contract_id}', ...)`
   with `x-api-key: cg_live_...` succeeds (200 + no CORS error).
3. Add the two origins to production `DASHBOARD_ORIGIN`.
4. Flip DNS for aischemagate.com / aischemagate.dev.

No gateway code change. No restart beyond re-reading the env.

### 5. DNS and TLS

Required records (final targets depend on the chosen edge, likely Vercel):

| Name | Type | Target | Notes |
|---|---|---|---|
| `aischemagate.com` | A/ALIAS | edge platform | Marketing + lead capture |
| `www.aischemagate.com` | CNAME | `aischemagate.com` | 301 redirect |
| `aischemagate.dev` | A/ALIAS | edge platform | Docs portal |
| `www.aischemagate.dev` | CNAME | `aischemagate.dev` | 301 redirect |

TLS: automatic cert issuance on the edge platform.

The two domains must be purchased before deploy. Flag in Q1 whether
they are already owned.

### 6. Edge platform

Draft recommendation: **Vercel**, matching dashboard. One project per
domain so logs / analytics are separable. Shared Vercel org for billing.

Alternative: Netlify, Cloudflare Pages. Any of them work; the decision
should match whatever operates the dashboard today.

### 7. Lead capture

The brief does not explicitly require a lead-capture form on
aischemagate.com, but a signup CTA is implicit. Draft design:

- "Try AISchemaGate free" button → `app.datacontractgate.com/signup?from=aischemagate`.
- No form on aischemagate.com itself in v1; the existing signup flow is
  the lead capture.
- RFC-084 (Slack lead bot) picks up the signup event through its existing
  hook, no change needed, as long as the `signup_source` column makes
  it into the Slack message. See Q4 for the migration ownership question.

### 8. Monitoring

- Vercel analytics per domain for traffic.
- RFC-016 metrics (Prometheus) stay on the gateway and already see
  CORS-prefllight success / failure counts. No new metric is required for
  v1; a dashboard panel titled "aischemagate.com origin failures" is a
  follow-up if traffic volumes make it worth the panel real estate.

### 9. The "no shared code" rule

`aischemagate-www` does not import from `dashboard/` or from this repo at
build time. All sharing is:

- Visual: Tailwind config + a stripped `components/` subset copied once at
  bootstrap.
- Content: `docs/llm-output-validation.md` synced from a contractgate tag
  (§Design 2).
- Marketing copy: a committed `MARKETING_COPY.md` mirroring RFC-095
  Appendix A. Updates come via a PR that cites the Appendix A diff.

This keeps CI independent and prevents accidental coupling (an AISchemaGate
redesign cannot break a dashboard build).

### 10. Deployment sequencing

1. RFC-094 ships `aischemagate` alias packages (RFC-094 §Rollout).
2. RFC-095 docs land on this repo and tag a release.
3. `aischemagate-www` repo is created, cloned-and-stripped from
   `dashboard/`, built, and deployed to a Vercel preview.
4. Marketing copy applied from RFC-095 Appendix A.
5. Docs sync script runs; `content/llm-output-validation.md` is committed.
6. `DASHBOARD_ORIGIN` updated in staging; smoke test passes.
7. DNS flipped for `aischemagate.com` and `aischemagate.dev`.
8. `DASHBOARD_ORIGIN` updated in production.
9. Launch announcement.

Each step is reversible: a bad deploy of the frontend does not touch the
dashboard or the gateway; a bad `DASHBOARD_ORIGIN` can be rolled back by
a single env change.

## Implementation checklist

1. Create `nightmoose/aischemagate-www` sibling repo. Confirm GitHub org
   access + naming.
2. Clone `dashboard/` layout and components into the new repo at a known
   commit; strip the authenticated / tenant-aware code paths.
3. Marketing page: hero + feature grid from RFC-095 Appendix A.
4. Docs portal: Next.js MDX render of
   `content/llm-output-validation.md`. The `<CodeTabs>` component is
   lifted from the dashboard.
5. Docs sync script (`scripts/sync-docs.ts`) + weekly cron workflow.
6. Lead-capture CTA linking to `app.datacontractgate.com/signup?from=aischemagate`.
7. CORS: extend `DASHBOARD_ORIGIN` in staging + prod. No gateway code
   change.
8. Smoke test: Vercel preview deploy + CORS success against staging.
9. DNS: purchase (if needed) and configure both domains.
10. Launch: production `DASHBOARD_ORIGIN` flip + DNS cutover.
11. STATUS.md and MAINTENANCE_LOG updates.
12. Optional follow-up (not in v1): `signup_source` column + Slack bot
    tag (RFC-084). Owned by the dashboard-schema team; flag in Q4.

## Test plan

1. CORS preflight from an aischemagate.com preview to staging gateway
   returns 204 + the expected `Access-Control-Allow-Origin` header.
2. End-to-end `curl -H "Origin: https://aischemagate.com" -H
   "x-api-key: cg_live_..." https://app.datacontractgate.com/egress/<cid>`
   returns 200.
3. Docs sync script run end-to-end: tag a dummy change in this repo,
   trigger the sync workflow, verify the frontend repo gets the PR.
4. Marketing-copy audit: diff `aischemagate.com` live copy against
   RFC-095 Appendix A; zero drift.
5. Links audit: every link on aischemagate.com and aischemagate.dev
   resolves; "Sign in" lands on `app.datacontractgate.com/signin`.
6. Lighthouse / accessibility baseline on both domains before launch.
7. Rollback drill: remove `aischemagate.com` from `DASHBOARD_ORIGIN` in
   staging; confirm CORS correctly denies the preflight.

## Rollout

See §Design 10. The three RFCs ship as a set, this one last.

## Decisions (signed off 2026-10-11)

- **Q1 → Domains already registered.** `aischemagate.com` and
  `aischemagate.dev` are owned. No purchase lead time.
- **Q2 → Vercel.** Matches dashboard. One Vercel project per domain so
  logs / analytics are separable; shared org for billing.
- **Q3 → No lead-capture form on aischemagate.com.** CTA only, deep-links
  to `app.datacontractgate.com/signup?from=aischemagate`.
- **Q4 → `signup_source` column deferred.** Not in v1 launch scope.
  Marketing attribution via Vercel analytics + the `from=aischemagate`
  query param is sufficient for v1. Dashboard-schema team can pick it up
  as a follow-up.
- **Q5 → AISchemaGate-branded dashboard view deferred.** Shared dashboard
  at `app.datacontractgate.com` in v1.
- **Q6 → Sibling repo `nightmoose/aischemagate-www`.** Confirmed.
- **Q7 → Clone-and-strip the dashboard components once at bootstrap.**
  No shared private npm package.

Original open-question text retained below as a record.

## Open questions (resolved — see Decisions above)

- **Q1 — Domain ownership**: are `aischemagate.com` and `aischemagate.dev`
  already registered? If not, that is a human-blocking step; flag the
  purchase lead time.
- **Q2 — Edge platform**: Vercel (draft default) or something else?
  Dashboard choice should drive this.
- **Q3 — Lead-capture form on aischemagate.com**: draft says no form, CTA
  only. Confirm, or specify what fields a form would collect.
- **Q4 — `signup_source` column + Slack bot tag**: owner? Dashboard team
  or this RFC? Draft punts to dashboard team as a follow-up.
- **Q5 — AISchemaGate-branded dashboard view**: deferred. Confirm defer.
- **Q6 — Repo location**: `nightmoose/aischemagate-www` is the draft
  default. Confirm.
- **Q7 — Dashboard component sharing**: clone-and-strip once (draft) vs.
  publish shared components to a private npm package? Draft prefers
  clone-and-strip to keep CI independent.

Close Q1–Q7 before implementation. Q1 is the hard blocker — nothing
deploys until the two domains exist.
