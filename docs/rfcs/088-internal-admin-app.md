# RFC-088 — Standalone internal admin app ("god-mode" ops console)

**Status:** Draft + v1 scaffold on branch
**Date:** 2026-07-21
**Depends on:** Supabase (orgs/org_memberships/auth), Stripe (customers/subscriptions), migration 028 (billing columns)
**Origin:** user request — "I need one place to see users + subscription status and jump to Stripe/Supabase to act."

---

## Problem

Operating the business currently means bouncing between the Supabase dashboard
(to see what accounts exist) and the Stripe dashboard (to deal with a
subscription), with no single view that ties a user/org to its plan, its Stripe
subscription status, and the place to act on each. There's no internal console.

This will also be the **first tenant of a broader internal-tools app** — other
tools (e.g. "Mercenary") will live alongside it — so it should be a standalone
app, not a page bolted onto the customer dashboard.

## Decision

A **separate Next.js app** (`internal-admin/`), deployed on its own (Vercel),
never linked from or sharing a domain with the customer dashboard. It has
**god-mode read access** over the app's data:

- **Supabase service-role key** (bypasses RLS) — server-side only.
- **Stripe secret key** — server-side only.

It is **read + navigate**, not act-in-place: it surfaces state and **deep-links**
to the Stripe customer and the Supabase row where you make the real change. This
keeps the blast radius small (no destructive writes from a god-mode console in
v1) while solving the actual pain (one view, direct links).

### Why separate app, not a `/admin` route in the dashboard

- The customer dashboard ships the anon key to the browser and is RLS-scoped by
  design. An admin surface needs the service-role key and cross-org reads —
  fundamentally different trust boundary. Keeping it in a separate app/deploy
  means the service-role key never lives in the customer app's process or env.
- It's the first of several internal tools; a standalone app gives them a home.

## Access control (hard wall)

God-mode data access demands a strong gate. Layered:

1. **Platform:** deploy behind Vercel Access / SSO (or equivalent) — the app is
   never publicly reachable without an org SSO login. (Ops step, documented.)
2. **App auth:** Supabase email/password login (reuses the same Supabase
   project), then a **superadmin allowlist**: the logged-in user's email must be
   in `ADMIN_EMAILS` (comma-separated env). Non-allowlisted sessions get 403.
3. **Server-only secrets:** service-role + Stripe secret keys are only read in
   server code (route handlers / server components). `middleware.ts` enforces
   the allowlist on every route; each server data call re-checks (defense in
   depth). Nothing privileged is ever sent to the browser.

The allowlist is env-based (no schema change, easy to rotate). A DB
`is_superadmin` flag is a later option if the admin roster grows.

## v1 scope — Users & Subscriptions

One view: every org with the operator's-eye data, joined across Supabase + Stripe.

Per org row:

- org name / slug / id, `plan`, `plan_status` (Stripe: trialing/active/past_due/…)
- member count + member emails (via the Supabase **auth admin API**, since
  `auth.users.email` isn't in the public schema)
- created date, current-month usage (optional, from `org_monthly_usage`)
- **Deep links:** Stripe customer (`dashboard.stripe.com/customers/{cus_id}`),
  Stripe subscription, and the Supabase `orgs` row (table editor URL for the
  project ref)
- Live Stripe subscription status fetched from the Stripe API for the org's
  `stripe_subscription_id` (source of truth, in case the webhook mirror drifted)

Search/filter by email, org name, or plan. Read-only.

## Architecture

```
internal-admin/                 # standalone Next.js (App Router)
  middleware.ts                 # allowlist gate on every route
  lib/supabaseAdmin.ts          # service-role client (server only)
  lib/stripe.ts                 # Stripe client (server only)
  lib/auth.ts                   # session + ADMIN_EMAILS check
  lib/data.ts                   # aggregation: orgs ⋈ members ⋈ Stripe
  app/login/page.tsx            # Supabase password login
  app/users/page.tsx            # the v1 console (server component)
  .env.example, README.md
```

Data flow: `app/users/page.tsx` (server) → `lib/auth` gate → `lib/data`
aggregates `orgs` (service role) + member emails (auth admin) + Stripe
subscription lookups → renders a table with deep links. No browser-side
privileged calls; no customer data leaves the server except the operator view.

## Deploy / secrets

Separate Vercel project. Env:

| Var | Purpose |
|---|---|
| `SUPABASE_URL` | project URL (also yields the ref for table-editor links) |
| `SUPABASE_SERVICE_ROLE_KEY` | god-mode DB reads (server only) |
| `STRIPE_SECRET_KEY` | live subscription/customer reads (server only) |
| `ADMIN_EMAILS` | comma-separated superadmin allowlist |
| `SUPABASE_PROJECT_REF` | for building Supabase dashboard deep links |

Never committed; set in Vercel. Put the deployment behind Vercel Access.

## Explicitly out of scope (v1)

- Any write/mutation from the console (cancel a sub, change a plan, edit a row) —
  you act in Stripe/Supabase via the deep links. A future RFC can add guarded,
  audited write actions.
- The broader internal-tools shell / other tenants ("Mercenary") — this app is
  structured so they can be added as routes, but they are separate work.

## Security notes

- Service-role + Stripe secret are the crown jewels; they exist only in this
  app's server env, never in the customer dashboard, never shipped to a browser.
- The console exposes all users' billing/account status — hence the layered gate
  (platform SSO + email allowlist + per-request recheck).
- Read-only v1 means a compromised session can *view* but not *change* customer
  billing or data — the destructive actions still require a real Stripe/Supabase
  login.

## Status

- v1 scaffold committed under `internal-admin/` (data layer, auth gate, users
  view, config). Needs `npm install`, env wiring, and a protected Vercel deploy;
  it cannot be built or run in the dev sandbox. Treat as review-ready scaffold,
  not yet deployed.
