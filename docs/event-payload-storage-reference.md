# Event Payload Storage — Reference

**RFC:** 086
**Status:** Backend implemented (dashboard + consumer-path gating are follow-ups)
**Added:** nightly-maintenance-2026-07-20-rfc086-gated-payload-storage

---

## What this controls

ContractGate records an audit row for **every** ingested event and a quarantine
row for every **failed** event. By default those rows now store only *metadata*
— contract, version, pass/fail, violations, counts, source IP, timing — and
**not** the event body. Storing the body (`audit_log.raw_event`,
`quarantine_events.payload`, both post-transform per RFC-004) is opt-in and
paid-only.

The body is required for exactly one feature: **quarantine replay**. Without a
stored body, a quarantine row is non-replayable (it still shows what failed and
why).

## Policy

Bodies are stored only when **all** of:

1. the org is on a paid plan (`growth` or `enterprise`), **and**
2. the org master switch `orgs.store_event_payloads` is on, **and**
3. the per-contract override `contracts.store_event_payloads` is on (default
   true; only consulted when the org switch is on).

Self-hosted / dev deployments (no org context) always store. On a lookup error
or missing org row, ingest **redacts** (fail-closed — never store what we're not
paid/permitted to hold).

When a body is not stored, the row is written with an empty/NULL body and a
redaction marker (`raw_event_redacted` / `payload_redacted`). `GET /quarantine`
returns `payload_redacted: true` and `raw_event: null` for such rows.

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

Body `{ "enabled": true }`. Enabling requires a paid plan (`400` otherwise).
Setting `false` purges all stored bodies for the org. Returns the same shape as
GET.

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

- [RFC-086](rfcs/086-gated-event-payload-storage.md) — design + implementation notes.
- [Quarantine + Replay reference](quarantine-replay-reference.md) — replay
  requires a stored body; redacted rows are non-replayable.
- [Plan gating reference](plan-gating-reference.md) — tier feature matrix.
