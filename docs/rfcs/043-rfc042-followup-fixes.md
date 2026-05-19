# RFC-043: RFC-042 Follow-up Fixes + P0-3 Loose Ends

**Status:** Accepted
**Branch:** `nightly-maintenance-2026-05-17-rfc043`
**Fixes:** Findings from 2026-05-17 review of RFC-039 / RFC-040 / RFC-041 / RFC-042 on `main`.

## Context

A review of `main` after the P0 bundle (RFC-039 / 040 / 041) and the P1 bundle (RFC-042) shipped surfaced three real bugs and five smaller correctness/doc issues. The shipped functionality is sound — this RFC just closes the gaps.

The fixes are bundled because they all live in the same surface area (auth + rate limit + api-key docs) and are individually small.

---

## Fix 1 — JWT rate-limit bucket must be keyed by `user_id`, not nil UUID

### Problem

`src/main.rs:765-774` calls `state.rate_limiter.check(validated.api_key_id, ...)` for JWT sessions.  `verify_supabase_jwt` sets `api_key_id = Uuid::nil()` as the "JWT session" sentinel.  Result: **every dashboard user shares one global bucket** keyed by the nil UUID.  Two active dashboard users can throttle each other; one runaway user 429s everyone.

### Fix

Key the JWT bucket by `validated.user_id` instead.  `ValidatedKey.user_id` is the real Supabase user UUID and is already populated by `jwt_auth.rs`.

```rust
// src/main.rs, JWT branch in require_api_key
let outcome = state.rate_limiter.check(
    validated.user_id,       // was: validated.api_key_id (nil)
    Some(500),
    Some(2_000),
);
```

No change to `RateLimitState::check` signature — it already takes a `Uuid`.

### Tests

Add to `src/rate_limit.rs`:
- `two_distinct_uuids_have_independent_buckets` — exhaust one, confirm the other still allows.

### Audit-log compatibility

`api_key_id = Uuid::nil()` remains the "this row came from a JWT session" sentinel.  Only the rate-limit lookup key changes, not the `ValidatedKey` struct.

---

## Fix 2 — Carve out `/contracts/infer/*` from the 1 MB body limit

### Problem

`src/main.rs:1082-1086` applies `RequestBodyLimitLayer::new(1_048_576)` to the entire `protected` router.  The inference endpoints accept user-supplied schemas:

