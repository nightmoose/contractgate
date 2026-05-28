# Test-Hardening Handoff — ignored tests + CI coverage gate

**Date:** 2026-05-28
**For:** Sonnet (execute when there are tokens to burn)
**Context:** Sale-readiness review flagged thin tests + no coverage enforcement.
This doc scopes the concrete work. RFC-first if any item grows non-trivial.

---

## Headline finding (fix first)

CI runs plain `cargo test` (`.github/workflows/ci.yml:66`), which **skips every
`#[ignore]`d test**. Those ignored tests are the DB-backed **integration** tests,
including cross-tenant isolation. So the suite that proves tenant isolation
**never runs in CI** — even though the `Migrations` job already stands up a
Postgres (`ci.yml:144+`). For a product whose core selling point is multi-tenant
contract enforcement, that's the single most important gap.

### Ignored tests (won't run in CI today)

| File | Count | What it protects |
|---|---|---|
| `tests/rfc_001_isolation.rs` | 1 | **Org isolation** — the tenant-boundary guarantee |
| `tests/v1_ingest.rs` | 1 | v1 ingest hot path against live DB |
| `tests/cli_push_pull.rs` | 1 | CLI deploy/pull round-trip |
| `tests/metrics.rs` | 1 | Prometheus metrics endpoint |
| `src/tests.rs` | 3 | two-org DB tests; live-pool 200-path |

All are gated on `DATABASE_URL` and tagged `#[ignore]` — correct hygiene, but
nothing ever runs them.

## Task 1 — Run DB-backed tests in CI

Add a job (or extend the existing `Migrations` job, which already has Postgres +
applied migrations) that runs:

```bash
cargo test -- --ignored
```

with `DATABASE_URL` pointing at the CI Postgres. Acceptance: `rfc_001_isolation`
and the two-org `src/tests.rs` tests execute and pass in CI on every PR. This
alone converts the isolation guarantee from "asserted in code" to "enforced on
every change."

## Task 2 — Coverage gate

Add `cargo-llvm-cov` to CI and report total line coverage. Start with a **low,
non-blocking** threshold (e.g. fail only if coverage drops below the current
measured baseline) so it stops backsliding without a big upfront test-writing
push. Wire it into `ci.yml` as a step after the test job; upload the report as a
CI artifact (and/or Codecov if desired).

Recommendation: ratchet-style gate (fail on *decrease*) over a fixed high bar —
it's the cheapest way to make coverage monotonic.

## Task 3 — Expand coverage on critical paths

Priority order (highest acquisition/risk value first):

1. **Auth** (`api_key_auth.rs`, `jwt_auth.rs`, `require_api_key` in `main.rs`) —
   valid/invalid/expired JWT, DB-key hit/miss, contract-scope (RFC-065), and —
   once RFC-066 lands — no-auth → 401.
2. **Storage org-scope guards** (`storage.rs`, RFC-047) — wrong-org returns 404,
   never leaks existence. Many `get_*`/`patch_*`/`delete_*` fns take an `org_id`
   guard; test the `Some(other_org)` denial path.
3. **Quarantine + replay** (`replay.rs`) — quarantined-event lifecycle.
4. **Inference** (`infer_*.rs`) — at least one happy-path test per format
   (csv/avro/proto/openapi/url) so format regressions surface.

## Out of scope (separate RFCs)

- Mutation testing on the validation engine (the patent core) — nice-to-have for
  defensibility; track separately.
- Load/perf regression gating in CI.

## Verification

`cargo test` and `cargo test -- --ignored` both green in CI; coverage step
produces a report and the ratchet gate is active.
