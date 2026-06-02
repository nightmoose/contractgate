# RFC-003: Manual Replay Quarantine

| Status        | Accepted (2026-04-18)                    |
|---------------|------------------------------------------|
| Author        | ContractGate team                        |
| Created       | 2026-04-18                               |
| Accepted      | 2026-04-18 — Alex sign-off on Q1–Q4 (all defaults chosen) |
| Target branch | `nightly-maintenance-2026-04-18`         |
| Tracking      | Post-demo feedback item #3               |
| Depends on    | RFC-002 (versioning) — landed            |

## Summary

Add a **manual "Replay Quarantine"** action that takes a set of previously-
quarantined events, re-validates their payloads against a chosen contract
version, and lands the passing ones into `audit_log` + `forwarded_events`
the way an ordinary ingest call would. The original `quarantine_events`
rows are **preserved** (audit integrity), annotated with whether each
replay attempt succeeded, and linked back to whichever `audit_log` row the
payload ultimately landed in.

This is the post-RFC-002 payoff: now that every quarantined row carries
the `contract_version` it was rejected under, we can re-run it against any
*current* version the contract has and know what changed.

**Auto / scheduled replay is explicitly out of scope** — enterprise tier.
v1 is strictly operator-triggered (dashboard button + REST endpoint).

## Goals

1. A single `POST /contracts/:id/quarantine/replay` endpoint that accepts
   up to 1,000 quarantine row IDs and a target version.
2. **Preserve original quarantine rows.** A replay never deletes, mutates
   payload/violation data, or rewrites the original row's `contract_version`.
   Only the lifecycle columns (`status`, `replayed_at`, `replayed_into_audit_id`)
   are updated — and only on success.
3. **Honest audit trail.** Every replayed event generates a *new* row in
   either `audit_log` (pass) or `quarantine_events` (fail) with a
   `replay_of_quarantine_id` link back to its source. The version tagged on
   the new row is the version that actually matched, same rule as ingest.
4. **Target version is explicit.** Default is latest-stable; callers may
   pin any non-draft version via `target_version: "x.y.z"`. Draft target is
   allowed (useful for "does my in-progress schema accept these events yet?")
   but flagged in the response.
5. **Fallback-mode aware.** If the contract's `multi_stable_resolution =
   fallback`, replay behaves the same as ingest — first-pass-wins across
   stables, with the per-event `contract_version` reflecting whichever
   version accepted it.
6. **Idempotent.** Replaying an already-successfully-replayed row is a
   per-item no-op (`already_replayed` result), not a double-write.
7. **Dashboard UI**: Quarantine list page gains a checkbox column + bulk
   "Replay" button with a version picker (defaults to latest stable).

## Non-goals

- **Automatic / scheduled replay.** Enterprise feature, deferred.
- **Cross-contract replay.** Quarantine rows belong to one contract; you
  cannot replay a `contract_A` quarantine row against `contract_B`.
- **Payload editing at replay time.** The payload is replayed exactly as
  it arrived. If you need to fix the event, that's an out-of-band data
  repair — not a ContractGate concern.
- **Bulk replay across arbitrary filters.** v1 takes a hand-picked ID list
  (up to 1,000). The dashboard builds the list from filter selections; the
  API itself stays simple. "Replay every `pending` row from 2026-04-15"
  is a client-side composition, not a server query.
- **Partial-commit replay atomicity.** Replay is per-item like non-atomic
  batch ingest. An `atomic=true` flag is deferred (it would mean "roll
  back audit/forward writes if any one event fails", which is both rarely
  useful and expensive).
- **Replay of rows already in a terminal state.** `status = 'purged'` rows
  are intentionally unreachable — purging is how operators say "this is
  garbage, lose it." Attempting to replay a purged row returns `not_found`
  (we don't distinguish purged from never-existed).

## Current state

Migration `002_quarantine_and_p99.sql` defined `quarantine_events` with:
- `status TEXT CHECK (status IN ('pending', 'reviewed', 'replayed', 'purged'))`

The `replayed` state is reserved but unreached — no code path sets it
today. Good: we don't need to rename or broaden the enum.

Migration `003_contract_versioning.sql` added `contract_version` to
`quarantine_events` (and `audit_log`, `forwarded_events`). Every row
knows which version rejected it.

`src/storage.rs` already has a `quarantine_events_batch` helper taking
`Vec<QuarantineEventInsert>`, and `ingest.rs` has a clean
`validate_against_compiled(CompiledContract, payload) -> ValidationOutcome`
path the replay handler can reuse without going through HTTP.

What's missing:
- Schema: `replay_of_quarantine_id`, `replayed_at`, `replayed_into_audit_id`
  columns (and corresponding column on `audit_log` for the forward link).
- Storage: `list_quarantine_by_ids`, `mark_quarantine_replayed_batch`,
  `quarantine_events_batch` needs an optional `replay_of` per row, and a
  mirror field on `AuditEntryInsert`.
- Handler: `POST /contracts/:id/quarantine/replay`.
- Dashboard: checkbox column + Replay button + version picker on the
  audit/quarantine page.

## Design

### Schema migration (004_quarantine_replay.sql)

```sql
-- quarantine_events: mark when a row was replayed and where the result landed.
ALTER TABLE quarantine_events
    ADD COLUMN replayed_at           TIMESTAMPTZ,
    ADD COLUMN replayed_into_audit_id UUID REFERENCES audit_log(id),
    ADD COLUMN replay_of_quarantine_id UUID REFERENCES quarantine_events(id);

