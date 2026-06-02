# RFC-070 — Org-scope tests for version-mutating storage fns

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Follows:** RFC-069 (pure-fn unit coverage), RFC-068 (org-isolation DB tests in CI)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 3 (storage org-scope)
**Severity:** P2 — mutating cross-tenant denial paths asserted in code, partially untested

---

## Problem

RFC-068's `two_org_get_version_isolation` proves org B is denied on
`get_version` and `promote_version`. But three other version-mutating storage
fns carry the **same** org guard (each calls `get_version(.., org_id)` first and
returns `VersionNotFound` on a wrong-org miss) and had no denial test:

- `patch_version_yaml`
- `deprecate_version`
- `delete_version`

These are the write-side BOLA surface: a regression that dropped the `org_id`
argument would let one tenant mutate or delete another tenant's version. Worth a
regression guard given multi-tenant enforcement is the product's core selling
point.

## Changes made

Test-only. No product code, schema, or config change. Extends the existing
`#[ignore]`d Class-1 DB test (already wired into CI by RFC-068 — no CI change
needed since it runs by name).

| File | Change |
|---|---|
| `src/tests.rs` — `org_scoping::two_org_get_version_isolation` | After the `promote_version` denial, add three more org-B denial assertions on `patch_version_yaml`, `deprecate_version`, `delete_version`, each expecting `VersionNotFound`. Reuses the already-seeded org A contract/version; no new setup or cleanup. |

## Why no new test / no CI change

The new assertions live inside an already-wired test that CI runs by exact name
(`tests::org_scoping::two_org_get_version_isolation`). Folding into it keeps the
CI step's `3 passed; 0 failed` count assertion valid and avoids a second DB
fixture. The version under test (`1.0.0`, created by `create_contract`) is in
`draft` state, so the `org_id` guard is reached **before** any state check in
all three fns — the denial is what's exercised, exactly as intended.

## Verification

```bash
DATABASE_URL=postgres://... \
cargo test --bin contractgate-server -- --ignored --exact \
  tests::org_scoping::two_org_get_version_isolation
cargo test   # unit suite, unchanged
```

## Follow-ups (tracked)

- Task 2 — coverage gate (`cargo-llvm-cov` ratchet-on-decrease).
- Quarantine/replay + inference happy-path coverage.
