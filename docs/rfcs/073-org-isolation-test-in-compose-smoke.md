# RFC-073 — Run the org-isolation integration test in the compose-smoke lane

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Follows:** RFC-072 (quarantine→replay race-guard test)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 1 (tail)
**Severity:** P2 — closes the last "tenant isolation never runs in CI" gap

---

## Problem

RFC-068 wired the *self-contained* DB-backed tests into the `migrations-check`
job. It explicitly could **not** cover the four Class-2 tests, which need a
**running gateway** plus seeded API keys / contract IDs supplied via env vars:

| File | Test | Needs |
|---|---|---|
| `tests/rfc_001_isolation.rs` | `cross_org_ingest_is_rejected` | gateway + `TEST_API_KEY_B`, `TEST_CONTRACT_ID_A` |
| `tests/rfc_001_isolation.rs` | `soft_delete_hides_from_list` | gateway + `TEST_API_KEY_A`, `TEST_CONTRACT_ID_A` (mutates state) |
| `tests/rfc_001_isolation.rs` | `expired_invite_rejected` | Supabase service role + a pre-expired invite token |
| `tests/v1_ingest.rs` | ingest/idempotency/rate-limit | gateway + `TEST_API_KEY`, `TEST_CONTRACT_ID` |
| `tests/metrics.rs`, `tests/cli_push_pull.rs` | — | gateway (+ CLI config) |

The headline finding of the sale-readiness review was that the **cross-tenant
isolation** guarantee — the product's core selling point — never executes in
CI. `cross_org_ingest_is_rejected` is the test that proves it: org B's key
POSTing to org A's contract must be rejected.

## Decision — land the one test that matters, scope the rest as follow-ups

This RFC wires exactly **`cross_org_ingest_is_rejected`** into the existing
compose-smoke lane (`tests/compose_demo_smoke.sh`), which already:

- builds the gateway image and stands it up on `localhost:8080`,
- applies all migrations then `ops/postgres/seed/*.sql` via the initdb wrapper,
- seeds a fixed-UUID demo org (`cccccccc-…`).

That test is **read-only** (it asserts a 403/404, never mutates), so it is safe
to run against the shared smoke stack and needs no teardown. The other three
isolation/v1 tests are deferred (see Follow-ups) because each needs extra,
divergent seed (state mutation, a Supabase service role + expired invite, CLI
config) — bundling them would risk a half-wired lane at the cost of the one
guarantee that actually sells the product.

## Seed approach — fixed-UUID, deterministic (chosen over runtime provisioning)

Add a compose-only seed file `ops/postgres/seed/098_isolation_test.sql` (runs
before `099_demo_org.sql`; both are compose-only and never touch real
Supabase). It seeds, with memorable fixed UUIDs:

1. **Org A** (`aaaaaaaa-…`) and **Org B** (`bbbbbbbb-…`).
2. One **contract** owned by org A (`a000…0001`) with a matching `stable`
   `contract_versions` row so unpinned ingest resolves a live version.
3. Two **api_keys** rows — one per org — using precomputed
   `key_hash = base64(SHA-256(raw_key))` (the scheme `api_key_auth.rs` verifies,
   per migration 024). The raw keys are committed *only* in the smoke script as
   test fixtures (they grant access to nothing but this throwaway compose DB):

   | Env var | Raw key | prefix | org |
   |---|---|---|---|
   | `TEST_API_KEY_A` | `cg_live_orgA_isolationtest_000000000001` | `cg_live_orgA` | A |
   | `TEST_API_KEY_B` | `cg_live_orgB_isolationtest_000000000002` | `cg_live_orgB` | B |

   Precomputed hashes (standard base64, 44 chars):
   - A → `4NsMO1Zse0aEiNQ1A4a+wcHggWWHYJbYY/LqYkYaT7E=`
   - B → `EEr2wK1BxOubLC0prqT40FeuX0pWBwG2ihy9D+avBrs=`

`fixed-UUID` was chosen over **runtime API provisioning** because the seed runs
deterministically at DB init (no ordering races with gateway startup), mirrors
the existing `099_demo_org.sql` pattern exactly, and keeps the test's env wiring
to three plain `export` lines. Runtime provisioning would be more end-to-end but
adds startup-ordering fragility and JSON-parsing for no extra coverage of the
isolation logic under test.

### Open seed dependency (verify at run time)

`api_keys.user_id` is `NOT NULL REFERENCES auth.users(id)`. The compose Postgres
must already satisfy this for the demo-seeder's keys; the seed file will insert
a stand-in `auth.users` row (or reference the existing demo user) the same way.
**This is the one thing I cannot verify without running the stack** — if the
compose image provisions `auth.users` differently than assumed, the seed insert
is a one-line fix (point `user_id` at whatever the demo user's id is). Flagged
explicitly so the maintainer checks the first run's psql output.

## Changes

| File | Change |
|---|---|
| `ops/postgres/seed/098_isolation_test.sql` | New. Orgs A+B, one contract+stable version for A, two api_keys with precomputed SHA-256 hashes. `ON CONFLICT DO NOTHING`. |
| `tests/compose_demo_smoke.sh` | After the existing assertions, `export TEST_BASE_URL/TEST_API_KEY_A/TEST_API_KEY_B/TEST_CONTRACT_ID_A` and run `cargo test --test rfc_001_isolation -- --ignored --exact integration::cross_org_ingest_is_rejected`, with the same false-green grep guard used in `migrations-check` (`1 passed; 0 failed`). |
| docs/STATUS.md, MAINTENANCE_LOG.md | Bookkeeping rows. |
| docs/reviews/test-hardening-handoff-2026-05-28.md | Mark Task 1 tail item `cross_org_ingest_is_rejected` done; list remaining three as deferred. |

No product code, schema migration, or runtime change. The new SQL is
compose-only seed, not a Supabase migration.

## Verification (maintainer runs — no docker/cargo in the agent env)

```bash
bash tests/compose_demo_smoke.sh
```

Expect the existing smoke assertions to pass, then
`cross_org_ingest_is_rejected` to run and print `1 passed; 0 failed`. To prove
the test bites, temporarily relax the ingest org check and confirm the smoke
lane goes red. Check the first run's init-wrapper psql output for the
`auth.users` FK note above.

## Follow-ups (tracked, not in this RFC)

- `soft_delete_hides_from_list` — mutates contract state; needs a dedicated
  disposable contract in the seed so it can't corrupt the shared smoke stack.
- `v1_ingest` round-trip / idempotency / rate-limit — same seed shape as A's
  key; the rate-limit test sends 1001 requests (slow — gate behind an opt-in
  env flag).
- `expired_invite_rejected` — needs a Supabase service role + a pre-expired
  invite row; belongs with dashboard-auth integration, not the gateway smoke.
- `metrics`, `cli_push_pull`.
- Promote the smoke lane to block merges once it's proven stable.