-- audit_log: link passing replays back to their source quarantine row.
ALTER TABLE audit_log
    ADD COLUMN replay_of_quarantine_id UUID REFERENCES quarantine_events(id);

-- Fast lookup: "show me the replay history of this quarantine row."
CREATE INDEX idx_quarantine_replay_of
    ON quarantine_events (replay_of_quarantine_id)
    WHERE replay_of_quarantine_id IS NOT NULL;

CREATE INDEX idx_audit_replay_of
    ON audit_log (replay_of_quarantine_id)
    WHERE replay_of_quarantine_id IS NOT NULL;
```

The `status` column and its existing CHECK constraint (`pending`,
`reviewed`, `replayed`, `purged`) stay as-is. `replayed` now has a
semantic: a replay attempt produced a passing audit_log row for this
quarantine event.

### State machine on the *source* quarantine row

```
                replay passes
pending ──────────────────────► replayed
  │ ▲
  │ │ replay fails (new quarantine row written, source untouched)
  ▼ │
pending (unchanged on failed replay)

pending ──► reviewed (manual)
pending ──► purged (manual, terminal)
replayed ──► purged (manual, terminal)
```

Notes:
- A failed replay does **not** touch the source row's status. It writes a
  *new* `quarantine_events` row with `replay_of_quarantine_id` set. The
  source row is still `pending` — operators can try again with a different
  target version.
- The source row transitions `pending → replayed` the first time *any*
  replay attempt of it succeeds. `replayed_at` is set to that attempt's
  timestamp. Subsequent replays of a `replayed` row short-circuit with
  `already_replayed` in the per-item response.
- `replayed_into_audit_id` is immutable once set — first successful
  replay wins.
- `reviewed → replayed` and `reviewed → pending` transitions are not
  permitted; once an operator marks a row `reviewed`, they've made a
  decision. (Open question: should `reviewed` rows be replayable? See
  **Open questions** below.)

### API surface

#### `POST /contracts/:id/quarantine/replay`

Request:
```json
{
  "ids": ["uuid1", "uuid2", "..."],
  "target_version": "2.1.0"   // optional; default = latest stable
}
```

- `ids`: 1..=1000 quarantine row IDs. All must belong to `:id`; rows from
  other contracts produce a per-item `wrong_contract` result.
- `target_version`: optional semver. Must exist on the contract. Draft
  versions are allowed (with a flag in the response).
- No `atomic` flag in v1.

Response (always `200` — per-item results; overall HTTP status is always
200 as long as the request parses, matching the non-atomic batch ingest
convention, since individual-item failures are expected):

```json
{
  "total": 3,
  "replayed": 1,
  "still_quarantined": 1,
  "already_replayed": 0,
  "not_found": 0,
  "wrong_contract": 0,
  "purged": 1,
  "target_version": "2.1.0",
  "target_version_source": "explicit",  // "explicit" | "default_stable"
  "target_is_draft": false,
  "results": [
    {
      "quarantine_id": "uuid1",
      "outcome": "replayed",
      "replayed_into_audit_id": "audit-uuid-1",
      "contract_version_matched": "2.1.0"
    },
    {
      "quarantine_id": "uuid2",
      "outcome": "still_quarantined",
      "new_quarantine_id": "quar-uuid-new",
      "contract_version_attempted": "2.1.0",
      "violation_count": 2,
      "violations": [ ... ]
    },
    {
      "quarantine_id": "uuid3",
      "outcome": "purged"
    }
  ]
}
```

Per-item outcomes:
- `replayed`: payload passed validation against target version; source row
  flipped to `status='replayed'`, `replayed_at=now()`, linked.
- `still_quarantined`: payload failed against target version; new
  quarantine row written with `replay_of_quarantine_id` set to the source.
- `already_replayed`: source row already has `status='replayed'`; no-op.
- `not_found`: no quarantine row with this ID exists for `:id`.
- `wrong_contract`: row exists but belongs to a different contract.
- `purged`: row exists but is `status='purged'`.

#### `GET /contracts/:id/quarantine/:quar_id/replay-history`

Returns the chain of replay attempts for a given quarantine row — the
source row plus every child (`replay_of_quarantine_id = :quar_id`) plus
the terminal `audit_log` row if one exists. Used by the dashboard drawer
that shows "replay attempts: 2 failed, 1 passed."

### Version resolution (mirrors ingest.rs::resolve_version)

Priority:
1. `target_version` in the request body, if present and exists on
   contract.
2. Latest stable (same logic as unpinned ingest).

If `multi_stable_resolution = fallback` on the contract AND no explicit
`target_version`, then under the hood the replay handler tries latest-
stable first and falls back across stables exactly like ingest — the per-
item `contract_version_matched` reflects the one that accepted the
payload.

If the contract has `multi_stable_resolution = fallback` AND the caller
supplies an explicit `target_version`, **the explicit version wins** and
fallback is *not* applied. This matches the ingest header-pin behavior.

### Handler flow (pseudocode)

```rust
async fn replay_handler(
    State(state): State<Arc<AppState>>,
    Path(contract_id): Path<Uuid>,
    Json(req): Json<ReplayRequest>,
) -> AppResult<Json<ReplayResponse>> {
    req.validate_bounds()?;  // 1..=1000 ids, all uuids

    // 1. Load all quarantine rows matching req.ids.  Bulk SELECT.
    let rows = storage::list_quarantine_by_ids(&state.db, &req.ids).await?;
    //    Categorize: not_found / wrong_contract / purged / already_replayed
    //    / eligible (status in (pending, reviewed)).

    // 2. Resolve target version.
    let (target_version, source, is_draft) = resolve_replay_target(
        &state, contract_id, req.target_version.as_deref()
    ).await?;  // default_stable | explicit

    // 3. For each eligible row, re-run validate_against_compiled.
    //    Fan out via spawn_blocking + rayon, same as ingest.
    //    Under fallback mode with no explicit target, retry across other
    //    stables on failure; per-event contract_version tracks match.
    let outcomes = validate_batch(&state, contract_id, &eligible, &target_version).await?;

    // 4. Bulk inserts: new audit_log rows for passes, new quarantine_events
    //    rows for failures; both carry replay_of_quarantine_id.
    let audit_inserts: Vec<AuditEntryInsert> = ...;
    let quar_inserts: Vec<QuarantineEventInsert> = ...;
    storage::log_audit_entries_batch(&state.db, audit_inserts).await?;
    storage::quarantine_events_batch(&state.db, quar_inserts).await?;

    // 5. For passes only: mark source rows replayed.
    storage::mark_quarantine_replayed_batch(&state.db, &pass_pairs).await?;
    //    pass_pairs: Vec<(source_id, new_audit_id)>

    Ok(Json(response))
}
```

### Dashboard changes

- Quarantine list page (`dashboard/app/audit/page.tsx` or a new
  `dashboard/app/quarantine/page.tsx`): add a checkbox column, a "Select
  all on page" checkbox, a "Replay selected" button, and a version picker
  dropdown (defaults to "Latest stable"). Also show `status` inline and a
  replay-count badge when `replay_of_quarantine_id` exists.
- Per-row drawer: add a "Replay history" tab showing the chain (fetches
  `/replay-history`).
- Confirm dialog before replay: "Replay N events against version X.
  Passes will be inserted into audit_log and forwarded. Originals will be
  preserved."

## Decisions (signed off 2026-04-18)

- **Q1 → A**: `reviewed` rows **are replayable**. `reviewed` is just
  triage state; replay stays available until `purged`.
- **Q2 → A**: Draft-version targets are **allowed and flagged**
  (`target_is_draft: true` in response). Useful for WIP sanity checks;
  the passing `audit_log` row is tagged with the draft version so the
  record is honest.
- **Q3 → A**: Replay-passes **fire the contract's forward destination**
  exactly like fresh events. Matches the "fix the downstream gap" use
  case — a replay is the mechanism for recovering events that should have
  been forwarded under a fixed schema.
- **Q4 → A**: Replay cap is **1,000 rows per request**, same as batch
  ingest. Operator-triggered action; global Axum timeout catches outliers.

Original options retained below for posterity.

### Q1. Can `reviewed` quarantine rows be replayed?

The `status` enum has `pending → reviewed` as a "someone looked at it,
made a call" transition. Two options:

- **A (default, recommended)**: `reviewed` rows are still replayable.
  Rationale: "reviewed" just means the row has been triaged; replay is a
  separate action and should always be allowed until `purged`.
- **B**: `reviewed` rows short-circuit with a `not_eligible` outcome —
  operator has to explicitly move them back to `pending` first.
  Rationale: forces a second confirmation for review work already done.

### Q2. Draft-version replay — allowed, disallowed, or flagged?

- **A (default, recommended)**: Allowed, flagged in response
  (`target_is_draft: true`). Useful for "does my WIP schema actually
  accept these?" — lets operators sanity-check a draft before promoting.
  A passing replay still writes to `audit_log` tagged with the draft
  version, which is honest but unusual.
- **B**: Disallowed — draft target returns 409. Rationale: `audit_log`
  should only reference promoted contracts; drafts churn and a replayed
  event tagged with a since-edited draft confuses the record.
- **C**: Allowed but **dry-run only** — response reports per-item outcome
  but no rows are inserted anywhere. A separate `dry_run=true` flag on
  non-draft replays stays useful too.

### Q3. Forward destination on replay-passes

When a replay passes and we write to `audit_log`, do we also fire the
contract's forward destination (same as ingest does for fresh events)?

- **A (default, recommended)**: Yes, forward replayed passes. The event
  *did* pass a contract; downstream systems missing the data because of a
  past schema bug is exactly the scenario replay exists to fix.
- **B**: No, replay writes only to `audit_log` and stops there. Operators
  who want to replay *into* downstream must export and re-ingest through
  `/ingest/...`. Safer default (no accidental double-writes if downstream
  already got a patched version via some other path).

This one genuinely depends on how Alex thinks about downstream idempotency.

### Q4. Rate / concurrency limits on the replay endpoint

Replay can touch up to 1,000 rows per call. Do we need special limits?

- **A (default, recommended)**: Reuse the existing batch ingest limits —
  1,000 rows/request, global Axum timeout, no per-endpoint rate limit.
  Replay is operator-triggered so abuse is limited by dashboard clicks.
- **B**: Cap replay at 100 rows/request until we have metrics. Rationale:
  replay writes roughly 2× the rows of ingest (new audit/quar + source
  updates) and we haven't profiled it.

## Test plan

### Unit (no DB)
1. `ReplayRequest::validate_bounds` — 0 ids → 400, >1000 ids → 400.
2. `resolve_replay_target` — explicit version present, explicit version
   absent-from-contract, default-to-latest-stable, no stable exists.
3. Outcome categorization given mixed source rows (pending / reviewed /
   purged / not-found / wrong-contract / already-replayed).
4. Response shaping: counts match the sum of per-item outcomes.

### DB-backed integration (deferred with RFC-002's harness)
5. Replay a `pending` row → `replayed` status, `replayed_at` set,
   `replayed_into_audit_id` matches new row.
6. Source row's `contract_version` is NOT overwritten by replay.
7. New audit_log row's `contract_version` is the target version.
8. Failed replay → new quarantine row, source stays `pending`,
   `replayed_at` remains NULL.
9. `already_replayed` is a no-op — no new rows written, no status change.
10. `wrong_contract` — row from contract B submitted to contract A's
    replay endpoint → flagged, no writes.
11. `purged` row → flagged, no writes.
12. Fallback mode without explicit target: replay against latest stable
    fails, retries across other stables, `contract_version_matched` ==
    whichever stable accepted.
13. Fallback mode with explicit target: only that target is tried, no
    fan-out.
14. Draft target (if Q2=A): allowed, `target_is_draft=true`, new audit
    row tagged with draft version.
15. 1,000-row replay completes in <1s on a typical dataset.
16. Two concurrent replays of the same row (race): exactly one succeeds
    with `replayed`, the other returns `already_replayed` (guarded by a
    DB-level conditional update in `mark_quarantine_replayed_batch`).
17. Forwarding side-effect on replay-pass (Q3=A): forward record created.
    (Q3=B): no forward record.

### Dashboard (Playwright / manual)
18. Bulk-select 3 rows, pick "Latest stable", replay — UI updates to show
    1 replayed / 2 still-quarantined.
19. Replay history drawer shows the child chain.
20. Version picker defaults to "Latest stable" and lists stables first,
    drafts last (visually distinct).

## Rollout

1. Write migration `004_quarantine_replay.sql`. Additive only (ALTER
   ADD COLUMN is safe on the non-empty tables from RFC-002 onward).
2. Extend `QuarantineEventInsert`, `AuditEntryInsert`, and the matching
   batch helpers to accept `replay_of_quarantine_id: Option<Uuid>`.
3. Add `list_quarantine_by_ids`, `mark_quarantine_replayed_batch`, and
   `replay_history_for` to storage.
4. Add `ReplayRequest` / `ReplayResponse` types + handler in a new
   `src/replay.rs` module (keeps ingest.rs from growing further).
5. Wire route on main.rs.
6. Ship unit tests (§1-4 of test plan).
7. Dashboard: checkbox column + bulk replay button + version picker +
   replay-history drawer.
8. Update MAINTENANCE_LOG.
9. DB integration tests ride in with the RFC-002 harness effort.

## Decisions locked in before implementation

- Q1, Q2, Q3, Q4 answers (above) must be signed off.
- Any new migration column additions (none expected beyond the three
  listed above).

Once Alex greenlights, start with the migration and work top-down through
the rollout list.
