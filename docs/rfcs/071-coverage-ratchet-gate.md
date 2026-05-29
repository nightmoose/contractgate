# RFC-071 — Coverage ratchet gate

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Follows:** RFC-070 (version-mutation org-scope tests)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 2
**Severity:** P3 — process/quality; no runtime impact

---

## Problem

Task 2 of the handoff: add coverage reporting to CI with a **low, non-blocking**
gate so coverage stops backsliding without an upfront test-writing push. The
handoff explicitly recommends a ratchet-style gate (fail on *decrease*) over a
fixed high bar.

## Decision

- **Tool:** `cargo-llvm-cov` — the handoff's pick; the standard LLVM
  source-based coverage front-end for Rust. Needs only the
  `llvm-tools-preview` rustup component.
- **Scope:** unit tests only (no `DATABASE_URL`, no service container). The
  global `SQLX_OFFLINE=true` already makes the unit suite DB-free; measuring
  just that keeps the number **deterministic** across runs. DB-backed
  (`#[ignore]`d) tests run elsewhere (RFC-068) and would make coverage vary by
  whether Postgres was up.
- **Gate style:** ratchet against a committed baseline
  (`coverage-baseline.txt`). Fails only when line coverage drops more than
  `COVERAGE_TOLERANCE` (default 0.5pp) below the baseline. A small tolerance
  absorbs measurement jitter from non-deterministic codegen without letting
  real regressions through.
- **Blocking:** the `coverage` job is **not** in `deploy-fly`'s `needs`. A
  regression turns the job red on the PR (visible, blocks merge if branch
  protection requires it) but does not block the main-branch deploy while the
  gate beds in. Promote it into `needs` once the baseline is trusted.

## Reading the number (note for reviewers)

The seeded baseline is **low on purpose** — expect roughly 30% line coverage,
not 80%. This gate measures **only the deterministic, DB-free unit suite**, and
ContractGate's largest modules are DB-bound (`storage.rs`, `ingest.rs`,
`main.rs`, the CLI and scaffold trees) — their real coverage comes from the
`#[ignore]`d integration tests that need a live Postgres (RFC-068), which this
job deliberately does **not** run so the number stays stable across CI runs. So
the figure is **not** a quality verdict on the codebase; it is a stable
reference point the ratchet defends against backsliding. Judge a PR by whether
it moves the number *down*, not by its absolute value.

The authoritative value lives in `coverage-baseline.txt` (the file the gate
reads), not in this document — intentionally, so it can't go stale as the
ratchet bumps it.

## How the baseline is seeded

The gate logic lives in `scripts/check-coverage.sh` (kept out of `ci.yml` so it
is runnable/inspectable locally). On a run where `coverage-baseline.txt` is
**absent**, the script measures, writes the file, and passes — so the gate is
non-blocking to land. The CI run's seeded value is uploaded as the
`coverage-summary` artifact.

**Action for the maintainer:** because I cannot run cargo here, the baseline is
not committed. Seed it once locally and commit:

```bash
cargo install cargo-llvm-cov --locked   # one-time
bash scripts/check-coverage.sh          # writes coverage-baseline.txt
git add coverage-baseline.txt && git commit -m "RFC-071: seed coverage baseline"
```

Until the file is committed, every CI run re-seeds from the ephemeral checkout
(always green) — correct but inert. Committing the file activates the ratchet.

## Ratchet-up behaviour

When a run measures coverage *above* the baseline, the script rewrites
`coverage-baseline.txt` to the higher number and emits a `::notice::`. In CI
that rewrite is ephemeral (not pushed); committing the bumped file in a PR locks
the gain in. This keeps the baseline monotonically rising as tests are added.

## Changes made

| File | Change |
|---|---|
| `scripts/check-coverage.sh` | New. Measures `cargo llvm-cov --summary-only`, parses the TOTAL line %, compares to baseline with tolerance, auto-seeds when absent, ratchets up on improvement. Exit 0 pass/seed, 1 regression, 2 tooling/parse error. |
| `.github/workflows/ci.yml` | New `coverage` job: installs `llvm-tools-preview` + `cargo-llvm-cov`, runs the script, uploads the baseline as an artifact. Not in `deploy-fly` needs. |

No product code, schema, or migration change.

## Verification

```bash
cargo install cargo-llvm-cov --locked
bash scripts/check-coverage.sh           # first run seeds + passes
bash scripts/check-coverage.sh           # second run compares against seed → passes
```

To prove the gate bites, temporarily set a high baseline and confirm exit 1:

```bash
echo 99.9 > coverage-baseline.txt
bash scripts/check-coverage.sh; echo "exit=$?"   # expect regression + exit 1
git checkout -- coverage-baseline.txt 2>/dev/null || rm coverage-baseline.txt
```

## Follow-ups (tracked)

- Promote `coverage` into `deploy-fly`'s `needs` once the baseline is trusted.
- Optionally publish to Codecov for trend graphs (artifact suffices for now).
- Remaining Task 3: quarantine/replay + inference happy-path coverage.
