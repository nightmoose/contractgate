-- 028_stripe_billing.sql
--
-- RETROACTIVE FILE (2026-07-09 drift reconciliation). These objects were
-- created directly in prod (untracked) during the Stripe launch. DDL below is
-- reconstructed from the live prod schema. Idempotent — a no-op on prod.
--
-- 1. Stripe billing identifiers on orgs.
-- 2. Webhook dedup log (at-most-once handling on Stripe retries).

ALTER TABLE public.orgs
    ADD COLUMN IF NOT EXISTS stripe_customer_id     TEXT,
    ADD COLUMN IF NOT EXISTS stripe_subscription_id TEXT;

COMMENT ON COLUMN public.orgs.stripe_customer_id IS
    'Stripe Customer ID (cus_...) for this org''s paid plan.';
COMMENT ON COLUMN public.orgs.stripe_subscription_id IS
    'Stripe Subscription ID (sub_...) for the current Growth/Enterprise plan.';

CREATE TABLE IF NOT EXISTS public.stripe_processed_events (
    event_id      TEXT PRIMARY KEY,
    type          TEXT,
    processed_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.stripe_processed_events IS
    'Dedup log of Stripe webhook event ids already processed by '
    '/api/stripe/webhooks. Ensures at-most-once handling on Stripe retries.';

-- Service-role access only; no policies by design.
ALTER TABLE public.stripe_processed_events ENABLE ROW LEVEL SECURITY;
