-- Migration 028: Stripe billing fields on orgs (self-serve Growth via Payment Links + Checkout)
--
-- Adds columns to track the Stripe customer + subscription for an org.
-- plan_status mirrors Stripe Subscription.status for dunning / trial UX.
-- These are written by the dashboard's /api/stripe/webhooks handler (and create-checkout-session).
--
-- The webhook is the source of truth for plan changes coming from Stripe.

-- Default is null: a free org has no Stripe subscription, so it has no status.
-- plan_status is populated by the webhook once a subscription exists.
alter table public.orgs
  add column if not exists stripe_customer_id text,
  add column if not exists stripe_subscription_id text,
  add column if not exists plan_status text;

-- Helpful for lookups from webhooks
create index if not exists orgs_stripe_customer_id_idx on public.orgs (stripe_customer_id);
create index if not exists orgs_stripe_subscription_id_idx on public.orgs (stripe_subscription_id);

comment on column public.orgs.stripe_customer_id is 'Stripe Customer ID (cus_...) for this org''s paid plan.';
comment on column public.orgs.stripe_subscription_id is 'Stripe Subscription ID (sub_...) for the current Growth/Enterprise plan.';
comment on column public.orgs.plan_status is 'Stripe subscription status: trialing | active | past_due | canceled | unpaid etc.';

-- Optional: tighten plan_status values (commented so it can be added after some data exists)
-- alter table public.orgs add constraint orgs_plan_status_valid
--   check (plan_status is null or plan_status in ('trialing','active','past_due','canceled','unpaid','incomplete','incomplete_expired'));

-- Webhook idempotency: Stripe redelivers events on retry. The webhook handler
-- inserts the event id here before processing; a unique-violation means the
-- event was already handled and is skipped. Written via the service-role key.
create table if not exists public.stripe_processed_events (
  event_id text primary key,
  type text,
  processed_at timestamptz not null default now()
);

comment on table public.stripe_processed_events is
  'Dedup log of Stripe webhook event ids already processed by /api/stripe/webhooks. Ensures at-most-once handling on Stripe retries.';
