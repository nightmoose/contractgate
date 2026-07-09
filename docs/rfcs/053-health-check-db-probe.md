# RFC-053 — Real `/health` with a database probe

**Status:** Accepted  
**Date:** 2026-05-22  
**Implemented:** 2026-05-24  
**Branch:** nightly-maintenance-2026-05-24-rfc052-053  
**Addresses:** REVIEW-2026-05-22-launch-readiness H3  
**Severity:** P1 — high

---

## Problem

`health_handler` (`src/main.rs:681-687`) returns a hardcoded
`{"status":"ok"}` and never touches the database. A backend whose DB pool is
exhausted or whose Supabase project is down still reports healthy. The
Fly.io / load-balancer health check therefore cannot detect or drain a broken
instance, so traffic keeps routing to a node that `500`s every real request.

---

## Fix

1. **Liveness vs readiness.** Keep `/health` cheap and add a real readiness
   check:
   - `GET /health` — process is up. Cheap, no DB. Used as the liveness probe.
   - `GET /ready` — runs `SELECT 1` against the pool with a short timeout
     (e.g. 2 s). Returns `200` with `{"status":"ready","db":"ok"}` on success,
     `503` with `{"status":"degraded","db":"error"}` on failure.
2. **Point the platform check at `/ready`.** Update `fly.toml` and the
   `docker-compose.yml` healthcheck to probe `/ready`.
3. **Optional detail.** Include `version` and pool stats
   (`size`, `idle`) in the `/ready` body for operator visibility.

Splitting the two avoids a DB outage cascading into the orchestrator killing
otherwise-live pods (liveness should not depend on a dependency).

---

## Testing

- `/ready` returns `503` when the pool cannot serve `SELECT 1` (test with a
  closed pool).
- `/ready` returns `200` against a live DB; `/health` always `200`.
- Probe response stays under the platform's health-check timeout.

## What does NOT change

- `/health` remains public and unauthenticated.

## Rollout

Application change plus `fly.toml` / compose healthcheck path updates.
Document `/health` vs `/ready` in `docs/` (see RFC-057). Independent — ship
standalone.
