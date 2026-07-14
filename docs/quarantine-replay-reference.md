# Quarantine + Replay API Reference

**Last updated:** 2026-07-14 (RFC-081)

Events that fail contract validation at ingest are written to the **quarantine**
store instead of being forwarded. This API lists them, replays them against a
target contract version, and reports per-attempt history. It backs the dashboard
Quarantine tab.

All routes are **org-scoped**: results are limited to contracts owned by the
caller's org (resolved from the API key or Bearer JWT — see
[`auth-reference.md`](./auth-reference.md)). Ids belonging to another org are
treated as not-found and never surfaced. In production a request with no
resolvable org returns **401**.

There is also a per-contract variant of replay
(`POST /contracts/{id}/quarantine/replay`, `GET /contracts/{id}/quarantine/{quar_id}/replay-history`)
retained from RFC-003; the routes below are the org-wide equivalents and share
the same replay engine.

---

## `GET /quarantine`

List source quarantine rows for the caller's org, newest first.

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
  resolved target version.
- `version` (optional): pin a target version. If omitted, each contract's latest
  stable is used. Draft targets are allowed.
- `contract_id` (optional): assertion — if present, every `event_id` must belong
  to this contract, else **400**.

Response:

```json
{
  "replayed": 1,
  "outcomes": [
    { "event_id": "uuid", "version": "2.0.0", "passed": true,  "violations": [],       "replayed_at": "..." },
    { "event_id": "uuid", "version": "2.0.0", "passed": false, "violations": [ ... ],  "replayed_at": "..." }
  ]
}
```

`replayed` is the count of events that passed on this attempt. `outcomes` has one
entry per input `event_id`, in input order. A passing event lands in the audit
log and fires the contract's forward destination (same as fresh ingest); a
failing event writes a new quarantine row linked to the source, and the source is
left untouched. Ids that are not found, already replayed, purged, or belong to
another org come back with `passed: false` and no violations.

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
