# RFC-081 — Quarantine list endpoint + top-level replay reconciliation

**Status:** Draft (implementation landed 2026-07-14, pending merge)
**Date:** 2026-07-14
**Branch:** nightly-maintenance-2026-07-14-rfc081
**Depends on:** RFC-003 (manual replay), RFC-047 (org scoping), RFC-074 (data-plane org ownership)

---

## Problem

The dashboard Quarantine tab is wired to backend endpoints that **do not exist**,
so quarantine review + replay — a headline feature for both the customer and the
acquisition story — is unreachable over HTTP.

`dashboard/lib/api.ts` (called live from `dashboard/app/contracts/_tabs/quarantine.tsx`)
calls:

| Frontend call | Body / query | Backend reality |
|---|---|---|
| `GET /quarantine?contract_id&limit&offset` | — | **no such route** (no list endpoint exists at all) |
| `POST /quarantine/replay` | `{event_ids, version?, contract_id?}` | only `POST /contracts/{id}/quarantine/replay` with `{ids, target_version}` |
| `GET /quarantine/replay-history?event_id&limit` | — | only `GET /contracts/{id}/quarantine/{quar_id}/replay-history` |

Two independent mismatches:

1. **No list endpoint.** There is no HTTP way to enumerate `quarantine_events`
   ids, so even the correct contracts-scoped replay route can't be driven by a
   client — you'd need direct DB access to get ids.
2. **Path + payload shape mismatch.** The frontend expects org-wide (optionally
   contract-filtered) routes and a flat `ReplayResponse { replayed, outcomes[] }`;
   the backend exposes per-contract routes and a detailed counts-based
   `ReplayResponse`. The two were never compatible — the tab has never worked
   against this backend.

Verified against `src/main.rs` route table, handlers, router `.merge`/`.nest`
structure, and `dashboard/next.config` (no rewrites).

## Goal

Make the existing dashboard Quarantine tab work **without rewriting the tested
replay engine**, by adding org-scoped top-level endpoints that match the client
already shipped in `api.ts`.

## Non-goals

- Rewriting `replay::replay_handler` (the per-contract engine, its version
  resolution, and the RFC-072 race guard stay untouched).
- Changing the `quarantine_events` schema (all needed fields already exist;
  `replay_count` / `last_replayed_at` / `last_replay_passed` are **derived**).
- Deprecating the contracts-scoped routes (kept; the new routes wrap the same
  logic).

## Design

Three new **org-scoped** routes on the protected router. Org scope is enforced
by joining `quarantine_events → contracts` and filtering `contracts.org_id =
caller_org` (same authority as `get_contract_identity`). In production
(`auth_configured()`), a `None` org → 401; cross-org ids are silently skipped
(treated as not-found), never leaked.

### 1. `GET /quarantine?contract_id=&limit=&offset=` → `QuarantinedEvent[]`

Lists **source** quarantine rows (`replay_of_quarantine_id IS NULL`) for the
caller's org, newest first, optional `contract_id` filter, `limit` capped at 500
(default 100), `offset` default 0. Failed-replay child rows are excluded from the
top-level list (they surface under replay-history).

Response row shape (matches `api.ts::QuarantinedEvent`):

```
id, contract_id, contract_version, raw_event (= payload),
violation_details, violation_count, source_ip, quarantined_at (= created_at),
replay_count, last_replayed_at, last_replay_passed
```

Derived fields, per source row `q`:

- `replay_count` = number of replay attempts against `q` = `count(audit_log where
  replay_of_quarantine_id = q.id)` + `count(quarantine_events where
  replay_of_quarantine_id = q.id)`.
- `last_replayed_at` = max timestamp across those attempt rows (NULL if none).
- `last_replay_passed` = whether the most-recent attempt was a pass (an
  `audit_log` row) vs fail (a child `quarantine_events` row); NULL if none.

Computed with a single `LEFT JOIN LATERAL` over the union of attempt rows so the
list stays one query (no N+1).

