# Worklist 2026-07-10 — for Sonnet

Prepared by Fable from the 2026-07-09 maintenance sweep ([REVIEW-2026-07-09-maintenance-sweep.md](../../REVIEW-2026-07-09-maintenance-sweep.md)) after the migration-drift reconciliation merged. Three independent items, in priority order. Do them as **separate branches/PRs**, one at a time.

## Ground rules (repeat of CLAUDE.md, plus session-learned facts)

- Branch per item: `nightly-maintenance-$(date +%Y-%m-%d)-<slug>`. **Fetch first and branch from `origin/main`** — local main has been stale before and it burned us.
- Run `cargo check && cargo test && cargo clippy --all-targets -- -D warnings` before declaring done. CI also runs cargo-deny and cargo-audit.
- Never break existing behavior. Validation engine stays <15ms p99 (item 3 must be a pure move-refactor).
- Org RLS must go through `public.get_my_org_ids()` — inline subqueries on `org_memberships` cause PG 42P17 recursion.
- **Do NOT apply anything to the prod Supabase project.** Write migration files only; Alex applies them. Prod is currently reconciled through migration 030; the ledger (`supabase_migrations.schema_migrations`) matches `supabase/migrations/*.sql`.
- CI has a migration sentinel job: adding migration file N requires bumping `EXPECTED_MIGRATION_COUNT` in `.github/workflows/ci.yml` **and** adding a "Sentinel" assertion for the new migration (see Sentinels A–A5 in the `migrations-check` job for the pattern).
- Update `docs/` for any user-facing change; append a MAINTENANCE_LOG.md entry per item.

---

## Item 1 — Migration 031: security-advisor fixes (P0)

Supabase advisors (fetched live 2026-07-09) flag the following. One migration file, `supabase/migrations/031_security_advisor_fixes.sql`, fixing all of it:

1. **SECURITY DEFINER views (4, ERROR level):** `v_ingestion_summary`, `provider_scorecard`, `provider_field_health`, `active_contracts_public`. They bypass the querying user's RLS via PostgREST. Fix: `ALTER VIEW ... SET (security_invoker = true);` (Postgres 17 in prod, supported). **Check first** how each view is consumed: `provider_scorecard`/`provider_field_health` are read by the Rust backend via the service-role connection (src/scorecard.rs) — service role bypasses RLS, so security_invoker is safe there. Verify `active_contracts_public` (public catalog) still works for anon reads after the change — it may need an explicit RLS-friendly path or a grant instead; reason it through and document the choice in the migration comments.
2. **`provider_field_baseline` policy `auth_all` is `USING (true) WITH CHECK (true)` for ALL/authenticated** (from migration 019): any signed-in user can read/write every org's baselines. The table has no org_id column — it's keyed by `source`. Decide: either (a) drop the `authenticated` policy entirely (backend writes via service role; dashboard doesn't query it directly — verify with grep), or (b) add org scoping. (a) is likely correct and simpler.
3. **Anon-executable SECURITY DEFINER functions:** `handle_new_user()`, `rls_auto_enable()`, `get_my_org_ids()` are callable via `/rest/v1/rpc/` by `anon` (and the two trigger fns by `authenticated`). Fix: `REVOKE EXECUTE ... FROM anon, authenticated;` for `handle_new_user` and `rls_auto_enable` (trigger-only). For `get_my_org_ids`, revoke from `anon` only — **it must stay executable by `authenticated`** because RLS policies evaluate it as the calling user.
4. **8 functions with mutable search_path** (WARN): `contract_versions_immutability_guard`, `contract_versions_delete_guard`, `contracts_name_history_trigger`, `quarantine_replay_stamp_guard`, `contract_versions_compliance_mode_guard`, `set_updated_at`, `guard_api_key_hash_immutable`, `update_updated_at`. Fix: `ALTER FUNCTION ... SET search_path = public;` each.
5. **RLS-enabled-no-policy tables (INFO, intentional):** `idempotency_keys`, `public_contracts`, `stripe_processed_events`, `stripe_failed_events` are service-role-only by design. Don't add policies; add a `COMMENT ON TABLE` to each stating this is deliberate, so future advisor runs are self-explaining.

Also: CI count 30 → 31 + new sentinel (suggest: assert `provider_scorecard` has `security_invoker=true` via `pg_views`/`reloptions`).

Out of scope: "leaked password protection" toggle — that's an Auth dashboard setting, not SQL. Tell Alex to flip it.

**Acceptance:** migration applies cleanly on a fresh Postgres in CI; all sentinels pass; a note lists which advisor lints the migration clears.

## Item 2 — CI drift check against the prod ledger (P1)

Today Sentinel B only counts files (`EXPECTED_MIGRATION_COUNT`). That never catches "file exists but prod never applied it" — the exact failure mode of the 2026-06-05 Stripe webhook incident.

Add a scheduled workflow (e.g. `.github/workflows/migration-drift.yml`, daily cron + manual dispatch — **not** in the PR path, since it needs prod access and PRs from forks must not get the secret):

- Query prod: `SELECT name FROM supabase_migrations.schema_migrations` via `psql` using a new `PROD_DATABASE_URL` **read-only** secret (Alex to create the secret and ideally a read-only DB role; document the needed grant in the workflow comments).
- Compare against `ls supabase/migrations/*.sql` basenames. Known ledger aliases (hard-code an alias map in the script): `create_early_access` ↔ `030_early_access.sql`.
- Fail (and thus notify) if a file has no ledger row or a ledger row has no file.
- Keep the existing PR-time Sentinel B as-is (fast, no secrets).

**Acceptance:** workflow runs green against current prod state; deliberately removing a ledger name in a local dry-run of the compare script makes it fail; README-level docs in `docs/` (new `docs/migration-drift-check-reference.md`, since a config/ops surface is user-facing per CLAUDE.md).

## Item 3 — Split `src/storage.rs` (P1, pure refactor)

2,766 lines / 53 fns. Convert to a `storage/` module directory, grouped by domain:

- `storage/mod.rs` — shared pool/types + `pub use` re-exports so **no call sites change**
- `storage/contracts.rs` — contract + contract_versions CRUD
- `storage/audit.rs` — audit_log + quarantine + replay writes
- `storage/keys.rs` — api_keys lookups/validation
- `storage/ingress.rs` — kafka/kinesis ingress config
- `storage/misc.rs` — whatever doesn't fit (publication, collaboration, catalog), split further only if a group exceeds ~600 lines

Constraints: **move-only** — no signature changes, no query changes, no behavior changes. sqlx query metadata is cached in `.sqlx/`; a pure move doesn't change query hashes, so `cargo sqlx prepare` should NOT need re-running — if `cargo sqlx prepare --check` fails, something changed that shouldn't have. Keep the audit-honesty invariant comments (contract_version = version that actually matched) attached to the functions they document.

**Acceptance:** `cargo check`, `cargo test` (103+ tests), clippy clean, `cargo sqlx prepare --check` passes, zero diff in behavior, `git log --follow` friendly (prefer `git mv` then edit).

---

Item order matters: 1 is pre-public-signup security, 2 prevents the next drift incident, 3 is debt. Stop and ask Alex rather than guessing if anything conflicts with what you find in the tree — origin/main may have moved past this document.
