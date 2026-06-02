# RFC-042: P1 Abuse-Prevention Bundle

**Status:** Accepted  
**Branch:** `dev/p1-abuse-prevention`  
**Fixes:** P1-1, P1-2, P1-3 from REVIEW-2026-05-16-saas-readiness.md

## Problem

### P1-1 — Rate limiting only wired for `POST /v1/ingest/{id}`

The token-bucket rate limiter (`src/rate_limit.rs`) exists and works but
`require_api_key` never calls it.  Only `v1_ingest.rs` explicitly consumes a
token.  Every other protected endpoint (`/contracts`, `/audit`, `/ingest`,
`/egress`, `/scorecard`, etc.) has no per-key limit — a single API key can
hammer the database or validation engine without throttling.

### P1-2 — No request body size limit on most endpoints

`tower_http::limit::RequestBodyLimitLayer` is applied only to the `v1` ingest
sub-router (10 MB).  All other routes accept unbounded bodies.  A client can
POST a multi-MB YAML blob to `/contracts`, `/contracts/infer/*`, or any other
endpoint and exhaust memory.

### P1-3 — `/playground/validate` is public and unlimited

`/playground/validate` sits in the `public` router (no auth, no size limit).
Any unauthenticated client can POST arbitrarily large or expensive contracts
and payloads and pin the validation engine indefinitely.

## Decision

Ship all three together as a single "hardening layer" applied in `build_router`.

### P1-1 fix — move rate-limit check into `require_api_key`

After a key is validated (DB-backed or legacy), call
`state.rate_limiter.check(key_id)`.  JWT sessions (nil key UUID) get a
separate, generous default limit so the dashboard is not throttled.
No change to the `ValidatedKey` struct needed.

### P1-2 fix — global body size limit

Apply `RequestBodyLimitLayer` (1 MB default) to the `protected` router.
Routes that need more (infer endpoints accepting large schemas) can be carved
out or given a higher limit.  The existing 10 MB limit on `v1` stays.

### P1-3 fix — auth + size limit on `/playground/validate`

Move `/playground/validate` from `public` to `protected` (requires a valid
API key or JWT session).  Apply a 256 KB body limit specific to that route.

## Files changed

- `src/main.rs` — rate-limit in `require_api_key`; body limit on `protected`;
  move playground route; playground-specific size limit
- `src/rate_limit.rs` — expose a `check_key` method usable by nil UUID (JWT)
