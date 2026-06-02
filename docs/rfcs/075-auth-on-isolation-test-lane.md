# RFC-075 — Auth-on test lane for cross-tenant isolation

**Status:** Draft
**Date:** 2026-05-29
**Branch:** TBD
**Depends on:** RFC-074 (org-ownership enforcement on the data plane)
**Supersedes the verification claim in:** RFC-073

---

## Problem

RFC-073 wired `cross_org_ingest_is_rejected` into `tests/compose_demo_smoke.sh`
to prove tenant isolation end-to-end in CI. It does not prove it.

The compose stack starts the gateway with `CONTRACTGATE_DEV_NO_AUTH=1`
(`docker-compose.yml`). That flag makes `require_api_key` (`src/main.rs`)
short-circuit: it runs the request without validating the `x-api-key` header and
**without attaching a `ValidatedKey`**. With no `ValidatedKey`, the handler's
`org_id` is always `None`, the org filter matches every org, and any key is
accepted for any contract.

Consequence: `cross_org_ingest_is_rejected` returns **200 regardless** of whether
the RFC-074 org-scoping fix is present. The smoke wiring is a false signal — it
went red while proving nothing, and a green result would have been equally
meaningless. The block has been disabled in `compose_demo_smoke.sh` (commented
out, 2026-05-29) pending this RFC.

The demo-seeder depends on `DEV_NO_AUTH=1` to publish contracts via `x-org-id`,
so we cannot simply flip the flag on the existing stack without breaking seeding.

## Goal

Run `cross_org_ingest_is_rejected` (and, later, the other auth-dependent
Class-2 isolation tests) against a gateway with **auth ON** (`DEV_NO_AUTH=0`),
using real seeded API keys, so the test actually exercises the org-scope check
that RFC-074 added.

## Approach (recommended)

A second, auth-on gateway instance alongside the existing demo stack, rather
than flipping the flag on the shared one.

1. **Add an `gateway-authon` service** to a dedicated compose file
   (`tests/compose.isolation.yml`, layered over the base compose) that runs the
   same image with `CONTRACTGATE_DEV_NO_AUTH=0` on a separate port (e.g. 8081),
   pointed at the same Postgres. The demo stack keeps `DEV_NO_AUTH=1` for
   seeding; the auth-on instance is only used by the isolation test.

2. **Seeding stays on the dev-no-auth instance.** `098_isolation_test.sql`
   already seeds the two orgs, the contract, and the two API keys directly in
   Postgres (precomputed `base64(SHA-256(raw_key))` hashes), so it needs no
   gateway at all — it runs at initdb time. No change required.

3. **Point the test at the auth-on port.** In the isolation step, export
   `TEST_BASE_URL=http://localhost:8081` and the existing
   `TEST_API_KEY_A/B` + `TEST_CONTRACT_ID_A`, then run the one `--ignored
   --exact` test with the same `1 passed; 0 failed` false-green guard that RFC-073
   already wrote.

4. **Sanity gate before the assertion.** First confirm auth is actually on:
   a request with **no** key (or a garbage key) to the auth-on instance must
   return 401. If it returns 200, the instance is misconfigured (still
   no-auth) and the lane should fail loudly rather than silently pass the
   isolation assertion. This is the guard whose absence let RFC-073 mislead us.

## Why not just flip the existing flag

The seeder publishes contracts over HTTP using `x-org-id` and no key, which only
works under `DEV_NO_AUTH=1`. Flipping the shared instance to auth-on breaks
seeding; standing up a second instance is cheaper than rewriting the seeder to
mint and use real keys.

## Test matrix (phased)

| Phase | Tests | Notes |
|---|---|---|
| 1 (this RFC) | `cross_org_ingest_is_rejected` | + the 401 sanity gate |
| 2 (follow-up) | `soft_delete_hides_from_list`, `v1_ingest` round-trips, `expired_invite_rejected`, `metrics`, `cli_push_pull` | the remaining Class-2 set tracked in the test-hardening handoff |

## Verification

```bash
cargo test
docker compose -f docker-compose.yml -f tests/compose.isolation.yml up -d
bash tests/compose_demo_smoke.sh   # isolation step re-enabled, pointed at :8081
```

Expect: no-key request → 401 (sanity gate); org B key → org A contract → 404
(isolation). To prove the test bites, revert RFC-074's `org_id` arg on
`src/ingest.rs` and confirm the isolation step goes red while the 401 gate stays
green.

## Out of scope

- `ingest_stats_handler` no-auth read exposure (separate finding; needs its own
  auth-signature change — track separately if/when scheduled).
