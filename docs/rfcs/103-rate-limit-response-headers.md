# RFC-103 — Standard `X-RateLimit-*` + `Retry-After` on every 429

**Status:** shipped 2026-10-13
**Driver:** 2026-10-12 E2E tester feedback: *"only 1 of the 20 429 responses included rate-limit headers, and none had a Retry-After. Clients need those to know when to back off."*

## Problem

Two code paths emit 429 for per-key rate-limiting:

1. **`src/v1_ingest.rs:390`** (versioned `/v1/*` ingest). Emits `X-RateLimit-Limit`, `-Remaining`, `-Reset`. **Missing `Retry-After`.**
2. **`src/main.rs:1141-1142` and `1183-1184`** (auth middleware → `AppError::RateLimitExceeded`). The middleware drops the `RateLimitOutcome` on the floor and `src/error.rs::into_response` renders just a text body — **no rate-limit headers, no `Retry-After`**.

The test loaded 30 concurrent requests from a Free key. The 20 denials were served mostly by path (2), explaining the "1 of 20" result the tester saw. Clients cannot back off intelligently because:

- `X-RateLimit-Reset` (unix seconds when the bucket refills) is absent → no absolute wake-up time.
- `Retry-After` (RFC 6585 seconds-to-retry) is absent on every path → no relative wake-up time.
- The only hint, `retry_after_ms`, is buried in the JSON body — forces every client to parse the body on a header-level signal.

## Design

### Carry the outcome into the response

`AppError::RateLimitExceeded` becomes `RateLimitExceeded(RateLimitOutcome)`. The middleware call sites pass the outcome through. In `IntoResponse` the 429 branch short-circuits with a header-full tuple.

### One helper, four headers

`RateLimitOutcome::headers()` returns the canonical set:

| Header | Source | Semantics |
|---|---|---|
| `X-RateLimit-Limit` | `outcome.limit` | req/s cap for this key |
| `X-RateLimit-Remaining` | `outcome.remaining` | tokens left this second (clamped to Limit per RFC-098) |
| `X-RateLimit-Reset` | `outcome.reset_unix` | absolute unix-seconds when bucket refills |
| `Retry-After` | `div_ceil(retry_after_ms, 1000)` | RFC 6585 seconds-to-retry |

Both call sites (`v1_ingest.rs` and the generic `IntoResponse`) use the same helper, so the four-header shape is a single source of truth.

### `Retry-After` is seconds, not milliseconds, not an HTTP-date

RFC 6585 §4 lists two forms: delta-seconds (preferred for short waits) and HTTP-date. We use delta-seconds because:

- Short waits are the common case for per-key limiting (bucket refills in sub-second to tens-of-seconds).
- Delta-seconds is a boring integer; HTTP-date parsers vary across clients.
- `retry_after_ms` stays in the JSON body as a finer-grained signal for clients that want sub-second backoff.

Ceiling (not floor) because advising a client to retry *before* the bucket refills defeats the purpose.

### Scope boundary: not touching `PlanLimitExceeded`

The monthly plan-cap 429 is a different animal: resets at the next calendar period, cap in the tens-of-thousands-of-seconds. Emitting `Retry-After: 2_592_000` is correct but hostile. That path keeps its richer JSON body (`upgrade_url`, `used`, `limit`); the client is expected to call `/account` to learn the period. Separate RFC if we decide to standardise it.

## Changes

- `src/rate_limit.rs` — `#[derive(Debug, Clone)]` on `RateLimitOutcome`; add `headers() -> [(&'static str, String); 4]`; add `denied_outcome_emits_four_headers_including_retry_after` test.
- `src/lib.rs` — `pub mod rate_limit;` (lifts the module into the library crate so `error.rs` can import `RateLimitOutcome`).
- `src/main.rs` — switch `mod rate_limit;` → `use contractgate::rate_limit;`; two middleware call sites pass `outcome` into `AppError::RateLimitExceeded(outcome)`.
- `src/error.rs` — new `use crate::rate_limit::RateLimitOutcome`; `RateLimitExceeded(RateLimitOutcome)` variant; the 429 branch of `IntoResponse` short-circuits with `outcome.headers()`; new `rate_limit_exceeded_response_has_all_four_headers` round-trip test.
- `src/v1_ingest.rs` — 429 path uses `rl_outcome.headers()` directly, picking up `Retry-After` without duplicating the header list.

## Non-goals

- Not changing `PlanLimitExceeded` (see scope boundary above).
- Not touching the 2xx `X-RateLimit-*` informational headers in `v1_ingest.rs` — those stay as the three-header `rate_limit_headers()` helper because `Retry-After` on a 2xx is meaningless.
- Not touching SDK retry logic — a follow-up will teach the Python/TS SDKs to honour `Retry-After` on 429 instead of fixed exponential backoff.

## Verification

- 7/7 `rate_limit` unit tests pass (1 new).
- 1/1 `error::tests::rate_limit_exceeded_response_has_all_four_headers` passes (round-trips an `AppError` through `IntoResponse` and asserts all four headers + body shape).
- Full `cargo test` suite green locally: 494 pass, 0 fail.
- `cargo fmt --check` clean.

After deploy, the tester's 30-parallel repro should produce 20 denials, **each** carrying all four headers and a `Retry-After` ≥ 1.
