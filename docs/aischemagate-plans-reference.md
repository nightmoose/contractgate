# Plans — Reference (DataContractGate and AISchemaGate)

**Added:** migration 036 (AISchemaGate plan values), unified ladder and
retention in migration 037, both 2026-10-12.

Both products sell **one plan ladder at the same prices**. AISchemaGate
accounts carry their own plan values (`aisg_*`) only so the app can brand and
trim its navigation for them; limits, prices and billing are identical.

| | Free | Pro | Enterprise |
|---|---|---|---|
| Plan values | `free`, `aisg_free` | `growth`, `aisg_pro` | `enterprise` |
| Price | $0 | $199 / month, or $166 / month billed annually ($1,992 / year) | custom |
| Validations (events) / month | 100,000 | 25,000,000 (hard cap) | unlimited |
| Rate limit per API key | 10 / sec (burst 100) | 100 / sec (burst 1,000) | 100 / sec, raised per key on request |
| Audit / quarantine log retention | 7 days | 90 days | 365 days (adjustable) |
| Payload storage, replay | no | yes | yes |

One validation is one event: an LLM output checked through `/egress` or
`/v1/ingest` counts like any other event, whatever its size (requests are
capped at 1 MB). Limits live in `src/plan.rs` (`monthly_event_limit`,
`default_rate_limit`); retention in migration 037 (`log_retention_days`,
`purge_expired_logs`, hourly via pg_cron). There is no overage billing.

## How an account lands on a plan

- **Sign-up.** Links from aischemagate.com carry `?product=aischemagate`.
  The dashboard stores `product: "aischemagate"` in the user's metadata and
  the `handle_new_user` trigger starts the org on `aisg_free`. GitHub
  sign-ups cannot carry metadata and start on `free` (same limits).
- **Upgrade.** One set of Stripe prices (`STRIPE_PRICE_GROWTH_MONTHLY`,
  `STRIPE_PRICE_GROWTH_ANNUAL`) serves both products. Checkout stamps
  `metadata.product`; the webhook sets `aisg_pro` for AISchemaGate accounts
  and `growth` otherwise. Cancellation returns each to its own free value.
- **Branding.** `?product=aischemagate` also sets the `cg_product` cookie;
  signed in, the org's plan decides (see `dashboard/lib/brand.ts`).

## Feature gating

For dashboard gates (`planAtLeast`), `aisg_free` ranks with `free` and
`aisg_pro` with `growth`. AISchemaGate accounts additionally get Generate
from Sample on the free plan, and don't see the data-pipeline pages.
