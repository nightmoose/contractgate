# RFC-069 — Unit coverage for untested pure functions

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Follows:** RFC-068 (org-isolation DB tests in CI)
**Addresses:** docs/reviews/test-hardening-handoff-2026-05-28.md — Task 3 (partial; auth)
**Severity:** P2 — correctness regressions in these functions fail silently

---

## Problem

Task 3 of the test-hardening handoff asks for coverage on critical paths, auth
first. Most auth logic is DB- or network-backed and belongs in the Class-1/2 DB
lanes (RFC-068, compose-smoke). But three **pure** functions carry real
regression risk and have **zero** tests today — they run in `cargo test` with no
DB, so there is no reason to leave them uncovered:

1. **`jwt_auth::jwks_url_from_database_url`** — derives the Supabase JWKS URL
   from `DATABASE_URL`. It hand-parses two distinct connection-string formats
   (direct `db.<ref>.supabase.co` and pooler `aws-0-<region>.pooler.supabase.com`
   with the project ref in the username) and returns `None` on anything else. A
   silent regression here breaks **all** dashboard JWT auth, because the server
   would fetch the wrong JWKS (or none) at startup. This is the single
   highest-value untested pure function in the codebase.
2. **`storage::PublicationRow::is_revoked`** — gates whether a published
   contract is still served. Trivial today, but untested.
3. **`jwt_auth::JwtAuthError` `Display`** — these strings land in logs/401
   diagnostics; lock the wording so a careless edit doesn't silently change
   operator-facing output.

Out of scope: `PublicationRow::visibility_parsed` is thin sugar over
`PublicationVisibility::FromStr`, which is **already** tested
(`src/tests.rs` `visibility_parsed`/round-trip cases) — no marginal value.

## Changes made

Test-only. No product code, schema, or config change.

| File | Change |
|---|---|
| `src/jwt_auth.rs` (tests mod) | Add `jwks_url_*` cases: direct host, direct host without `db.` prefix, pooler with `postgres.<ref>` user, non-Supabase host → `None`, malformed (no `@`) → `None`, pooler with empty ref → `None`. Add `JwtAuthError` `Display` assertions for each variant. |
| `src/storage.rs` (new tests mod) | Add `is_revoked` true/false cases over a `PublicationRow`. |

## Why these inputs

`jwks_url_from_database_url` branches on host suffix and, for the pooler case,
re-parses the **username** for the project ref. The cases pin every branch:
the two success shapes, the `db.`-prefix strip, and three independent `None`
exits (wrong suffix, no `@`, empty ref). That is the full decision surface.

## Rollout / migration

None. Adds tests only. `cargo test` (unit) gains the new cases and stays green.

## Verification

```bash
cargo test --bin contractgate-server jwt_auth::tests
cargo test --bin contractgate-server storage::
cargo test            # full unit suite, unchanged behavior
```

## Follow-ups (tracked, not in this RFC)

- Task 2 — coverage gate (`cargo-llvm-cov`, ratchet-on-decrease).
- Task 3 — storage org-scope guard tests (`Some(other_org)` denial paths):
  DB-backed, belong in the Class-1 ignored lane wired by RFC-068.
- Quarantine/replay + inference happy-path coverage.
