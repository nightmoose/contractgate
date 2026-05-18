-- Migration 026: Rename 'pro' plan value to 'growth' in public.orgs
--
-- The plan column already exists (migration 007) as:
--   plan text not null default 'free'
--   constraint orgs_plan_valid check (plan in ('free', 'pro', 'enterprise'))
--
-- This migration does three things:
--   1. Drops the old check constraint (which uses 'pro').
--   2. Renames any existing 'pro' rows to 'growth' (safe: no prod data yet,
--      but written defensively for future migrations).
--   3. Adds the new check constraint with 'growth' replacing 'pro'.
--
-- No column or type is added — plan is and remains a plain text column.

alter table public.orgs
    drop constraint if exists orgs_plan_valid;

update public.orgs
    set plan = 'growth'
    where plan = 'pro';

alter table public.orgs
    add constraint orgs_plan_valid
        check (plan in ('free', 'growth', 'enterprise'));

comment on column public.orgs.plan is
    'Billing plan tier: free | growth | enterprise. '
    'Controls which dashboard features the org can access (RFC-045). '
    'Set by admins; self-serve upgrade is a future feature.';
