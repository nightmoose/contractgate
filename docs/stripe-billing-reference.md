# Stripe Billing Reference

Self-serve **Growth** plan billing for the ContractGate dashboard. The Stripe
webhook is the source of truth for plan changes; the dashboard never sets a
paid `plan` directly.

## Plans

| Plan        | Source                                      | `plan` value |
|-------------|---------------------------------------------|--------------|
| Free        | Default for every org                       | `free`       |
| Growth      | Self-serve via in-app Stripe Checkout       | `growth`     |
| Enterprise  | Sales-managed, set manually                 | `enterprise` |

Enterprise is never auto-downgraded by webhooks.

**Checkout is in-app-only (Option A).** Growth is purchased only by a logged-in
user from the pricing page, so org resolution is always deterministic via
`metadata.orgId`. There is **no public marketing Payment Link** — the old
email-match fallback was removed because it silently no-ops whenever the Stripe
email differs from the signup email (an invisible "paid but still free" failure).
If a public Payment Link funnel is ever added, it must stamp a deterministic
org/user identifier into the link's metadata before re-enabling any fallback.

## API routes

All routes live under `dashboard/app/api/stripe/`.

### `POST /api/stripe/create-checkout-session`
Auth: Supabase session (cookie). Starts a subscription Checkout for the caller's
primary org.

Body (optional):

```json
{ "annual": false, "priceId": "price_..." }
```

- `annual` selects `STRIPE_PRICE_GROWTH_ANNUAL` vs `STRIPE_PRICE_GROWTH_MONTHLY`.
- `priceId` overrides the env price (rarely needed).
- Rejects with 400 if the org is already on `growth`/`enterprise`.
- Sets a 14-day trial and stamps `metadata.orgId` on both session and subscription.

Returns `{ "url": "<checkout-url>" }`.

### `POST /api/stripe/portal`
Auth: Supabase session. Opens the Stripe Billing Portal for the org's
`stripe_customer_id`. Returns `{ "url": "<portal-url>" }`. 400 if no customer on file.

### `POST /api/stripe/webhooks`
Auth: Stripe signature (`stripe-signature` header) verified against
`STRIPE_WEBHOOK_SECRET`. Not for direct calls.

Idempotent: each `event.id` is recorded in `stripe_processed_events` **only after
its handler does real work**; redelivered events are skipped. An event that
no-ops (unresolved org, unmatched subscription, or an event type we don't act on)
is left unrecorded so a later redelivery can succeed once the data exists.

Handled events:

| Event                           | Effect |
|---------------------------------|--------|
| `checkout.session.completed`    | Validates price, resolves org from `metadata.orgId` (deterministic; no email fallback), sets `plan=growth`, `plan_status` from the live subscription, stores customer + subscription ids. |
| `customer.subscription.updated` | Maps Stripe status → `plan` (`growth` for active/trialing/past_due, `free` for canceled/unpaid). Enterprise untouched. Mirrors status to `plan_status`. |
| `customer.subscription.deleted` | Downgrades org to `free`, clears `stripe_subscription_id`. |
| `invoice.payment_failed`        | Sets `plan_status=past_due`. |

Org resolution for checkout is `session.metadata.orgId` only (stamped by
`create-checkout-session` on both the session and the subscription). No email
fallback — see the Option A note above.

### Failure visibility (no more silent 200s)

The webhook still returns **200** to Stripe on handler errors (so Stripe doesn't
retry-storm), but failures are now **persisted** to `stripe_failed_events` instead
of living only in `console.error`. A row is written when:

- `unresolved_org` — a `checkout.session.completed` arrived with no `metadata.orgId`.
- `unexpected_price` — checkout used a price that isn't the configured Growth price.
- `db_write_failed` — the Supabase `UPDATE orgs …` failed (e.g. schema drift like
  the 2026-06-05 `growth` check-constraint bug).
- `handler_error` — any handler threw.

`resolved=false` rows are the alerting signal that a paying customer may be stuck
on free. Suggested alert (unresolved failures in the last 24h):

```sql
select * from public.stripe_failed_events
where resolved = false and last_seen > now() - interval '24 hours'
order by last_seen desc;
```

## Environment variables

Set in the dashboard deployment (see `dashboard/.env.example`). Use the
**ContractGate** Stripe account.

| Var                            | Purpose |
|--------------------------------|---------|
| `STRIPE_SECRET_KEY`            | Server secret key (`sk_live_…` / `sk_test_…`). |
| `STRIPE_WEBHOOK_SECRET`        | Signing secret (`whsec_…`) for the webhook endpoint. |
| `STRIPE_PRICE_GROWTH_MONTHLY`  | Price id (`price_…`) for monthly Growth. |
| `STRIPE_PRICE_GROWTH_ANNUAL`   | Price id (`price_…`) for annual Growth. |
| `NEXT_PUBLIC_APP_URL`          | Base URL for success/cancel/portal redirects (no trailing slash). |
| `NEXT_PUBLIC_SUPABASE_URL`     | Used by the webhook's service-role client. |
| `SUPABASE_SERVICE_ROLE_KEY`    | Service-role key; webhook writes bypass RLS. |

Stripe API version is pinned in code to `2026-05-27.dahlia` (matches `stripe@22`).

## Database (migration 028)

Columns added to `public.orgs`:

- `stripe_customer_id text` — `cus_…`
- `stripe_subscription_id text` — `sub_…`
- `plan_status text` — mirrors Stripe subscription status; `null` for free orgs

Indexes on `stripe_customer_id` and `stripe_subscription_id` for webhook lookups.

Table `public.stripe_processed_events` — webhook idempotency log
(`event_id` PK, `type`, `processed_at`).

### Migration 029

Table `public.stripe_failed_events` — durable log of webhook events that failed
to do useful work (`event_id` PK, `type`, `reason`, `detail`, `org_id`,
`resolved`, `first_seen`, `last_seen`). Indexed on `(resolved, last_seen)` for the
alerting query above. Written via the service-role key by the webhook.

## Flows

**In-app upgrade (the only purchase flow):** pricing page →
`create-checkout-session` → Stripe Checkout → `billing/success` (cosmetic) →
webhook sets the plan. The success page does not verify the session; the webhook
is authoritative. Org resolution is deterministic via `metadata.orgId`.

## Known limitations

- `billing/success` is cosmetic — no immediate session verification endpoint yet.
- `plan_status` value constraint is left commented in migration 028 until there
  is data to validate against.
- `stripe_failed_events` is written but not yet wired to an external alerter
  (Slack/email/PagerDuty). Until then, monitor it with the SQL query above.