### 2. `POST /quarantine/replay` `{event_ids, version?, contract_id?}` → `{replayed, outcomes[]}`

Because the engine resolves **one target version per contract**, the handler:

1. Loads the given `event_ids` (org-scoped); groups them by their own
   `contract_id`. `contract_id` in the body, if present, is an assertion — ids
   from other contracts are rejected (400) to avoid silent surprises.
2. For each contract group, calls the existing per-contract replay path
   (`resolve_replay_target` + the validate/stamp pipeline) with
   `target_version = version`.
3. Flattens per-event results into `ReplayOutcome { event_id, version, passed,
   violations, replayed_at }` and sets `replayed = count(passed)`.

`replayed_at` for a passing event is the stamp time; for a still-quarantined
(failed) event it is the attempt time. Non-eligible outcomes (not-found,
already-replayed, purged, wrong-contract) map to `passed=false` with an empty
`violations` list and are documented in the reference.

Refactor: extract the current `replay_handler` body into
`replay_for_contract(state, org_id, contract_id, ids, target_version) ->
ReplayResponse` (internal). Both the existing `POST /contracts/{id}/quarantine/replay`
and the new top-level handler call it; the top-level handler adapts the detailed
per-contract `ReplayResponse` into the flat `{replayed, outcomes[]}` the frontend
expects. Zero behavior change to the contracts-scoped route.

### 3. `GET /quarantine/replay-history?event_id=&limit=` → `ReplayOutcome[]`

Org-scoped wrapper over the existing `replay_history_for(pool, quar_id)`,
reshaped to `ReplayOutcome[]` (newest first, `limit` default 100 / cap 500).

## Files

- `src/replay.rs` — extract `replay_for_contract(...)`; add
  `replay_all_handler` (top-level) + response adapter; add
  `replay_history_all_handler`.
- `src/quarantine.rs` (**new**) — `list_quarantine_handler` + the
  `QuarantinedEventOut` DTO.
- `src/storage/replay.rs` — add `list_quarantine_events(pool, org_id,
  contract_id, limit, offset)` with the derived-field lateral join; add
  `replay_attempts_for(pool, quar_id)` if not already covered by
  `replay_history_for`.
- `src/main.rs` — register the three routes on the **protected** (org-scoped)
  router; keep the contracts-scoped routes.
- `dashboard/lib/api.ts` — verify the client matches (it already targets these
  shapes; adjust only if a field name drifts). No new frontend logic.
- `docs/quarantine-replay-reference.md` (**new**) — user-facing endpoint doc.

## Testing

- Unit: DTO serialization matches `api.ts` field names (serde rename check).
- DB-backed (`#[ignore]`, `migrations-check` lane): seed two orgs, one contract
  each with quarantined rows; assert (a) `GET /quarantine` returns only the
  caller's org rows, (b) `contract_id` filter narrows correctly, (c) a cross-org
  `event_id` in a replay body is rejected/skipped, (d) `replayed`/`outcomes`
  counts match a known pass/fail fixture, (e) `replay_count` increments after a
  failed replay.
- Add the new named tests to the `migrations-check` guarded list and bump its
  expected-count sentinel.
- `cargo check && cargo test && cargo clippy --all-targets -- -D warnings`.

## Rollout / risk

- No migration; additive routes only. Contracts-scoped routes unchanged →
  existing tests stay green.
- Org isolation is the sensitive surface: every new query is org-joined, and the
  DB-backed cross-org test is the gate. Follows RFC-074's "thread org_id through
  storage" rule; RLS helper `get_my_org_ids()` unaffected (service-role path).
- Validation hot path (`validation.rs`) untouched — no p99 impact.

## Open question

Should the top-level list include `status='purged'` / already-replayed source
rows, or only actionable (`pending`/`reviewed`, not-yet-passed) ones? Default:
include all source rows with their derived replay state so the UI can show
history; the tab can filter client-side. Revisit if the list grows unwieldy.
