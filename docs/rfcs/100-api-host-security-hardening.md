---
title: API-host security — fail-closed /metrics + global security headers
status: Accepted
author: product owner
date: 2026-10-12
---

# RFC-100 — API host security hardening (cluster 5 of 6)

## Problem

The 2026-10-12 E2E found two security issues on `api.datacontractgate.com`
(the Rust Axum backend now served directly from Fly after commit
`44887e0`, no longer behind the Next.js dashboard):

- **F4**: `/metrics` returned HTTP 200 to unauthenticated requests. The
  Prometheus-text body includes per-route counters that have accumulated
  entries from scanner probes (`/wp-login.php`, `/.env`, etc.) — valuable
  reconnaissance signal.
- **F5**: No security response headers on any endpoint. Missing HSTS,
  `X-Content-Type-Options`, `X-Frame-Options`, `Content-Security-Policy`,
  `Referrer-Policy`.

### F4 root cause

`src/observability.rs:83-116` was deliberately open-by-default per
RFC-016 §Q3/Q4: "if `METRICS_AUTH_TOKEN` is set, require Bearer; else
open." This was chosen so a local `docker-compose up` Prometheus could
scrape without credentials — fine for dev. Prod was supposed to set
`METRICS_AUTH_TOKEN` in Fly; it wasn't.

The fix must preserve easy dev scraping while making prod fail-closed
with zero config.

### F5 root cause

Before `44887e0`, API traffic went `client → app.datacontractgate.com
(Next.js, Vercel) → Fly API`. Vercel sets HSTS, X-Content-Type-Options,
and friends at its edge. After `44887e0`, API traffic goes direct to Fly,
and the Rust backend sets no headers of its own. `tower-http`'s
`set-header` feature is not enabled; `fly.toml` has no header injection.

## Decision

### 1. Flip `/metrics` to fail-closed by default

New env var `METRICS_PUBLIC`. Behavior matrix:

| `METRICS_AUTH_TOKEN` | `METRICS_PUBLIC` | Result                                   |
|---|---|---|
| set                  | any              | Bearer required (existing behavior)      |
| unset                | `"1"`            | Open (dev/compose opt-in)                |
| unset                | unset / other    | **401 Unauthorized** (new, prod default) |

`src/observability.rs::metrics_handler` picks up both env vars at request
time (same pattern as the existing Bearer check).

Operational prereqs (not in this PR but documented in Rollout):

- Set `METRICS_AUTH_TOKEN=<32-byte random>` in Fly.
- Update `ops/prometheus/prometheus.yml` to pass the Bearer in the scrape
  config.
- Local compose keeps the status quo via `METRICS_PUBLIC=1` in
  `docker-compose.yml`.

A startup log line prints which auth mode is active, so a prod deploy
without the token produces an obvious log signal.

### 2. Global security headers middleware

New `src/security_headers.rs` with a `middleware::from_fn` layer applied
in `build_router` **outside** CORS and tracing so it wraps every response
(including 404s, 429s, and `/metrics`-401s).

Headers set:

| Header | Value | Why |
|---|---|---|
| `Strict-Transport-Security` | `max-age=31536000; includeSubDomains` | HTTPS enforcement, 1 year (no `preload` to avoid an irreversible commitment at launch). |
| `X-Content-Type-Options` | `nosniff` | Blocks MIME-sniff. |
| `X-Frame-Options` | `DENY` | Clickjacking defence; API never framed. |
| `Content-Security-Policy` | `default-src 'none'; frame-ancestors 'none'` | API returns JSON/plaintext; no scripts, no framing. |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Prevents full URL leak on cross-origin fetch. |

Only set when the header isn't already present, so a handler that wants
a specific CSP (none currently) could override.

### 3. Tests

- Rename `metrics_endpoint_open` → `metrics_endpoint_public_flag` to
  explicitly test `METRICS_PUBLIC=1` path.
- Add `metrics_endpoint_closed_by_default` — no token, no public flag →
  401.
- Keep `metrics_endpoint_bearer_auth` as-is.
- Add integration test `security_headers_applied` under `tests/` asserting
  all five headers appear on `/health`.

## Non-goals

- API key auth on `/health`, `/ready`, `/openapi.json` — the test brief
  explicitly says these stay 200 unauthenticated. Security headers still
  apply.
- HSTS preload submission. 1-year `max-age` without `preload` is the
  conservative first step; preload comes after the ladder has baked.
- CSP for the Next.js dashboard — different host, already covered by
  Vercel edge headers; separate concern.

## Rollout

- Files:
  - `src/observability.rs` — ~10 lines (new flag check, startup log).
  - `src/security_headers.rs` — ~40 lines (new file).
  - `src/main.rs` — ~5 lines (mod decl, startup-log call, layer wiring).
  - `tests/metrics.rs` — rename + add two tests.
  - `tests/security_headers.rs` — new integration test (~40 lines).
  - `docker-compose.yml` — swap `METRICS_AUTH_TOKEN: ""` for
    `METRICS_PUBLIC: "1"`.
  - `ops/prometheus/prometheus.yml` — add Bearer scrape config for prod
    scraper (hostname-conditional block, so the compose scraper is
    unaffected).
- Verify: `cargo check`, `cargo test`.
- Deploy prereq: `fly secrets set METRICS_AUTH_TOKEN=<...>` **before**
  this ships, or `/metrics` scraping breaks for the window between the
  deploy and the secret being set.

## Risks

- **Prometheus scraping breaks until the token is set in Fly.** Mitigation:
  set the secret before the deploy; the startup log will confirm the
  active mode.
- **CSP `default-src 'none'`** could in theory block a browser extension
  trying to render the JSON. Rare and user-controlled; standard for API
  hosts.
- **HSTS 1-year is sticky.** Browsers cache the policy. If we ever need
  to downgrade `api.datacontractgate.com` to HTTP (we won't), users would
  continue to upgrade for a year. Acceptable risk for a stable prod API.
