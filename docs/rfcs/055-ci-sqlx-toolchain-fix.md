# RFC-055 — Fix CI sqlx-cli version drift and the stale migration sentinel

**Status:** Accepted  
**Date:** 2026-05-22  
**Branch:** nightly-maintenance-2026-05-23-rfc055  
**Addresses:** REVIEW-2026-05-22-launch-readiness H5, M7  
**Severity:** P1 — high (CI correctness)

---

## Problem

`.github/workflows/ci.yml`, `migrations-check` job:

1. **sqlx-cli version drift.** The step is labelled "Install sqlx-cli
   (matches sqlx 0.7 dep)" and runs `cargo install sqlx-cli --version
   0.7.4`. `Cargo.toml` is on `sqlx = "0.8"`. The `.sqlx/` offline-metadata
   format changed between 0.7 and 0.8, so `cargo sqlx prepare --check` is
   either failing the job outright or — worse — passing without actually
   verifying anything. Either way the guard that catches uncommitted query
   metadata drift is not working.
2. **Stale sentinel.** The job asserts only that migration 009's
   `github_integrations` table exists, with the message "Expected all 9
   migrations to apply cleanly". There are now 26 migrations. A failure in
   any of 010–026 is not caught by the sentinel.

---

## Fix

1. **Pin sqlx-cli to the crate version.** Install `sqlx-cli --version 0.8.x`
   (match the exact `sqlx` minor in `Cargo.lock`). Better: derive it so the
   two cannot drift again — e.g. `cargo install sqlx-cli --locked` with the
   version read from `Cargo.lock`, or a CI step that greps the lockfile.
2. **Verify the check actually runs.** After the fix, confirm `cargo sqlx
   prepare --check --workspace` exercises real metadata — intentionally break
   a query locally and confirm CI goes red.
3. **Update the sentinel.** Replace the migration-009 check with one that
   asserts the **latest** migration's sentinel object (currently migration
   026's `plan_tier` column / table). Update the message to the real count,
   or — more robust — assert a count of applied migration files matches a
   committed expected count so every new migration auto-updates the gate.
4. **Refresh comments.** The job header comment still says "all 9 migrations".

---

## Testing

- A deliberately-drifted query makes `migrations-check` fail.
- A deliberately-broken later migration fails the sentinel.
- A clean tree passes the whole job.

## What does NOT change

- The Postgres 16 service container and the apply-in-numeric-order loop.

## Rollout

CI-only change. Merge to `main`; confirm the next CI run is green for the
right reasons (not green because the check is inert). Independent — ship
standalone.
