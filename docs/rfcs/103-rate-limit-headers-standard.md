# RFC-103: Standard rate-limit headers on every 429

## Problem
Follow-up from the 2026-10-12 E2E (C1 retest). Of 20 429 responses from a
30-parallel Free-key burst, only one carried `X-RateLimit-*` headers and
**none** carried `Retry-After`. Clients have no reliable way to decide when to
back off, so they either retry too quickly (compounding the overload) or wait
a long static timeout (bad DX).

Two 429 code paths exist in the API and they diverged over time:

1. **`src/v1_ingest.rs:390-399`** (versioned `/v1/*` endpoints): emits
   `X-RateLimit-Limit/Remaining/Reset` on the 429, but no `Retry-After`.
2. **`src/main.rs:1136-1145` and `1178-1189`** (middleware for `/ingest`,
   dashboard JWT): returns `Err(AppError::RateLimitExceeded)` with the
   `RateLimitOutcome` discarded. `src/error.rs:155` then renders a bare
   `(StatusCode::TOO_MANY_REQUESTS, body)` with no headers at all.

A third 429 path exists (`AppError::PlanLimitExceeded`, monthly plan cap)
that is out of scope here — its reset lifecycle is weeks/months and warrants
a separate pass.

## Change

### `src/rate_limit.rs`
- `#[derive(Debug, Clone)]` on `RateLimitOutcome` so it can live inside an
  error enum.
- New method `RateLimitOutcome::headers(&self) -> [(&'static str, String); 4]`
  returning the canonical 429 header set. `Retry-After` is in **seconds**
  (RFC 6585), ceiling of `retry_after_ms`.

### `src/error.rs`
- `AppError::RateLimitExceeded` → `AppError::RateLimitExceeded(RateLimitOutcome)`.
- `IntoResponse` arm attaches `outcome.headers()` and returns a JSON body
  `{error, status, retry_after_ms}` instead of the previous bare text body.

### `src/main.rs`
- Both middleware call sites (JWT path at line 1142, API-key path at line
  1184) now pass the `outcome` into the error variant.

### `src/v1_ingest.rs`
- The 429 response now calls `rl_outcome.headers()` instead of hand-rolling
  the three `X-RateLimit-*` tuples. This picks up `Retry-After` on the
  versioned path too.
- Success-path header attachment is unchanged — `Retry-After` is only
  meaningful on a denied response.

### Test (`src/error.rs`)
- `rate_limit_exceeded_response_has_all_four_headers`: constructs an
  `AppError::RateLimitExceeded(outcome)`, converts to a `Response`, asserts
  all four headers are set with the right values (including that 1,250 ms
  rounds up to `Retry-After: 2`), and the JSON body includes `status: 429`
  + `retry_after_ms`.

## Out of scope
- `AppError::PlanLimitExceeded` — needs seconds-until-end-of-period logic.
  Follow-up RFC when a tester or client reports it.

## Verified locally
- `cargo check` clean.
- `cargo test` 493+ tests pass (new test green; 0 regressions).
- `cargo fmt --check` clean.
