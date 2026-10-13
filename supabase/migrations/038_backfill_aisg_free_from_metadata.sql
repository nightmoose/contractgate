-- Migration 038: backfill aisg_free for orgs whose owner signed up via the
-- AISchemaGate funnel but landed on 'free' because `product` metadata hadn't
-- yet been plumbed through signup.
--
-- Timeline that produced the gap:
--   2026-10-12  Migration 036 shipped the `aisg_free`/`aisg_pro` tiers +
--               `handle_new_user()` trigger that reads
--               `raw_user_meta_data->>'product'`.
--   2026-10-13 01:56 UTC  RFC-097 shipped the signup-page change that actually
--               passes `product: "aischemagate"` into Supabase user metadata
--               when the URL is `?product=aischemagate`.
--
-- Any AISG signup between those two timestamps landed as `plan='free'` because
-- the trigger ran against empty metadata. This one-shot UPDATE heals them.
--
-- Safety:
--   * Filters on BOTH the owner's `raw_user_meta_data->>'product'='aischemagate'`
--     AND current `plan='free'` — DataContractGate free users are untouched.
--   * Idempotent: re-running affects zero rows after the first apply.
--   * No new signups will need this migration — RFC-097's signup page sets the
--     metadata, and migration 036's trigger assigns `aisg_free` directly.
--
-- Not healed by this migration:
--   Owners who signed up on the pre-RFC-097 page (metadata empty) do NOT match
--   the filter — correctly, because we can't distinguish them from
--   DataContractGate free users via metadata alone. For known test accounts in
--   that state (e.g. `cgtest-aisg-1012@agentmail.to`), run the one-off UPDATE
--   documented in docs/rfcs/102-aisg-plan-brand.md after this migration is
--   applied.

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
