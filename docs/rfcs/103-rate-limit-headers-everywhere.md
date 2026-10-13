# RFC-103: standardize rate-limit response headers on every 429

**Status:** proposed → implemented
**Owner:** maintenance
**Depends on:** RFC-098 (burst tightening and `X-RateLimit-Remaining` clamping)

## Problem

E2E retest on 2026-10-13 confirmed RFC-098's burst tightening works (10 of 30 parallel allowed, 20 denied), but exposed a client-DX gap:

- Only **1 of 20** 429 responses carried any `X-RateLimit-*` headers.
- **Zero** 429 responses carried `Retry-After`.

Without those headers, clients can't do intelligent backoff. SDKs can only guess at wait time, and the generic advice "retry on 429" degrades to blind exponential backoff.

### Root cause

Two 429 code paths exist in the backend; they disagree on headers:

| Path | Site | X-RateLimit-* | Retry-After |
|---|---|---|---|
| Versioned ingest | `src/v1_ingest.rs:390-399` | ✓ | ✗ |
| Auth middleware → generic error | `src/main.rs:1142/1184` → `src/error.rs:155` (`AppError::RateLimitExceeded`) | ✗ | ✗ |
| Plan monthly cap | `AppError::PlanLimitExceeded` → `src/error.rs:157-177` | ✗ (body only) | ✗ |

The middleware path emits `AppError::RateLimitExceeded` as a unit variant — the `RateLimitOutcome` is dropped on the floor. `error.rs::into_response` then renders the 429 as a text body with no response headers.

Nothing anywhere emits `Retry-After` — only an in-body `retry_after_ms` field, which generic HTTP clients (reqwest, httpx, fetch) can't see without parsing the body.

## Fix

1. **Carry the outcome through the error.** Change `AppError::RateLimitExceeded` → `AppError::RateLimitExceeded(RateLimitOutcome)` so the limit/remaining/reset/retry fields survive into `IntoResponse`.
2. **Shared header helper.** `RateLimitOutcome::headers()` returns the canonical 4-tuple `[X-RateLimit-Limit, X-RateLimit-Remaining, X-RateLimit-Reset, Retry-After]` with `Retry-After` in **seconds** (RFC 6585), computed as `retry_after_ms.div_ceil(1000)`.
3. **Attach headers on every 429.** `error.rs::into_response` for `RateLimitExceeded(outcome)` and `v1_ingest.rs:390-399` both use `outcome.headers()`.
4. **Module scoping.** `rate_limit` moves from binary-only (`main.rs`) to `lib.rs` so `error.rs` (part of the lib) can import `RateLimitOutcome`.
5. **Test.** Unit test in `error.rs` asserts a `RateLimitExceeded(outcome)` response carries all four headers, that `Retry-After` is a ceiled seconds integer (1250 ms → 2 s), and that the body still contains `status: 429` and `retry_after_ms`.

## Out of scope

- **`AppError::PlanLimitExceeded`** (monthly cap). Separate semantics — the retry window is "end of calendar month," not "milliseconds from now." Leaving it on the current JSON-body-only response and tackling it in a follow-up RFC if clients need the header there too.
- **Changing the IETF-draft header family.** Keep `X-RateLimit-*`; do not rename to `RateLimit-*` (the newer draft form) yet — would break clients already parsing the X-prefixed names.

## Verification

- `cargo test` 315/315 lib + all integration suites pass; new `error::tests::rate_limit_exceeded_response_has_all_four_headers` guards the regression.
- Post-deploy retest: fire ≥20 parallel requests from a Free key against `app.datacontractgate.com/ingest/{id}`; every 429 must include all four headers.
