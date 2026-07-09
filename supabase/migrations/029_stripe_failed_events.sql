-- 029_stripe_failed_events.sql
--
-- RETROACTIVE FILE (2026-07-09 drift reconciliation). Applied to prod on
-- 2026-06-07 (tracked in supabase_migrations as "029_stripe_failed_events")
-- but the file was never committed. DDL below is reconstructed from the live
-- prod schema. Idempotent — a no-op on prod.
--
-- Durable log of Stripe webhook events that failed to do useful work. The
-- webhook still 200s to Stripe; this row is the alerting/reconciliation
-- signal. resolved=false means a paying customer may be stuck.

CREATE TABLE IF NOT EXISTS public.stripe_failed_events (
    event_id    TEXT PRIMARY KEY,
    type        TEXT,
    reason      TEXT NOT NULL,
    detail      TEXT,
    org_id      UUID,
    resolved    BOOLEAN NOT NULL DEFAULT false,
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.stripe_failed_events IS
    'Durable log of Stripe webhook events that failed to do useful work '
    '(unresolved org, DB write failure, handler error). The webhook still '
    '200s to Stripe; this row is the alerting/reconciliation signal. '
    'resolved=false means a paying customer may be stuck.';

CREATE INDEX IF NOT EXISTS stripe_failed_events_unresolved_idx
    ON public.stripe_failed_events (resolved, last_seen);

-- Service-role access only; no policies by design.
ALTER TABLE public.stripe_failed_events ENABLE ROW LEVEL SECURITY;
