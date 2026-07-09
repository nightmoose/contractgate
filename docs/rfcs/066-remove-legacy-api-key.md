# RFC-066 — Remove the legacy env-var `API_KEY` master key

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc065-ingest-egress-scope` (bundled with RFC-065)
**Follows:** RFC-065 (ingest/egress contract-scope enforcement)
**Addresses:** docs/reviews/sale-readiness-review-2026-05-28.md — legacy master-key risk
**Severity:** P0 — cross-tenant authorization (the last broad-impersonation path)

> **Scope decision (Alex, 2026-05-28):** "Master key only." Remove the env-var
> master key + its `x-api-key` acceptance branch (the prod impersonation risk).
> Keep dev-mode no-auth — which the demo/compose/CI-smoke stack depends on — but
> re-key it on an **explicit** `CONTRACTGATE_DEV_NO_AUTH=1` flag (defaults off)
> so it can never silently turn on in production. Full removal of dev-mode was
> rejected as larger, untestable churn into the onboarding stack.

---

## Problem

The env-var `API_KEY` is a single global secret that, when presented as
`x-api-key`, authenticates with **no org and no contract scope**. RFC-065 closed
the scoped-key gap on ingest/egress, but a holder of this env-var key still
bypasses all of it: it is issued implicitly as an unrestricted key
(`allowed_contract_ids: None`, `org_id: None`), so it can read/write every
contract in every org.

## The entanglement, and how it was resolved

`state.api_key` did **two** jobs, coupled through one field:

1. The legacy master key itself (`provided == state.api_key`).
2. The implicit **dev-mode no-auth switch**: when `api_key` was empty,
   `require_api_key` passed every request through unauthenticated, and
   `auth_configured()` keyed off it.

Removing job (1) naively would leave dev-mode "always on" → every request open.
The compose/demo/CI-smoke stack genuinely depends on dev-mode (the demo-seeder
posts with `x-org-id` and no real key). So the two jobs were **split**:

- Job (1) — **deleted**. No env-var master key exists anymore.
- Job (2) — **kept but made explicit**. A new `dev_no_auth: bool` on `AppState`,
  driven by `CONTRACTGATE_DEV_NO_AUTH=1`, defaults to `false`. `auth_configured()`
  is now simply `!dev_no_auth`.

## Changes made

| File | Change |
|---|---|
| `src/main.rs` | Replaced `api_key: String` field with `dev_no_auth: bool`; `AppState::new` takes `dev_no_auth`; `auth_configured()` = `!dev_no_auth`; deleted the legacy `provided == state.api_key` branch; dev-mode passthrough now gated on `state.dev_no_auth`; startup reads `CONTRACTGATE_DEV_NO_AUTH` instead of `API_KEY`; updated `org_id_from_req` comment. |
| `src/tests.rs` | 3 `AppState::new(pool, String::new(), …)` fixtures → `…, true, …` (preserve dev-mode-unscoped intent); updated stale comment. |
| `docker-compose.yml` | Added `CONTRACTGATE_DEV_NO_AUTH: "1"` to the `gateway` service so `make demo` + CI smoke keep working. |
| `.env.example` | Removed `API_KEY`; documented `CONTRACTGATE_DEV_NO_AUTH`. |
| `docs/auth-reference.md` | Replaced the legacy-key section, the before/after table, and the resolution-order list. |

After this, the only ways to authenticate are: **Bearer JWT** (dashboard) or a
**DB-backed `x-api-key`** (connectors/SDKs). A request with neither → 401, unless
`CONTRACTGATE_DEV_NO_AUTH=1` (local only).

The demo-seeder needs no change: it hits the gateway in dev mode (passthrough),
and its now-meaningless `cg_demo_key` is simply ignored.

## Rollout / migration

Breaking for any caller that was using the env-var key as a credential. Confirm
connectors/SDKs issue DB-backed keys before deploying (the seeder/demo paths use
dev-mode, not a credential). No DB migration. `README.md` had no `API_KEY`
quick-start to change.

## Verification

`cargo check` + `cargo test` (incl. the rewritten dev-mode/legacy tests). Manual:
a request with no `Authorization` and no `x-api-key` returns 401; a DB-backed
key still works; an unknown `x-api-key` returns 401.
