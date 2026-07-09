-- Migration 029: Persist Stripe webhook failures for visibility/alerting.
--
-- Context: the webhook returns 200 to Stripe even when a handler throws or a DB
-- upgrade fails (so Stripe doesn't retry-storm). Previously those failures lived
-- only in console logs, so a paying customer could be silently stuck on free
-- (this is exactly how the 2026-06-05 plan-constraint bug hid). This table makes
-- every unresolved/failed event durable and queryable so it can be alerted on.
--
-- Written via the service-role key by /api/stripe/webhooks.

create table if not exists public.stripe_failed_events (
  event_id    text primary key,
  type        text,
  reason      text not null,           -- 'unresolved_org' | 'db_write_failed' | 'handler_error' | 'unexpected_price'
  detail      text,                    -- error message / context
  org_id      uuid,                    -- best-effort, when known
  resolved    boolean not null default false,  -- flip true once reconciled
  first_seen  timestamptz not null default now(),
  last_seen   timestamptz not null default now()
);

comment on table public.stripe_failed_events is
  'Durable log of Stripe webhook events that failed to do useful work (unresolved org, DB write failure, handler error). The webhook still 200s to Stripe; this row is the alerting/reconciliation signal. resolved=false means a paying customer may be stuck.';

-- Alerting query: unresolved failures in the last 24h.
create index if not exists stripe_failed_events_unresolved_idx
  on public.stripe_failed_events (resolved, last_seen);
