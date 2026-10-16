# Event Payload Storage — Reference

**RFC:** 086
**Status:** Backend implemented (dashboard + consumer-path gating are follow-ups)
**Added:** nightly-maintenance-2026-07-20-rfc086-gated-payload-storage

---

## What this controls

DataContractGate records an audit row for **every** ingested event and a quarantine
row for every **failed** event. By default those rows now store only *metadata*
— contract, version, pass/fail, violations, counts, source IP, timing — and
**not** the event body. Storing the body (`audit_log.raw_event`,
`quarantine_events.payload`, both post-transform per RFC-004) is opt-in and
paid-only.

The body is required for exactly one feature: **quarantine replay**. Without a
stored body, a quarantine row is non-replayable (it still shows what failed and
why).

Passing events also insert a `forwarded_events` row. That row keeps
`contract_id`, `contract_version`, and `created_at` either way. The body is
written only under the policy above. When storage is off, `payload` is JSON
`null` (the column is `NOT NULL`, so the event is omitted without a schema
change). Existing rows are not rewritten. Kafka and Kinesis publish the body
from memory and do not write this table. A reader must treat JSON `null` as
"body not stored"; a passing event is a JSON object, so `null` is not a
captured body. Paid plans can pull these rows with
[`GET /v1/contracts/{id}/passed`](passed-events-reference.md). That response
uses the same `null` payload.

## Policy

Bodies are stored only when **all** of:

1. the org is on a paid plan (`growth`, `aisg_pro`, or `enterprise`), **and**
2. the org master switch `orgs.store_event_payloads` is on, **and**
3. the per-contract override `contracts.store_event_payloads` is on (default
   true; only consulted when the org switch is on).

Self-hosted / dev deployments (no org context) always store. On a lookup error
or missing org row, ingest **redacts** (fail-closed — never store what we're not
paid/permitted to hold).

When a body is not stored, the row is written with an empty/NULL body and a
redaction marker (`raw_event_redacted` / `payload_redacted`). `GET /quarantine`
returns `payload_redacted: true` and `raw_event: null` for such rows.

`GET /contracts` and `GET /contracts/{id}` report `store_event_payloads` as the
**effective** value, not the raw column (neither column is rewritten). It is
the same decision ingest uses: paid plan **and** `orgs.store_event_payloads`
**and** the per-contract override. It is `false` for `free`, `aisg_free`, and
unknown plans, and also `false` on a paid plan (`growth`, `aisg_pro`,
`enterprise`) while the org master switch is off (that column defaults to
`false`). A paid org with the switch on reports the per-contract override.
Both products share these responses. `POST /contracts` and `PATCH /contracts/{id}`
return the same effective field on `ContractResponse`.

## Turning it off purges history

- **Org switch on → off:** all stored bodies for the org are purged.
- **Per-contract override on → off:** write-forward only — new events stop
  storing bodies; existing bodies are **not** purged.
- **Purge body history (per contract):** redacts stored bodies on demand,
  independent of the toggles.

Purge redacts **in place**: it nulls the body column and sets the redaction
marker. It never deletes an `audit_log` or `quarantine_events` row — all rows and
metadata are retained.

## API

All routes are org-scoped (owner/admin; production requires a resolvable org).

### `GET /settings/payload-storage`

```json
{ "enabled": false, "plan": "growth", "eligible": true }
```

`enabled` = org master switch. `eligible` = plan is paid (switch is meaningful).

### `PUT /settings/payload-storage`

Body `{ "enabled": true }`. Enabling requires a paid plan. Otherwise the
response is `400` and the message names every plan the paid-plan check accepts
(`growth`, `aisg_pro`, `enterprise` — DataContractGate Pro, AISchemaGate Pro,
and Enterprise). That list is the same slice the check uses, so a new paid
plan shows up in the message without a separate copy. Setting `false` purges
all stored bodies for the org. Returns the same shape as GET.

### `PATCH /contracts/{id}/payload-storage`

Body `{ "enabled": false }`. Per-contract override. Write-forward only — does
not purge history. `404` if the contract isn't owned by the caller's org.

### `POST /contracts/{id}/purge-bodies`

Redacts stored bodies for one contract, retaining all audit/quarantine rows.

```json
{ "quarantine_bodies_redacted": 12, "audit_bodies_redacted": 480 }
```

## Setting a plan

Plan changes are made in Supabase until a self-serve flow exists:

```sql
UPDATE orgs SET plan = 'growth' WHERE id = 'your-org-id';
```

## Related

- [Passed events](passed-events-reference.md) — cursor page of forwarded (passing) events.
- [RFC-086](rfcs/086-gated-event-payload-storage.md) — design + implementation notes.
- [Quarantine + Replay reference](quarantine-replay-reference.md) — replay
  requires a stored body; redacted rows are non-replayable.
- [Plan gating reference](plan-gating-reference.md) — tier feature matrix.