- `/contracts/infer/openapi` — real OpenAPI specs routinely exceed 1 MB (Stripe's is ~6 MB).
- `/contracts/infer/proto` — .proto bundles for large services.
- `/contracts/infer/csv` — sample CSVs.
- `/contracts/infer/avro` — Avro schemas (smaller, but still bursty).

RFC-042 promised a carve-out; it was never implemented.  Real users will hit 413 on first non-toy schema.

### Fix

Split `protected` into two sub-routers and merge them, applying body limits per group:

```rust
let infer = Router::new()
    .route("/contracts/infer", post(infer::infer_handler))
    .route("/contracts/infer/avro",    post(infer_avro::infer_avro_handler))
    .route("/contracts/infer/proto",   post(infer_proto::infer_proto_handler))
    .route("/contracts/infer/openapi", post(infer_openapi::infer_openapi_handler))
    .route("/contracts/infer/csv",     post(infer_csv::infer_csv_handler))
    .route("/contracts/infer/url",     post(infer_url::infer_url_handler))
    .layer(tower_http::limit::RequestBodyLimitLayer::new(10 * 1024 * 1024)); // 10 MB

let protected = Router::new()
    // ... all current protected routes EXCEPT /contracts/infer/*
    .layer(tower_http::limit::RequestBodyLimitLayer::new(1024 * 1024));      // 1 MB

let protected = protected.merge(infer)
    .layer(middleware::from_fn_with_state(state.clone(), require_api_key));
```

The auth middleware is applied **after** the merge so both groups share the same auth, but each has its own body-size cap.

10 MB matches the existing `v1_ingest` cap — same order of magnitude, same memory budget.

### Tests

Add an integration test that POSTs a 2 MB payload to `/contracts/infer/openapi` and asserts `!= 413`.  Add another that POSTs a 2 MB payload to `/contracts` and asserts `== 413`.

---

## Fix 3 — `test_live.sh` playground call now 401s

### Problem

`test_live.sh:135` calls `POST /playground/validate` with no auth header.  RFC-042 moved this route from `public` to `protected`, so the smoke test now fails on step 8.

### Fix

Add the `x-api-key` header to the curl command, sourcing from `${API_KEY}` (already required by the rest of the script implicitly).  Add an early guard:

```bash
: "${API_KEY:?API_KEY env var required since RFC-042 moved playground to protected}"
```

Then update the call:

```bash
PG=$(curl -sf -X POST "$BASE/playground/validate" \
  -H "Content-Type: application/json" \
  -H "x-api-key: $API_KEY" \
  -d "...")
```

Also audit the rest of the script — any other previously-public endpoints affected.  At time of writing only `/playground/validate` moved.

### CI

If `test_live.sh` runs in CI, ensure `API_KEY` is set in the workflow env.

---

## Fix 4 — Stale "bcrypt" comments in migration 006

### Problem

RFC-041 / migration 024 overrides the column comment via DDL, but the file-level `--` comments in `supabase/migrations/006_accounts_and_api_keys.sql` still mislead readers:

- Line 10: `bcrypt hash (key_hash) and an 8-char prefix...`
- Line 116: `bcrypt hash of the full raw key...`
- Line 131: `Only the bcrypt hash and 8-char prefix are stored...`
- Line 138-139: comment-on-column block still has the old wording in the file source (the DDL is overridden by 024 at runtime, but the file source is what future engineers read).

Migration files are immutable history; we don't rewrite them.  But the file-level `--` comments are misleading.

### Fix

Two options:

1. **(Recommended)** Leave migration 006 alone (immutable history) and rely on migration 024's `COMMENT ON COLUMN` for the source of truth.  Add a single-line header at the top of 024 explicitly stating: *"This supersedes the bcrypt wording in migration 006 — that file is kept intact as history."*
2. (Rejected) Edit migration 006 in place — violates the "migrations are immutable" convention and risks drift between deployed DBs and the file.

Implement option 1.

---

## Fix 5 — Migration 024 `CHECK` should be `NOT VALID` then validated

### Problem

`supabase/migrations/024_api_key_hash_algorithm_docs.sql:17-19` adds:

```sql
alter table public.api_keys
    add constraint api_keys_key_hash_length
        check (length(key_hash) = 44);
```

Without `NOT VALID`, Postgres takes an `ACCESS EXCLUSIVE` lock on `api_keys` and validates every existing row inline.  On a quiet table this is fine; on a busy one it blocks all reads/writes for the duration.  More importantly, **any pre-existing row with length != 44 fails the migration outright**, requiring data fix-up under emergency.

### Fix

Two-step pattern that's safe under load and lets bad rows be reported instead of aborting the deploy:

```sql
-- Step 1: add as NOT VALID — locks only briefly to add the constraint
alter table public.api_keys
    add constraint api_keys_key_hash_length
        check (length(key_hash) = 44) not valid;

-- Step 2: validate — uses a SHARE UPDATE EXCLUSIVE lock, allows reads/writes
alter table public.api_keys
    validate constraint api_keys_key_hash_length;
```

If step 2 errors, the DBA can `SELECT id, length(key_hash) FROM api_keys WHERE length(key_hash) != 44` to find the bad rows, fix them, and re-run validate.  The constraint still rejects future bad inserts because of step 1.

### Migration number

This patches an already-deployed migration.  Create migration **025** with the corrected pattern, after dropping the original constraint if it exists:

```sql
alter table public.api_keys
    drop constraint if exists api_keys_key_hash_length;

-- then the NOT VALID + VALIDATE pair above
```

Idempotent — safe to apply whether or not 024 succeeded.

---

## Fix 6 — Entropy mismatch: RFC says 224 bits, code makes 192 bits

### Problem

`docs/rfcs/041-api-key-hash-algorithm-docs.md:27` says the raw key is `cg_live_<28-byte hex>` = 224 bits.  `dashboard/app/account/page.tsx:42` actually generates **24** bytes = 192 bits.  Comment at `page.tsx:50` claims 224.

192 bits is still wildly more than enough — SHA-256 of any value > ~80 bits is unguessable for the lifetime of the universe.  But the doc/code disagreement is a trip-wire for the next person reading the audit.

### Fix

Cheapest fix: align the doc to the code.

- Edit `dashboard/app/account/page.tsx:50` comment: `192 bits` not `224 bits`.
- Edit `docs/rfcs/041-api-key-hash-algorithm-docs.md:27`: change `28-byte hex` to `24-byte hex`, recompute prefix length notes if any.

We deliberately do **not** bump the random byte count — existing deployed keys are 24-byte and the column would need to accommodate both.  Documentation is cheaper than a migration.

---

## Fix 7 — `.env.example` missing `SUPABASE_URL`

### Problem

RFC-039 promised "no new env vars required."  Commit `518da8d` (`fix: handle Supabase pooler DATABASE_URL`) and the post-hoc edits added a `SUPABASE_URL` env var fallback in `src/main.rs:1175-1180` for Fly deployments where pooler URL parsing doesn't yield a project ref.  `.env.example` was never updated, so anyone copying it for fresh Fly deploys is one debug session away from "JWKS not loaded — Bearer JWT auth disabled" mystery.

### Fix

Append to `.env.example`:

```
# Optional: explicit Supabase project URL for JWKS lookup.
# Set this if DATABASE_URL is a pooler URL (aws-0-*.pooler.supabase.com)
# and JWKS auto-derivation fails at startup.
# Example: SUPABASE_URL=https://abcdefghijklmnopqrst.supabase.co
# SUPABASE_URL=
```

---

## Fix 8 — Confirm `cargo check && cargo test` clean after RFC-042

### Problem

RFC-042 merged without an explicit cargo run logged.  This RFC's reviewer (Claude) cannot run cargo per project rules.

### Fix

Before merging this RFC: run `cargo check` and `cargo test` on a clean tree.  Fix anything that breaks; report back if anything looks unrelated to this RFC.

---

## Rollout order

The fixes are independent.  Suggested commit order on `nightly-maintenance-2026-05-17-rfc043`:

1. Fix 5 (migration 025) — DB change, deploy independently first.
2. Fix 1 (JWT bucket) — single-line bug.
3. Fix 2 (infer carve-out) — modest router refactor.
4. Fix 3 (test_live.sh) — script + CI env.
5. Fixes 4, 6, 7 — docs only.
6. Fix 8 — pre-merge gate.

Each fix is small enough to land as its own commit.  Bundle 4/6/7 if convenient.

---

## Files touched

- `src/main.rs` — fixes 1, 2
- `src/rate_limit.rs` — fix 1 (new test only)
- `supabase/migrations/025_api_key_hash_length_check_safe.sql` — fix 5 (new file)
- `supabase/migrations/024_api_key_hash_algorithm_docs.sql` — fix 4 (header note added)
- `test_live.sh` — fix 3
- `dashboard/app/account/page.tsx` — fix 6 (comment only)
- `docs/rfcs/041-api-key-hash-algorithm-docs.md` — fix 6 (24-byte not 28-byte)
- `.env.example` — fix 7
- This RFC document.

## What does NOT change

- `ValidatedKey` struct shape.
- `verify_supabase_jwt` (still returns `api_key_id = Uuid::nil()` for JWT sessions).
- `api_key_auth.rs` (SHA-256/base64 verify is already correct).
- The 10 MB cap on `/v1/ingest/{id}`.
- Migration 023 (RLS) and 024 (column comment + constraint intent).
