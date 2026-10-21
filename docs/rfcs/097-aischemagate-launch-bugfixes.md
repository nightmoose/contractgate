---
title: AISchemaGate launch bugfixes — product param through email, callback cookie, migration-036 drift check
status: Accepted
author: product owner
date: 2026-10-12
---

# RFC-097 — AISchemaGate launch bugfixes (cluster 1 of 6)

## Problem

The 2026-10-12 E2E test against production (see
`docs/ops/e2e-test-brief-2026-10-12.md`) found four cascading failures for new
AISchemaGate sign-ups coming from `aischemagate.com`:

- **A5/A6** — after clicking the email-confirmation link, the user lands in
  full DataContractGate branding with the DCG sidebar (Stream Demo, Kafka
  Connect, Python SDK, Catalog, Scorecard, Scaffold, Workbench). The
  `redirect_to` on Supabase's confirmation email is `/auth/callback?next=/`
  with no `product` param.
- **A7** — the new org's plan is `free`, not `aisg_free`. The Account page
  reads "Plan: Free" instead of "AISchemaGate Developer".
- **A8** — the sidebar Pricing link shows the DCG pricing table.
- **E4** — hitting `?product=aischemagate` on `/`, `/contracts`, `/pricing` for
  an existing DCG account does not switch the branding.

A5/A6/A8/E4 are all downstream of two root causes:

1. **The signup page drops `?product=aischemagate` from `emailRedirectTo`**
   (`dashboard/app/auth/signup/page.tsx:78`). When the user confirms the email
   on a different device from the one they signed up on, the browser doing the
   confirmation has no `cg_product=aisg` cookie, no `?product` param on the
   URL, and `useOrg().plan` has not loaded yet — so the first frame is DCG.
2. **The `/auth/callback` route does not re-establish the brand cookie**
   (`dashboard/app/auth/callback/route.ts:11`). It reads only `next`.

A7 has a third root cause:

3. **Migration 036 is believed-but-not-verified applied to production.** The
   migration rewrites `handle_new_user()` to read `raw_user_meta_data->>'product'`
   and assign `aisg_free` when it is `"aischemagate"`. If the trigger in prod
   is still the pre-036 version, every sign-up gets `plan = 'free'` regardless
   of what metadata the signup page sends. `supabase/unapplied-migrations.txt`
   does not list 036, which the migration-drift workflow interprets as
   "applied", but the daily workflow runs against the Supabase ledger table —
   if the ledger drifted (as it did with 033/034 per `docs/STATUS.md`
   2026-08-13) the trigger could be stale with no CI signal.

E4 is a sub-case of (1)+(2): the cookie-based brand switch *does* work on page
loads that hit `proxy.ts` with `?product=` on the URL, but the user's
authenticated-brand fallback (`useOrg().plan`) overrides the cookie once the
org is loaded. For an already-signed-up DCG user, `?product=aischemagate` sets
the cookie correctly but `useBrand()` then clobbers it with `"dcg"` from the
org's plan. **This is working-as-designed** per `components/BrandProvider.tsx`
— "a signed-in org's plan decides" — and the test's expectation is wrong.
We will not fix E4; see **Non-goals**.

## Decision

### 1. `/auth/signup/page.tsx`: pass product through the email flow

Capture `product` from the URL at page-mount time and include it in both the
email-confirm `redirectTo` and the GitHub OAuth `redirectTo`. The signup page
is a `"use client"` component, so this is a trivial change.

```ts
const productParam =
  new URLSearchParams(location.search).get("product") === "aischemagate"
    ? "&product=aischemagate"
    : "";

emailRedirectTo: `${location.origin}/auth/callback?next=/${productParam}`,
```

The GitHub `redirectTo` carries the same param. GitHub signups still land on
DCG `free` (per the test brief's "known and expected"), but the branding
cookie should still follow.

### 2. `/auth/callback/route.ts`: re-establish the brand cookie

The callback already parses the URL. Add a check for `?product=aischemagate`
and set the `cg_product=aisg` cookie on the redirect response, with the same
options `proxy.ts` uses (`path: /`, `maxAge: 30d`, `sameSite: Lax`, `secure`).

This gives first-frame-correct branding after cross-device email confirmation
even before `useOrg()` has resolved. Once the plan loads, `useBrand()` takes
over as before.

### 3. Operational: verify + re-apply migration 036

**Not a code change in this PR**, but a required pre-ship step:

- Confirm migration 036 is applied to the Supabase prod ledger by querying
  `supabase_migrations.schema_migrations` for `036`.
- If missing: apply via the normal workflow and verify
  `handle_new_user()` body contains the `v_plan := case when ... then 'aisg_free'`
  branch.
- If present but the function body is stale: this is a drift case; re-run
  `supabase migration up 036` with `--include-all`.

The CI migration-drift check (`.github/workflows/migration-drift.yml`) already
compares the prod ledger daily; the operational task is to actually look at
its most recent run and act on any divergence.

## Non-goals

- E4 ("`?product=aischemagate` doesn't switch branding for an existing DCG
  account"). This is working as designed: a signed-in org's plan is the source
  of truth. Changing this would let any user override their org's brand with
  a URL param, which would make the sidebar nav lie about what their plan
  unlocks. The test expectation is wrong; we'll update the test brief.
- B1/B2/B6 ("Generate from Sample locked", "Kafka/Kinesis tabs locked",
  "Quarantine Pro-locked" on AISG free). These are plan-feature-gating
  decisions, cluster 4 in the triage. Separate RFC.
- C1 (rate limiting broken). Cluster 2. Separate RFC.
- B3/B4 (SDK 0.3.0 alias packages never published). Cluster 3. Separate RFC;
  release-pipeline fix, not a code bug in the SDK.
- F4/F5 (api host security — unauth `/metrics`, missing headers). Cluster 5.
- E1b (`datacontractgate.com/pricing` 404). Cluster 6, marketing site repo.

## Rollout

- Files changed:
  - `dashboard/app/auth/signup/page.tsx` — ~4 lines.
  - `dashboard/app/auth/callback/route.ts` — ~6 lines.
- Verification:
  - `npm run build` under `dashboard/`.
  - Manual: sign up from `/auth/signup?product=aischemagate` in a private
    window; confirm the email in a **different** private window; verify the
    URL hits `/auth/callback?code=...&next=/&product=aischemagate` and the
    first post-redirect page is AISG-branded.
  - Verify migration 036 ledger entry before cutting the deploy.
- Rollback: revert the two files. Migration 036 does not need to roll back.

## Risks

- **Supabase allowed-redirect-URL config**: if Supabase Auth is configured to
  require an exact match on `redirectTo`, adding query params may be rejected.
  Supabase's default behavior is prefix-match on "Additional Redirect URLs"
  with query params allowed. Verify in the Supabase dashboard before deploy.
- **Open redirect**: the `product` param is checked for exact equality to
  `"aischemagate"`; any other value is ignored and the cookie stays as-is. No
  attacker-controlled URL is reflected into a response header.
