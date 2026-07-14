# Worklist 2026-07-14 — for Sonnet — RFC-081 quarantine endpoints

> **STATUS: IMPLEMENTED 2026-07-14** on branch `nightly-maintenance-2026-07-14-rfc081`
> (all backend steps done; `cargo check/clippy/test` green in-session; DB-backed
> isolation test added to CI). Kept for reference / review. The only thing not
> runnable in-session was the `#[ignore]` DB-backed test — verify it in CI.

Prepared from a session review that found the dashboard Quarantine tab is wired
to backend routes that don't exist (no list endpoint; path + payload mismatch on
replay). Full rationale + design: [`docs/rfcs/081-quarantine-list-and-replay-reconciliation.md`](../rfcs/081-quarantine-list-and-replay-reconciliation.md).
One branch/PR.

## Ground rules (CLAUDE.md + session-learned)

- Branch: `nightly-maintenance-2026-07-14-rfc081`. **Fetch first and branch from `origin/main`** — local main has been stale before.
- Run `cargo check && cargo test && cargo clippy --all-targets -- -D warnings` before declaring done. (Cargo now runs in-session with `CARGO_TARGET_DIR=/tmp/cgtarget`; the `#[ignore]` DB-backed tests still need a live Postgres — run them in the compose/CI lane, not the check sandbox.)
- **No migration.** All fields already exist on `quarantine_events` (schema 002 + 003 `contract_version` + 004 replay columns). `replay_count`/`last_replayed_at`/`last_replay_passed` are DERIVED, not columns. So NO `EXPECTED_MIGRATION_COUNT` bump.
- Org RLS via `public.get_my_org_ids()` only. New queries are org-scoped by JOINing `quarantine_events → contracts` and filtering `contracts.org_id = caller_org` (same authority as `get_contract_identity`). Cross-org ids are skipped/rejected, never leaked. Follow RFC-074: thread `org_id` through storage.
- Do NOT touch `validation.rs` or the per-contract replay engine's behavior — this is additive + a pure extract-refactor.
- Update `docs/` + append `MAINTENANCE_LOG.md`.

## The contract to match (already shipped in `dashboard/lib/api.ts`)

Match these EXACT field names (serde) — the frontend types are the source of truth:

- `GET /quarantine?contract_id=&limit=&offset=` → `QuarantinedEvent[]`
  `{ id, contract_id, contract_version, raw_event, violation_details, violation_count, source_ip, quarantined_at, replay_count, last_replayed_at, last_replay_passed }`
- `POST /quarantine/replay` body `{ event_ids: string[], version?: string, contract_id?: string }` → `{ replayed: number, outcomes: ReplayOutcome[] }`
- `GET /quarantine/replay-history?event_id=&limit=` → `ReplayOutcome[]`
  `ReplayOutcome = { event_id, version, passed, violations, replayed_at }`

## Steps

1. **Extract engine (pure refactor).** In `src/replay.rs`, move the body of
   `replay_handler` into `async fn replay_for_contract(state, org_id,
   contract_id, ids, target_version) -> AppResult<ReplayResponse>`. `replay_handler`
   becomes a thin caller. **No behavior change** — existing
   `POST /contracts/{id}/quarantine/replay` and its tests must stay green.

2. **New `src/quarantine.rs`.** `list_quarantine_handler` (org-scoped) +
   `QuarantinedEventOut` DTO with the field names above. Lists **source** rows
   only (`replay_of_quarantine_id IS NULL`), newest first, `limit` default 100 /
   cap 500, `offset` default 0, optional `contract_id` filter.

3. **Storage (`src/storage/replay.rs`).** Add
   `list_quarantine_events(pool, org_id, contract_id, limit, offset)` — the
   org-joined SELECT with a `LEFT JOIN LATERAL` over the union of attempt rows
   (`audit_log` + child `quarantine_events` where `replay_of_quarantine_id = q.id`)
   to compute `replay_count` / `last_replayed_at` / `last_replay_passed` in one
   query (no N+1). `last_replay_passed` = true if the most-recent attempt is an
   audit_log (pass) row, false if a child quarantine (fail) row, NULL if none.

4. **Top-level handlers (`src/replay.rs`).**
   - `replay_all_handler`: load `event_ids` org-scoped, group by their
     `contract_id` (reject ids from a different contract than a supplied
     `contract_id` with 400), call `replay_for_contract` per group with
     `target_version = version`, flatten to `ReplayOutcome[]`, set
     `replayed = count(passed)`. Map non-eligible outcomes (not-found/
     already-replayed/purged/wrong-contract) to `passed=false`, empty violations.
   - `replay_history_all_handler`: org-scoped wrapper over `replay_history_for`,
     reshaped to `ReplayOutcome[]`.

5. **Routes (`src/main.rs`).** Register on the **protected** (org-scoped) router:
   `GET /quarantine`, `POST /quarantine/replay`, `GET /quarantine/replay-history`.
   Keep the contracts-scoped routes.

6. **Frontend.** Verify `dashboard/lib/api.ts` matches (it already targets these
   shapes). Adjust only if a field name drifts. `npm run build` in `dashboard/`.

7. **Docs.** New `docs/quarantine-replay-reference.md` (endpoints, bodies,
   outcome semantics, org scoping). Append `MAINTENANCE_LOG.md`. Flip RFC-081
   Status → Shipped + STATUS.md row on merge.

## Tests (add to the `migrations-check` guarded named-test list)

Seed two orgs, one contract each with quarantined rows. Assert:
- `GET /quarantine` returns only the caller's org rows; `contract_id` filter narrows.
- A cross-org `event_id` in a replay body is rejected/skipped (never replayed, never leaked).
- `replayed` / `outcomes` counts match a known pass/fail fixture.
- `replay_count` increments after a failed replay; `last_replay_passed` reflects the latest attempt.
- Serde round-trip: DTO field names equal the `api.ts` names.

Bump the `migrations-check` named-test count/sentinel for the new tests (this is the test-list count, not the migration count).

## Acceptance

Dashboard Quarantine tab loads real events and replays them end-to-end against a
running gateway; cross-org isolation test is green in the auth-on lane
(`tests/compose_isolation_smoke.sh` idiom); `cargo check/test/clippy` clean;
contracts-scoped routes + their tests unchanged.
