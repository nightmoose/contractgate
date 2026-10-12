-- Migration 036: AISchemaGate plans (aisg_free, aisg_pro)
--
-- AISchemaGate shares the DataContractGate tenant, API keys and billing, but
-- sells its own tiers (decided 2026-10-12):
--   aisg_free  — $0,      10,000 validations / month
--   aisg_pro   — $99/mo,  250,000 validations / month (hard cap in v1)
-- Monthly limits live in src/plan.rs; this migration only widens the allowed
-- plan values and lets sign-up start an org on aisg_free.
--
-- Changes:
--   MOD  orgs_plan_valid      — adds 'aisg_free', 'aisg_pro'
--   MOD  handle_new_user()    — sign-ups carrying user metadata
--                               product = 'aischemagate' (set by the sign-up
--                               page from ?product=aischemagate) start on
--                               'aisg_free'; everyone else keeps the column
--                               default 'free'. Body is otherwise identical to
--                               migration 012 (verified against prod 2026-10-12).
--
-- Safe to apply before the code that uses it: nothing writes the new values
-- until the dashboard ships, and existing rows already satisfy the constraint.

alter table public.orgs drop constraint orgs_plan_valid;
alter table public.orgs
    add constraint orgs_plan_valid
    check (plan in ('free', 'growth', 'enterprise', 'aisg_free', 'aisg_pro'));

create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer set search_path = public
as $$
declare
    v_org_id uuid;
    v_slug   text;
    v_name   text;
    v_plan   text;
begin
    -- 1. Create user_profile row (unchanged from migration 006).
    insert into public.user_profiles (id, display_name)
    values (
        new.id,
        coalesce(new.raw_user_meta_data->>'display_name', split_part(new.email, '@', 1))
    )
    on conflict (id) do nothing;

    -- 2. Starting plan. Only the exact product value switches it; anything
    --    else (missing, typo, tampering) keeps the default free plan.
    v_plan := case
        when new.raw_user_meta_data->>'product' = 'aischemagate' then 'aisg_free'
        else 'free'
    end;

    -- 3. Derive slug. On the first try, use the bare slug; on uniqueness
    --    conflict, append an 8-char hex suffix from gen_random_uuid().
    v_name := coalesce(
        new.raw_user_meta_data->>'display_name',
        split_part(new.email, '@', 1)
    );
    v_slug := lower(regexp_replace(v_name, '[^a-zA-Z0-9]+', '-', 'g'));
    v_slug := trim(both '-' from v_slug);

    begin
        insert into public.orgs (name, slug, plan)
        values (v_name, v_slug, v_plan)
        returning id into v_org_id;
    exception when unique_violation then
        insert into public.orgs (name, slug, plan)
        values (
            v_name,
            v_slug || '-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 8),
            v_plan
        )
        returning id into v_org_id;
    end;

    -- 4. Make the new user the owner of their org.
    insert into public.org_memberships (org_id, user_id, role)
    values (v_org_id, new.id, 'owner');

    return new;
end;
$$;

-- Keep migration 031's lockdown: trigger-only, never callable via PostgREST.
revoke execute on function public.handle_new_user() from public, anon, authenticated;
