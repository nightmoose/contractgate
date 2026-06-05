# Stripe Billing Reference

Self-serve **Growth** plan billing for the ContractGate dashboard. The Stripe
webhook is the source of truth for plan changes; the dashboard never sets a
paid `plan` directly.

## Plans

| Plan        | Source                                      | `plan` value |
|-------------|---------------------------------------------|--------------|
| Free        | Default for every org                       | `free`       |
| Growth      | Self-serve via Stripe Checkout / Payment Link | `growth`   |
| Enterprise  | Sales-managed, set manually                 | `enterprise` |

Enterprise is never auto-downgraded by webhooks.

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

Idempotent: each `event.id` is recorded in `stripe_processed_events`; redelivered
events are skipped. Handler errors return 200 (logged) so Stripe does not retry
endlessly.

Handled events:

| Event                           | Effect |
|---------------------------------|--------|
| `checkout.session.completed`    | Validates price, resolves org (metadata → email fallback), sets `plan=growth`, `plan_status` from the live subscription, stores customer + subscription ids. |
| `customer.subscription.updated` | Maps Stripe status → `plan` (`growth` for active/trialing/past_due, `free` for canceled/unpaid). Enterprise untouched. Mirrors status to `plan_status`. |
| `customer.subscription.deleted` | Downgrades org to `free`, clears `stripe_subscription_id`. |
| `invoice.payment_failed`        | Sets `plan_status=past_due`. |

Org resolution order for checkout: `session.metadata.orgId` → look up Supabase
user by `customer_email` (via paginated `listUsers`, capped at 1000 users) →
that user's earliest org membership.

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

## Flows

**In-app upgrade:** pricing page → `create-checkout-session` → Stripe Checkout →
`billing/success` (cosmetic) → webhook sets the plan. The success page does not
verify the session; the webhook is authoritative.

**Marketing-site Payment Link:** buyer pays on a Payment Link (no `orgId`
metadata) → webhook resolves the org by email → upgrades. Requires the buyer's
Stripe email to match their Supabase account email.

## Known limitations

- Email-fallback resolution scans up to 1000 users; very large user bases would
  need an indexed profiles lookup instead.
- `billing/success` is cosmetic — no immediate session verification endpoint yet.
- `plan_status` value constraint is left commented in migration 028 until there
  is data to validate against.
