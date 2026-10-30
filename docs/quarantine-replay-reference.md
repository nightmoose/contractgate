# Quarantine + Replay API Reference

**Last updated:** 2026-10-16

Events that fail contract validation at ingest are written to the **quarantine**
store instead of being forwarded. This API lists them, replays them against a
target contract version, and reports per-attempt history. It backs the dashboard
Quarantine tab.

All routes are **org-scoped**: results are limited to contracts owned by the
caller's org (resolved from the API key or Bearer JWT — see
[`auth-reference.md`](./auth-reference.md)). Ids belonging to another org are
treated as not-found and never surfaced. In production a request with no
resolvable org returns **401**.

Replay and bulk replay are included with Pro. Free keeps the failure details. See [`quarantine-plan-reference.md`](./quarantine-plan-reference.md).

There is also a per-contract variant of replay
(`POST /contracts/{id}/quarantine/replay`, `GET /contracts/{id}/quarantine/{quar_id}/replay-history`)
retained from RFC-003; the routes below are the org-wide equivalents and share
the same replay engine.

---

## `GET /quarantine`

List source quarantine rows for the caller's org, newest first. Rows whose
contract has been soft-deleted are not listed. `DELETE /contracts/{id}`
removes that contract's quarantine rows and `forwarded_events` rows in the
same transaction as the soft delete (audit rows and contract versions are
kept; a published version is not unpublished). `forwarded_events` is the
passed-event export copy, not the audit record. Soft delete does not cascade,
so those rows are deleted explicitly.

Query parameters:

| Param | Type | Default | Notes |
|---|---|---|---|
| `contract_id` | uuid | — | Restrict to one contract. Omit for all of the org's contracts. |
| `limit` | int | 100 | Clamped to 1–500. |
| `offset` | int | 0 | For pagination. |

Only **source** rows are returned (the failed-replay children of a replay
attempt are excluded; they show up under replay-history). Response is an array of:

```json
{
  "id": "uuid",
  "contract_id": "uuid",
  "contract_version": "1.0.0",
  "raw_event": { "...": "the stored (post-transform) payload" },
  "violation_details": [ { "field": "...", "rule": "...", "message": "..." } ],
  "violation_count": 1,
  "source_ip": "203.0.113.10",
  "quarantined_at": "2026-07-14T12:00:00Z",
  "replay_count": 0,
  "last_replayed_at": null,
  "last_replay_passed": null
}
```

`replay_count` / `last_replayed_at` / `last_replay_passed` summarize replay
attempts against the row: count of attempts, the most-recent attempt time, and
whether that most-recent attempt passed (`true`), failed (`false`), or there were
none (`null`).

---

## `POST /quarantine/replay`

Re-validate quarantined events against a target contract version.

Request body:

```json
{
  "event_ids": ["uuid", "uuid"],
  "version": "2.0.0",
  "contract_id": "uuid"
}
```

- `event_ids` (required): 1–1000 quarantine row ids. May span multiple contracts;
  they are grouped by contract and each group is replayed against its own
  resolved target version. An empty list is **400**
  `Invalid request: replay request must include at least one id`. More than
  1000 ids is **400**
  `Invalid request: replay request exceeds cap of 1000 ids (got N)`.
  `POST /contracts/{id}/quarantine/replay` uses the same two messages for `ids`.
- `version` (optional): pin a target version. If omitted, each contract's latest
  stable is used. Draft targets are allowed.
- `contract_id` (optional): assertion — if present, every `event_id` must belong
  to this contract, else **400**.

Response:

```json
{
  "replayed": 1,
  "outcomes": [
    { "event_id": "uuid", "version": "2.0.0", "passed": true,  "violations": [],       "replayed_at": "...", "redacted": false },
    { "event_id": "uuid", "version": "2.0.0", "passed": false, "violations": [ ... ],  "replayed_at": "...", "redacted": false },
    { "event_id": "uuid", "version": "2.0.0", "passed": false, "violations": [],       "replayed_at": "...", "redacted": true }
  ]
}
```

`replayed` is the count of events that passed on this attempt. `outcomes` has one
entry per input `event_id`, in input order. A passing event lands in the audit
log and fires the contract's forward destination (same as fresh ingest); a
failing event writes a new quarantine row linked to the source, and the source is
left untouched. Ids that are not found, already replayed, purged, or belong to
another org come back with `passed: false`, `redacted: false`, and no violations.

`redacted: true` means the source body was never stored or was purged, so the
row was not re-validated. `passed` is false and `violations` is empty; this is
not a validation failure. It is the same classification the per-contract route
reports as `"outcome": "redacted"`. Both routes use one helper
(`replay_item_view`) over the shared replay engine. DataContractGate and
AISchemaGate share these routes.

`contract_id` set to a contract that does not own every id is **400**
`Invalid request: event_ids span multiple contracts or a contract other than contract_id`.

### `POST /contracts/{id}/quarantine/replay`

Same engine. Body is `{ids, target_version?}`. **200** is one object, not the
org-wide `{replayed, outcomes}` shape:

```json
{
  "total": 1,
  "replayed": 0,
  "still_quarantined": 0,
  "already_replayed": 0,
  "not_found": 0,
  "wrong_contract": 0,
  "purged": 0,
  "redacted": 1,
  "target_version": "1.0.0",
  "target_version_source": "default_stable",
  "target_is_draft": false,
  "results": [{ "quarantine_id": "uuid", "outcome": "redacted" }]
}
```

`results[].outcome` is `replayed`, `still_quarantined`, `already_replayed`,
`not_found`, `wrong_contract`, `purged`, or `redacted`. `replayed` adds
`replayed_into_audit_id` and `contract_version_matched`. `still_quarantined`
adds `new_quarantine_id`, `contract_version_attempted`, `violation_count`, and
`violations`. The id-list **400** messages are the same as the org-wide route.
**404** when the contract or an explicit target version does not exist.

---

## `GET /quarantine/replay-history`

Attempt history for a single quarantined event.

| Param | Type | Default | Notes |
|---|---|---|---|
| `event_id` | uuid | — | The source quarantine row id. |
| `limit` | int | 100 | Clamped to 1–500. |

Returns an array of `ReplayOutcome` (same shape as `outcomes` above), one per
replay attempt (passes and fails), newest first. An unknown or cross-org
`event_id` returns an empty array.

---

## Notes

- Replay is idempotent on success: once a source row is stamped `replayed`, a
  second replay of the same id is a no-op (`passed: false`, reported as already
  replayed). A race between two concurrent replays of the same id resolves to
  exactly one winner.
- The stored `raw_event` is already in post-transform form (PII masking from
  RFC-004 is applied at ingest and carried forward on replay).
- **RFC-086:** replay requires a stored body. If event-payload storage was off
  for the contract (Free plan, org/contract opted out, or the body was purged),
  the quarantine row is returned with `payload_redacted: true` and `raw_event:
  null`, and replay reports it as `redacted` (non-replayable) rather than
  re-validating an empty body. See
  [event-payload-storage-reference.md](event-payload-storage-reference.md).
  A passing replay's audit body follows that same storage rule. A failed
  replay's new quarantine row keeps the source body so it can be replayed
  again.
- The passing `audit_log` row is inserted before the replay response returns,
  with `org_id` set to the caller's org. `GET /audit` lists
  `WHERE org_id = $caller`, so that stamp is what makes the new passing row
  appear. `forwarded_events` is inserted on a background task after the
  response (a sink error must not fail the replay); the insert is not skipped.
