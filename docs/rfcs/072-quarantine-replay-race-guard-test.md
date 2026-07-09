# RFC-072 — Quarantine→replay race-guard test

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Follows:** RFC-071 (coverage ratchet gate)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 3 (quarantine/replay coverage)
**Severity:** P3 — test hardening; no runtime impact

---

## Problem

Task 3 of the handoff calls for quarantine/replay coverage. The pure surface of
the replay path is already well-tested (`validate_bounds` across all bounds,
the outcome `tally` over every category, serde round-trips — see
`src/replay.rs` tests). The genuine untested gap is the **race guard** inside
`storage::mark_quarantine_replayed_batch`: the conditional UPDATE
(`WHERE status IN ('pending','reviewed') AND replayed_at IS NULL`) that ensures
two concurrent replays of the same source row stamp it at most once. A
regression dropping that predicate would silently allow double-replay — a row
stamped twice, linking to the wrong audit id, with no test to catch it.

Inference happy-path coverage (the other Task 3 tail item) was **investigated
and skipped**: all five formats (csv/avro/proto/openapi/url) already have
basic-types/happy-path tests. Marginal value did not justify a new increment.

## Decision

Add one self-seeding Class-1 DB integration test
(`tests::org_scoping::quarantine_replay_race_guard`, `#[ignore]`d, gated on
`DATABASE_URL`) that drives the real storage lifecycle:

1. Seed an org + contract, then a single `pending` quarantine row via
   `quarantine_event`.
2. Call `mark_quarantine_replayed_batch` **twice** with two distinct audit ids,
   simulating two concurrent replays of the same source.
3. Assert the **first** call returns the source id (won the UPDATE) and the
   **second** returns empty (lost — status no longer `pending`/`reviewed`).
4. Read the row back via `list_quarantine_by_ids` and assert it is stamped
   `replayed` exactly once and links to the **first** (winning) audit id.

Class-1 (RFC-068): needs only a migrated Postgres, no running gateway or seed
data. It self-seeds and cleans up, so it runs against any migrated DB.

## Why test sequentially, not with real concurrency

The guard is a single conditional SQL statement; its correctness is in the
`WHERE` predicate, not in client-side timing. Two sequential calls exercise the
exact same predicate path a true race would — the second call sees the stamped
row and matches zero rows — without flaky timing. A genuinely concurrent test
would add nondeterminism for no extra coverage of the guard logic.

## Changes made

| File | Change |
|---|---|
| `src/tests.rs` | New `quarantine_replay_race_guard` test in the `org_scoping` mod. Self-seeds org+contract+quarantine row; asserts only the first `mark_quarantine_replayed_batch` marks the row and it links to the winning audit id. |
| `.github/workflows/ci.yml` | Added the test to the `--ignored --exact` list in the migrations-check step; bumped the false-green guard from `3 passed` to `4 passed`. |

No product code, schema, or migration change.

## Verification

Per the no-cargo-here constraint, the maintainer runs:

```bash
bash scripts/test-db-up.sh            # throwaway Postgres + migrations
DATABASE_URL=postgres://contractgate:contractgate@localhost:5432/contractgate_test \
  cargo test --bin contractgate-server -- --ignored --exact \
    tests::org_scoping::quarantine_replay_race_guard
bash scripts/test-db-up.sh --down     # tear down
```

Expect `1 passed; 0 failed`. To prove the guard bites, temporarily drop
`AND qe.status IN ('pending','reviewed') AND qe.replayed_at IS NULL` from
`mark_quarantine_replayed_batch` and confirm the second-mark assertion fails.
