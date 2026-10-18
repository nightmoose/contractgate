# Passed events — reference

`GET /v1/contracts/{id}/passed` returns events that **passed** validation for
one contract, oldest first. Rows come from `forwarded_events`. A Fivetran
Connector SDK connector uses the cursor to pull only passing records
incrementally.

Auth is the same `x-api-key` as the rest of `/v1`. The contract must belong
to the key's org. A key with a contract allow-list must include this
contract (otherwise 401).

## Plans

DataContractGate and AISchemaGate share this gate. The 403 message names
the same paid plans the check allows.

| Plan | Values | This endpoint |
|---|---|---|
| Free | `free`, `aisg_free` | 403 |
| Pro | `growth`, `aisg_pro` | allowed |
| Enterprise | `enterprise` | allowed |

```json
{ "error": "Forbidden: passed-event export is a paid-plan feature; upgrade to growth, aisg_pro, or enterprise", "status": 403 }
```

Self-hosted / dev (auth off, no org) is not plan-gated.

## Request

```
GET /v1/contracts/{id}/passed?cursor=&limit=
X-Api-Key: cg_live_...
```

| Query | Default | Rule |
|---|---|---|
| `cursor` | absent | Opaque cursor from `next_cursor`. Send it back unchanged. |
| `limit` | 500 | Max 1000. Larger integers are capped. `0`, negatives, and non-integers (`-1`, `abc`) are 400. |

404 when the contract does not exist, is owned by another org, or is
soft-deleted. Those three cases look the same. Deleting a contract removes
its `forwarded_events` rows in that same transaction. The audit log is kept.

The per-key token bucket already on `/v1` applies. A full bucket is 429
with `X-RateLimit-*` and `Retry-After`.

## Cursor

Order is `created_at`, then `id`, ascending. Ids are random UUIDs, so they
are only a tie-break.

The cursor is opaque base64url (no padding) of
`{created_at as microseconds},{id}`. Do not parse it. It is safe in a query
string.

Rows newer than **5 seconds** are left out, so an insert still in flight is
not skipped. Events show up about 5 seconds late.

| Cursor | Status | What the connector does |
|---|---|---|
| Missing or blank | 200 | First page, oldest row still stored. |
| Not decodable | 400 | The message contains `invalid cursor`. Drop the cursor and re-sync (call with no cursor). This is the only 400 that means the cursor is bad. |
| Other 400 (`limit`) | 400 | Keep the cursor. The message does not contain `invalid cursor`. |
| Older than retention | 410 | Drop the cursor and re-sync. |

Retention is the plan's log window (migration 037): **90 days** on Pro
(`growth`, `aisg_pro`), **365 days** on Enterprise. `CONTRACTGATE_SELF_HOSTED=true`
allows a `free` or `aisg_free` org and uses the 90-day window. Hosted free
stays 403. A bookmark outside that
window is not a safe resume point. Continuing would skip a gap if those rows
were removed. Re-sync reads what is still stored, from the start.

```json
{ "error": "Invalid request: invalid cursor; drop the cursor and re-sync from the start", "status": 400 }
```

Drop the stored cursor only when a 400 contains `invalid cursor`. A bad
`limit` is also 400 and does not contain that phrase, so keep the cursor
and fix the query.

```json
{ "error": "Invalid request: limit must be an integer 1-1000", "status": 400 }
```

```json
{ "error": "passed-event cursor is older than the 90-day retention window for this plan; drop the cursor and re-sync from the start", "status": 410 }
```

## Response

```json
{
  "events": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "contract_version": "1.0.0",
      "created_at": "2026-10-16T12:00:00.123456Z",
      "payload": { "user_id": "u_1", "event_type": "purchase" }
    }
  ],
  "next_cursor": "MTc5MjE1MjAwMDEyMzQ1Niw1NTBlODQwMC1lMjliLTQxZDQtYTcxNi00NDY2NTU0NDAwMDA",
  "has_more": false
}
```

`payload` is JSON `null` when the body was not stored. Storage is on only
for a paid plan with the org switch and the per-contract override both on
(see [event payload storage](event-payload-storage-reference.md)). A passing
event is a JSON object, so `null` means "body not stored", not an empty
event. A null payload is still an event on this page.

`next_cursor` is always a resume cursor. Save it and send it as `cursor` on
the next call.

When this page has events, `next_cursor` is the cursor of the last returned
event, even when `has_more` is false. A one-page contract still returns that
cursor, so the next poll does not start over.

When this page is empty, `next_cursor` repeats the cursor you sent. It is
null only when you sent no cursor. `has_more` alone says whether to keep
paging now. `has_more` is false when this page ended inside the holdback
window; still store `next_cursor`.

Kafka and Kinesis consumers publish from memory and do **not** insert
`forwarded_events`. Those passes are not in this feed. HTTP ingest, egress,
and quarantine replay are.

## Related

- [POST /v1/ingest](v1-ingest-reference.md)
- [Event payload storage](event-payload-storage-reference.md)
- [Plans](aischemagate-plans-reference.md)
