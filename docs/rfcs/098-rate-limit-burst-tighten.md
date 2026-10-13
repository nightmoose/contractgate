---
title: Tighten rate-limit burst to match documented per-key rps
status: Accepted
author: Alex
date: 2026-10-12
---

# RFC-098 — Tighten per-key rate-limit burst (cluster 2 of 6)

## Problem

The 2026-10-12 E2E test (C1) sent 30 parallel requests from a Free-plan API
key and received **zero HTTP 429 responses**. Response headers showed
`X-RateLimit-Limit: 10` and `X-RateLimit-Remaining: 41–59` (later `98`).

Both numbers are individually correct under the current token-bucket
implementation (`src/rate_limit.rs`):

- `limit` reports `rps` (10 r/s for Free, per `plan.rs::default_rate_limit`).
- `remaining` reports the bucket's current token count.
- The bucket's capacity (`burst`) is **100** for Free (10× rps).

A fresh Free bucket therefore allows 100 requests back-to-back before the
first denial, which:

1. Breaks the test brief's expectation ("30 in one second → some 429s").
2. Breaks the pricing-page promise ("10 req/s per key").
3. Confuses observability: `Remaining: 98` with `Limit: 10` is a header lie
   under the IETF draft semantics (remaining is "allowance within the current
   window", not "bucket tokens").

The 10× burst comes from `src/rate_limit.rs:28–29`'s historical defaults
(`DEFAULT_RATE_LIMIT_RPS: 100, DEFAULT_RATE_LIMIT_BURST: 1_000`), which the
plan ladder (RFC-083 / 2026-10-12 ea237b3) inherited verbatim as the ratio.
When defaults were "100 r/s, 1 000 burst" (the pre-plan state), the 10×
burst was high but no customer complained. Once plan-based limits dropped
Free to 10 rps, 10× burst became customer-visible gaming surface.

## Decision

### 1. Burst = 2× rps

In `src/plan.rs::default_rate_limit`:

```rust
pub fn default_rate_limit(plan: &str) -> (u32, u32) {
    if is_paid(plan) {
        (100, 200)   // was (100, 1_000)
    } else {
        (10, 20)     // was (10, 100)
    }
}
```

Rationale for 2× (not 1×, not 1.5×):

- 1× (`burst = rps`) is a hard rate limit; a legitimate 20-event bulk SDK
  flush would fragment into denials.
- 1.5× is tight enough to catch gaming but leaves little headroom for
  normal batching.
- 2× is the industry-common ratio (Stripe, Shopify, Twilio) and matches a
  common SDK batch size (10–20 events) with one burst's worth of slack.
- 30 parallel Free requests → 20 pass, 10 deny. Test brief satisfied.
- Steady 5 r/s → all pass. Test brief satisfied.
- Steady 10 r/s with the occasional 15–20-event batch → all pass. Normal
  SDK usage still clean.

The documented `rps` on the pricing page is unchanged — only the burst
shape is corrected.

### 2. `X-RateLimit-Remaining` floored at `rps`

In `src/rate_limit.rs::RateLimitState::check`:

```rust
let remaining_header = remaining.min(rps_override.unwrap_or(DEFAULT_RATE_LIMIT_RPS));
```

Report `min(bucket_tokens, rps)` as `remaining`. Internally the bucket
still tracks its real token count — only the header is clamped. This
removes the `Remaining > Limit` lie without changing the limiter's
behavior.

### 3. Also lower the hard-coded defaults

`DEFAULT_RATE_LIMIT_RPS: 100` and `DEFAULT_RATE_LIMIT_BURST: 1_000` in
`src/rate_limit.rs:28–29` are used only when a key has no override AND
no plan is resolved — a path we no longer hit in production (every row
in `api_keys` is JOIN-resolved against `orgs.plan` since RFC-083). But
they're the fallback for tests and for self-hosted builds where no
`orgs` row exists. Lower to `(100, 200)` so even that path respects the
new 2× ratio.

## Non-goals

- Changing the per-second steady-state rate for any plan.
- Changing how `limit` is reported (stays `rps`, matches the pricing page).
- Adding a new `X-RateLimit-Burst` header. Nice-to-have, out of scope here.
- Fixing C2 / B3 / other test clusters.

## Rollout

- Branch: `nightly-maintenance-2026-10-12-rfc098-rate-limit-burst-tighten`.
- Files:
  - `src/plan.rs` — one function edit, one test update.
  - `src/rate_limit.rs` — two constant edits, one `remaining` clamp,
    two existing tests may need their `assert_eq!` burst values updated.
- Verification:
  - `cargo test rate_limit::` locally.
  - `cargo test plan::` locally.
  - After deploy: re-run C1 (30 parallel on a Free key) → expect ≥1 × 429.
- Rollback: `(10, 20) → (10, 100)` is a one-line revert if a legitimate
  customer flow depends on the old 10× burst (unlikely — RFC-083 just
  shipped).

## Risks

- **Customer-visible tightening.** Technically yes — a Free key that
  previously could spike 100 req/s momentarily now caps at 20. But no
  customer was promised 100-req/s burst (the pricing page says 10 r/s),
  and the plan ladder itself only shipped on 2026-10-12 so churn is
  minimal.
- **Test fixtures with hard-coded burst assumptions.** The two unit
  tests in `rate_limit.rs` set their own `burst_override`, so they
  don't depend on the defaults. The `plan::tests::rate_limit_defaults_per_plan`
  test needs its asserts updated.
