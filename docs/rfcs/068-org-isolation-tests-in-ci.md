# RFC-068 — Run the org-isolation DB tests in CI

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc068-isolation-ci`
**Follows:** RFC-067 (request-path panic hardening)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 1
**Severity:** P1 — the multi-tenant isolation guarantee is asserted in code but
never executed in CI

---

## Problem

CI runs plain `cargo test` (`.github/workflows/ci.yml`), which **skips every
`#[ignore]`d test**. The DB-backed org-isolation tests are ignored, so the suite
that proves cross-tenant isolation — the product's core selling point — never
runs on any PR.

The handoff doc proposed simply adding `cargo test -- --ignored` to a job that
has Postgres. That is **not** sufficient as written: the ignored tests fall into
two distinct classes, and only one is runnable without standing up a full
gateway + external seed data.

### Class 1 — self-contained (need only a migrated Postgres)

These connect to `DATABASE_URL` via `PgPool::connect`, build their own data
through `storage::*`, assert the cross-org denial, and clean up after
themselves:

- `tests::org_scoping::two_org_get_contract_isolation`
- `tests::org_scoping::two_org_get_version_isolation`
- `tests::rfc053_ready_tests::ready_returns_200_live_db`

### Class 2 — need a running gateway and/or external seed (left ignored)

These hit a live HTTP server (`TEST_BASE_URL`) with pre-seeded API keys /
contract IDs supplied via env vars, or need a Supabase service role:

- `tests/rfc_001_isolation.rs::integration::*`
- `tests/v1_ingest.rs::integration::*`
- `tests/metrics.rs::integration_metrics_endpoint_live`
- `tests/cli_push_pull.rs::push_and_pull_round_trip`

Class 2 belongs to the existing compose-smoke lane (which already boots a real
gateway), not the migrations job. Wiring them here would require duplicating the
seed/gateway setup and is deliberately **out of scope** for this RFC.

## The blocker the handoff missed

The Class-1 two-org tests generate **random** org UUIDs and call
`storage::create_contract(..., Some(org_a))`. But `contracts.org_id` is
`references public.orgs(id)` (migration 007). The tests do **not** insert those
`orgs` rows — they implicitly relied on a pre-seeded dev/compose database (which
seeds a demo org). Against a bare migrated CI Postgres, `create_contract` fails
the foreign key. This is the real reason these tests were never wired up.

### Fix: make the tests self-seeding

Each test now inserts its two `orgs` rows up front and removes them in cleanup.
`orgs` requires `id`, `name`, `slug` (unique, non-empty); `plan` defaults to
`'free'`. Because `contracts.org_id` is `ON DELETE CASCADE`, deleting the org
also removes the contract, so cleanup is a single delete per org and the
existing `DELETE FROM contracts` becomes redundant (kept as belt-and-suspenders,
ordered before the org delete).

This makes the tests runnable against **any** migrated Postgres with zero
external seed data — which is exactly what makes them CI-safe.

## Changes made

| File | Change |
|---|---|
| `src/tests.rs` | Both two-org tests insert `orgs(id, name, slug)` for `org_a`/`org_b` before `create_contract`; cleanup deletes those org rows (cascade removes contracts). Slugs derived from the UUID to stay unique across reruns. |
| `.github/workflows/ci.yml` | New step in `migrations-check` (after migrations apply) running the three self-contained ignored tests by name against the CI Postgres, with `SQLX_OFFLINE=false` already set for that job. |

## CI wiring

The `migrations-check` job already stands up Postgres 16 and applies every
migration in order. The new step runs **after** that, reusing the same
`DATABASE_URL`:

```yaml
- name: Run self-contained DB-backed tests (org isolation + readiness)
  run: |
    set -o pipefail
    cargo test --bin contractgate-server -- --ignored --exact \
      tests::org_scoping::two_org_get_contract_isolation \
      tests::org_scoping::two_org_get_version_isolation \
      tests::rfc053_ready_tests::ready_returns_200_live_db \
      2>&1 | tee /tmp/iso_test.out
    if ! grep -qE '3 passed; 0 failed' /tmp/iso_test.out; then
      echo "::error::Expected '3 passed; 0 failed' ..."; exit 1
    fi
```

Two non-obvious correctness details:

- **`--bin contractgate-server`, not `--lib`.** These tests live in
  `mod tests` inside `src/main.rs`, which is the `contractgate-server` binary
  target. Running `--lib` would match zero tests.
- **Count assertion.** `cargo test` exits 0 even when a filter matches *zero*
  tests, so a future rename of any test would silently turn this step green
  without running it. The `grep` for `3 passed; 0 failed` makes that failure
  loud instead.

Naming the tests explicitly (rather than a blanket `--ignored`) keeps the
gateway-dependent Class-2 tests from running here and failing for lack of a
server — a blanket `--ignored` in this job would go red immediately.

## Rollout / migration

No product code, schema, or config change. Test-only + CI-only. After this, a
change that breaks org scoping in `storage.rs` fails CI on the PR instead of
shipping. Class-2 end-to-end isolation coverage remains future work (track in
the compose-smoke lane).

## Verification

`cargo test` (unit, unchanged) + the new explicit `--ignored` invocation against
a migrated Postgres, both green. Locally:

```bash
DATABASE_URL=postgres://... \
cargo test --bin contractgate-server -- --ignored --exact \
  tests::org_scoping::two_org_get_contract_isolation \
  tests::org_scoping::two_org_get_version_isolation \
  tests::rfc053_ready_tests::ready_returns_200_live_db
```
