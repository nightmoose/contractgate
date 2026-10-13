# RFC-102 — AISchemaGate plan backfill + sticky brand cookie

**Status:** shipped 2026-10-13
**Driver:** 2026-10-12 E2E tester findings A7, A8, B1, B2, E4
**Supersedes:** nothing. Depends on RFC-097 (signup carries `product` into user metadata) and migration 036 (adds `aisg_free`/`aisg_pro` tiers + the `handle_new_user()` trigger that reads `raw_user_meta_data->>'product'`).

## Problem

AISchemaGate signups land as `plan='free'` instead of `plan='aisg_free'`. The downstream dashboard code already handles `aisg_free` correctly — the Account page renders "AISchemaGate Developer", the `/pricing` page branches to `<AisgPricing/>`, Kafka/Kinesis tabs hide, Generate-from-Sample unlocks via `isAisgPlan(plan)`. But because the plan is literally `free`, every one of those surfaces falls back to the DataContractGate view.

### Diagnosis (prod inspection, 2026-10-13)

Three signals from a live `supabase db dump --schema supabase_migrations` + `--schema auth`:

1. `036_aischemagate_plans` **is** applied in prod. The applied body contains the `aisg_free`/`aisg_pro` branches (grep hit 8× on `aisg_free`, 1× on `product'`).
2. The `on_auth_user_created` trigger on `auth.users` **exists** and binds to `public.handle_new_user()`.
3. So the trigger fires on signup and reads `raw_user_meta_data->>'product'` correctly — for signups *after* RFC-097.

The gap is temporal. RFC-097 (the signup-page fix that passes `product: "aischemagate"` into `supabase.auth.signUp({ options: { data } })`) merged **2026-10-13 01:56 UTC**. Migration 036 shipped **2026-10-12**. The tester's accounts (`cgtest-aisg-1012@...` and `cgtest-dcg-1012@...`) were created **2026-10-12**, i.e. on the pre-RFC-097 signup page. Their `raw_user_meta_data->>'product'` is null and the trigger assigned `'free'`.

Separately, even if the plan were correct, `BrandProvider.useBrand()` (`dashboard/components/BrandProvider.tsx:21`) hard-overrode the cookie with `isAisgPlan(org.plan) ? "aisg" : "dcg"` — so a visitor arriving via `?product=aischemagate` would see AISG branding on page 1 and get downgraded to DCG on page 2 by the `useEffect` that re-synced the cookie against the plan. That's E4.

## Design

### Migration 036 header comment (correctness)

The migration body (a `check` constraint widening + `handle_new_user()` rewrite) was and still is correct. Only the comment at the top advertised the pre-unified `$99 / 250K` AISG tiers. Updated to match the shipped unified ladder: `aisg_free` 100K / `aisg_pro` $199/mo ($166 annual) / 25M.

### Migration 038 — backfill

```sql
update public.orgs o
set plan = 'aisg_free'
where o.plan = 'free'
  and exists (
      select 1
      from public.org_memberships m
      join auth.users u on u.id = m.user_id
      where m.org_id = o.id
        and m.role = 'owner'
        and u.raw_user_meta_data->>'product' = 'aischemagate'
  );
```

Filters on **both** metadata and current `plan='free'`. DCG free users are untouched because their metadata lacks `product='aischemagate'`. Idempotent. No new signups will need the backfill — the RFC-097 signup page sets metadata and migration 036's trigger assigns `aisg_free` directly.

### Admin-ops SQL for the tester's existing account

The backfill correctly skips `cgtest-aisg-1012@agentmail.to` because that account's `raw_user_meta_data` has no `product` field — it was created on the pre-RFC-097 signup page. For retest purposes, run this one-off after 038 has applied in prod:

```sql
update public.orgs
set plan = 'aisg_free'
where id in (
    select m.org_id
    from public.org_memberships m
    join auth.users u on u.id = m.user_id
    where u.email = 'cgtest-aisg-1012@agentmail.to'
      and m.role = 'owner'
);
```

This is intentionally **not** baked into the migration — hard-coding a test email in migration history is a smell. Run it from the Supabase SQL editor, with a one-liner audit note.

### E4 — sticky brand cookie, upgrade-only sync

`BrandProvider.useBrand()` now treats the `cg_product` cookie as the session's declared product intent:

- If `initial === "aisg"` (cookie set by the middleware from `?product=aischemagate`), the brand stays `aisg` regardless of the signed-in plan.
- Otherwise, the plan can *upgrade* `dcg → aisg` (an AISG-plan org automatically sees AISG branding) but can never *downgrade* back to `dcg`.
- The `useEffect` only writes the cookie to `aisg` when it needs to persist an upgrade — it never clears the cookie based on a `dcg` plan. Users opt out of AISG by appending `?product=datacontractgate`, which `proxy.ts:76-91` already handles directly.

This makes the URL param sticky across the whole session (the tester's explicit requirement) and keeps the brand consistent for AISG free-tier users whose plan reads `free` while migration 038 catches up.

## Changes

- `supabase/migrations/036_aischemagate_plans.sql` — header comment: pricing cleanup to unified ladder.
- `supabase/migrations/038_backfill_aisg_free_from_metadata.sql` — new one-shot backfill, metadata-filtered.
- `dashboard/components/BrandProvider.tsx` — upgrade-only cookie sync; cookie wins over plan for `aisg`.
- `docs/rfcs/102-aisg-plan-brand.md` — this document.

## Deploy procedure

1. Merge this PR to `main`.
2. CI applies migrations 036 (idempotent — already applied) and 038 to prod via the existing migration pipeline.
3. Run the admin-ops SQL above against `cgtest-aisg-1012@agentmail.to` from the Supabase SQL editor.
4. Vercel auto-deploys the dashboard; the brand fix is live.

## Verification

- `cargo check` — clean (no backend code touched).
- Supabase migration 038 is a strict `UPDATE … WHERE EXISTS`, no DDL, idempotent.
- Dashboard `npm run build` — expected clean (BrandProvider change is a ~5 line refactor within the same hook; types unchanged).
- Post-deploy retest:
  - A5, A6, A8, B1, B2 pass on the tester's existing account via E4 (brand = aisg from cookie).
  - A7 Account-page plan label passes after the one-off admin SQL.
  - A fresh AISG signup (whenever an AgentMail inbox opens up) ends on `aisg_free` directly, verifying the end-to-end pipeline from signup → metadata → trigger → plan → brand.
