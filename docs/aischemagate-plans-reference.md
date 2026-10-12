# AISchemaGate Plans — Reference

**Added:** migration 036, `nightly-maintenance-2026-10-12-aischemagate-plans`

AISchemaGate shares the DataContractGate tenant, API keys, dashboard and
billing, but sells its own tiers. They are separate plan values on the same
`orgs.plan` column, so DataContractGate's Free / Growth / Enterprise are
unchanged.

| Plan value | Name | Price | Validations / month | Paid features |
|---|---|---|---|---|
| `aisg_free` | Developer | $0 | 10,000 | no |
| `aisg_pro` | Pro | $99 / month | 250,000 (hard cap) | yes (payload storage, replay) |
| `enterprise` | Enterprise | custom (from $499 / month) | unlimited | yes |

One validation is one event: an LLM output checked through `/egress` or
`/v1/ingest` counts exactly like any other event. Limits are enforced by the
gateway (`src/plan.rs`, `monthly_event_limit`); at the cap, ingest returns
the same quota error as DataContractGate plans. There is no overage billing
in v1.

## How an account gets an AISchemaGate plan

- **Sign-up.** `app.datacontractgate.com/auth/signup?product=aischemagate`
  (the link on aischemagate.com) stores `product: "aischemagate"` in the
  user's metadata; the `handle_new_user` trigger starts that org on
  `aisg_free`. GitHub sign-ups cannot carry metadata and start on
  DataContractGate `free` (1M / month); they can still buy Pro.
- **Upgrade.** `/pricing` shows the AISchemaGate tiers to `aisg_*` orgs and to
  any visitor with `?product=aischemagate`. Pro checkout posts
  `{ plan: "aisg_pro" }` to `/api/stripe/create-checkout-session`, which uses
  `STRIPE_PRICE_AISG_PRO_MONTHLY` (30-day trial, same as Growth).
- **Webhook.** A completed checkout on the Pro price sets `aisg_pro`.
  Subscription updates keep the org on its product line: active / trialing /
  past_due → `aisg_pro`, canceled / unpaid / deleted → `aisg_free`.

## Feature gating

For dashboard feature gates (`planAtLeast`), `aisg_free` ranks with `free`
and `aisg_pro` with `growth`. Upsell cards tell AISchemaGate accounts to
upgrade to Pro, not Growth.

## Configuration

| Where | Name | Value |
|---|---|---|
| Stripe | Product "AISchemaGate Pro", recurring price | $99 / month |
| Vercel (dashboard) | `STRIPE_PRICE_AISG_PRO_MONTHLY` | that price's id (`price_...`) |
| Supabase | migration `036_aischemagate_plans.sql` | apply before deploying the dashboard |
