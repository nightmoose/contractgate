# RFC-050 — Replace wildcard CORS with an explicit origin allowlist

**Status:** Accepted  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-22-rfc050  
**Addresses:** REVIEW-2026-05-22-launch-readiness B4  
**Severity:** P0 — launch blocker

---

## Problem

`build_router` (`src/main.rs:847-850`) installs:

```rust
let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any);
```

Every origin can call every route with any header. For a public multi-tenant
API whose dashboard delivers bearer tokens to the browser, this lets any
website on the internet drive the authenticated API surface with a victim's
credentials. It also blocks ever moving the dashboard session to a cookie
(wildcard origin cannot be combined with credentialed requests).

---

## Fix

Two CORS layers, scoped by router group:

1. **Authenticated surface** (`protected`, `infer`, `v1`): an explicit origin
   allowlist read from a `DASHBOARD_ORIGIN` env var (comma-separated; e.g.
   `https://app.datacontractgate.com`). Allow only the methods and headers the
   dashboard actually uses (`GET, POST, PATCH, DELETE`; `authorization,
   content-type`). No wildcard.
2. **Public surface** (`/health`, `/metrics`, `/openapi.json`, `/demo/*`,
   `/public-contracts`, `/catalog`, `/published/*`): a separate permissive
   `allow_origin(Any)` layer — these expose no tenant data and benefit from
   being embeddable.

Startup behaviour: if `DASHBOARD_ORIGIN` is unset, log a warning and fall back
to `http://localhost:3000` (dev). Production deploy sets it explicitly via
`fly secrets`.

---

## Testing

- Preflight `OPTIONS` from an allowed origin succeeds; from an unlisted origin
  the response carries no `Access-Control-Allow-Origin`.
- `/health` and `/catalog` still respond to any origin.
- Dashboard end-to-end still works against the configured origin.

## What does NOT change

- Auth logic, rate limiting, body limits — untouched.

## Rollout

Application change plus one new env var. Add `DASHBOARD_ORIGIN` to
`.env.example`, the Fly secrets, and the Docker/compose configs. Independent of
other RFCs — ship standalone.
