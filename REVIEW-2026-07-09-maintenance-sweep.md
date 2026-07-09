# REVIEW 2026-07-09 — Maintenance Sweep

Scope: full repo scan (src/, dashboard/, docs/, supabase/) + live Supabase advisors + prod migration table. No code changed. Branch at time of review: `RFC-080_GrokPass`.

---

## P0 — fix before public signup

### 1. Prod migration drift is real and unresolved
Prod `supabase_migrations` tracks only **5** entries:
`003_contract_versioning, 004_quarantine_replay, 005_pii_transforms, create_early_access, 029_stripe_failed_events`

Repo has files `001–026`. Two-way drift:
- Migrations 001–002, 006–026 applied to prod but **untracked** (or never applied — needs schema diff to know which).
- `029_stripe_failed_events` (and presumably 027–028) exist in prod but have **no file in supabase/migrations/**.

This is the exact failure mode behind the 2026-06-05 silent Stripe webhook 23514. Action: dump prod schema, diff against concatenated migration files, commit missing 027–029 files, backfill the migration table so CI file-count checks mean something.

> **CORRECTION (same day):** the review above ran against a stale checkout
> (`RFC-080_GrokPass` / stale local main). origin/main already had files
> 027–029 (027 = RFC-056 api_keys server-side issuance, 028 = Stripe billing
> incl. `orgs.plan_status`, 029 = stripe_failed_events). Only `early_access`
> truly lacked a file (now `030_early_access.sql`).
>
> **RESOLVED 2026-07-09:** prod was missing 024, 025, and 027 — all three
> applied and verified. Ledger backfilled for 001–023, 026, 028
> (see scripts/backfill_schema_migrations.sql). Prod and repo now match
> through 030. Remaining lesson: branch from origin/main, and the CI
> file-count drift check should compare against the prod ledger.

### 2. Supabase security advisors — 4 ERRORs + serious WARNs
- **ERROR — SECURITY DEFINER views**: `v_ingestion_summary`, `provider_scorecard`, `provider_field_health`, `active_contracts_public`. These bypass the querying user's RLS via PostgREST. Scorecard views summarize `audit_log` cross-org → any authenticated user can read other orgs' quality data. Switch to `security_invoker = true` (PG15+) or gate behind backend-only access.
- **WARN — `provider_field_baseline` policy `auth_all` is `USING (true) WITH CHECK (true)` for ALL/authenticated**: any signed-in user can read/write every org's baselines. Contradicts RFC-074 data-plane isolation. Scope with `get_my_org_ids()` per the RLS helper rule.
- **WARN — anon can execute SECURITY DEFINER fns** `handle_new_user()`, `rls_auto_enable()`, `get_my_org_ids()` via `/rest/v1/rpc/`. Revoke EXECUTE from `anon` (and `authenticated` for the trigger fns).
- **WARN — leaked password protection disabled** in Auth. One toggle.
- INFO — RLS enabled, no policies: `idempotency_keys`, `public_contracts`, `stripe_failed_events`, `stripe_processed_events`. Fine if intentionally service-role-only; confirm and note in migration comments.
- WARN — 8 functions with mutable `search_path` (trigger/guard fns). One migration: `ALTER FUNCTION ... SET search_path = public`.

### 3. Stripe webhook not in this repo
Prod has `stripe_failed_events` / `stripe_processed_events`, but no Stripe handler exists in src/ or dashboard/app/api/. The "fix silent-200 webhook" follow-up can't be verified from here — whichever repo hosts it, it's still on the pre-signup checklist.

---

## P1 — tech debt / simplify

### 4. Oversized files (split candidates)
| File | Lines | Suggested split |
|---|---|---|
| `src/storage.rs` | 2,766 (53 fns) | `storage/` module: contracts, versions, audit, keys, scorecard |
| `dashboard/app/contracts/page.tsx` | 2,097 | more into `_tabs/` (pattern already exists) |
| `dashboard/app/workbench/WorkbenchClient.tsx` | 1,454 | per-panel components |
| `src/main.rs` | 1,354 | extract router builder + auth middleware into `router.rs` / `auth_layer.rs` |
| `dashboard/lib/api.ts` | 1,349 | split by domain (contracts, scorecard, catalog, auth) |

Mechanical refactors, zero behavior change, each one PR-sized.

### 5. Dual ingest paths
`/ingest/{raw_id}` vs `/v1/ingest/{contract_id}` — divergence is well documented in `v1_ingest.rs`, but legacy `/ingest` lacks idempotency, rate limiting, body caps, and `quarantine_id`. Decide: deprecate `/ingest` with a sunset header, or backport the guards. Carrying both indefinitely doubles every ingest-path change.

### 6. Kafka/Kinesis duplication (unverified candidate)
`kafka_consumer/kafka_ingress` and `kinesis_consumer/kinesis_ingress` (~2,500 lines combined) likely share validate→audit→forward plumbing. Candidate for a shared `stream_ingress` core trait.

---

## P2 — hygiene

### 7. RFC bookkeeping is stale
- `docs/rfcs/` stops at **058**, but RFCs through 075 shipped and the branch is `RFC-080_GrokPass`. Files 059+ were never committed (or live on other branches).
- 15+ RFC files still say **Draft** for work that shipped (049–055 closed in the launch-readiness pass; 034/035/046 shipped per maintenance log). Batch status update.

### 8. Root clutter
`REVIEW-*.md`, `BROWSER_TEST_REPORT-*.md`, `Grok_Suggestions.txt` (→ `docs/`), `mri_*.yaml` (→ `contracts/`), `ContractGate_Benefits_Findigs_v3.pdf` (typo'd name, → `docs/` or delete).

---

## What's in good shape (no action)
- No `TODO`/`FIXME`/`unimplemented!` in production Rust code; all `unwrap()`s confined to `#[cfg(test)]`.
- Dashboard TS hygiene: zero `: any`, one stray `console.log`.
- Module-level docs in `ingest.rs`/`v1_ingest.rs` are excellent.
- Audit-honesty invariant (contract_version = matched version) explicitly documented in code.

## Suggested order
1. Migration reconciliation (#1) — everything else DB-side builds on it
2. Security-advisor migration bundle (#2) — one migration + one Auth toggle
3. RFC status batch-fix + commit missing RFC files (#7) — 30 min
4. storage.rs split (#4) — first refactor PR
5. Ingest-path decision (#5) — needs your call: deprecate or backport
