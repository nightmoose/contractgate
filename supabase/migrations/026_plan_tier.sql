-- Migration 026: Add plan tier to organizations
--
-- Adds a plan enum and column to the organizations table so the dashboard
-- can gate Growth+ and Enterprise features per org.
--
-- All existing orgs default to 'free'.  Admins promote orgs via the
-- Supabase dashboard or a future admin API.  Backend quota enforcement
-- (event limits, audit retention windows) is a separate concern and is
-- NOT in this migration.

-- Enum: ordered from least to most capable.
create type public.plan_tier as enum ('free', 'growth', 'enterprise');

-- Add column — NOT NULL with default so existing rows are covered atomically.
alter table public.organizations
  add column plan public.plan_tier not null default 'free';

-- Comment for schema browsers.
comment on column public.organizations.plan is
  'Billing plan tier: free | growth | enterprise.  '
  'Controls which dashboard features the org can access.';
