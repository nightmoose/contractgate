# Egress Validation Reference

ContractGate validates data on **ingest**.  The egress validation endpoint
runs the identical `validate()` engine against **outbound** payloads so you
can guarantee that what leaves your API also conforms to a named contract.

---

## Endpoint

```
POST /egress/{contract_id}
```

- **Auth**: requires `x-api-key` header (same as ingest).
- **Path**: `{contract_id}` — UUID of the contract.
  Accepts an optional `@version` suffix: `/egress/{uuid}@1.2.3`.
- **Version header**: `X-Contract-Version: 1.2.3` (takes precedence over
  the path suffix when both are supplied).
- **Default version**: latest stable (same as ingest).

### Request body

Same shape as the ingest endpoint — a single JSON object or a JSON array of
up to 1 000 objects.

```json
[
  { "user_id": "alice", "event_type": "purchase", "timestamp": 1712000000, "amount": 49.99 },
  { "user_id": "bob",   "event_type": "click",    "timestamp": 1712000001 }
]
```

### Query parameters

| Parameter     | Type    | Default | Description |
|---------------|---------|---------|-------------|
| `disposition` | string  | `block` | How to handle failing records. |
| `dry_run`     | boolean | `false` | Validate without writing to the database. |

---

## Disposition modes

| Mode    | Behavior | Response status |
|---------|----------|-----------------|
| `block` | Drop failing records from the response `payload`. Good records still ship. | 200 (all pass) / 207 (some blocked) |
| `fail`  | Any failing record rejects the **entire** response. Atomic mode. | 200 (all pass) / 422 (any fail) |
| `tag`   | All records pass through; failures are flagged in per-record `outcomes`. | 200 (all pass) / 207 (some tagged) |

The default disposition is **`block`** — graceful degradation: one bad row
does not block the rest.

---

## Response body

```json
{
  "total": 2,
  "passed": 1,
  "failed": 1,
  "dry_run": false,
  "disposition": "block",
  "resolved_version": "1.0.0",
  "payload": [
    { "user_id": "alice", "event_type": "purchase", "timestamp": 1712000000, "amount": 49.99 }
  ],
  "outcomes": [
    {
      "index": 0,
      "passed": true,
      "violations": [],
      "validation_us": 12,
      "action": "included"
    },
    {
      "index": 1,
      "passed": false,
      "violations": [
        {
          "field": "amount",
          "message": "Required field 'amount' is missing",
          "kind": "missing_required_field"
        }
      ],
      "validation_us": 9,
      "action": "blocked"
    }
  ]
}
```

### `action` values

| Value       | Meaning |
|-------------|---------|
| `included`  | Record passed and is present in `payload`. |
| `blocked`   | Record failed and was dropped from `payload` (block mode). |
| `rejected`  | Record is part of a wholesale rejection (fail mode). |
| `tagged`    | Record failed but is present in `payload` with a flag (tag mode). |

### HTTP status codes

| Status | Meaning |
|--------|---------|
| 200    | All records passed validation. |
| 207    | block or tag mode with a mix of pass and fail. |
| 422    | fail mode — any record failed; or all records failed in any mode. |
| 400    | Malformed request (bad UUID, empty batch, batch > 1 000). |
| 404    | Contract not found, or pinned version not found. |
| 409    | Contract has no stable version (and no version was pinned). |

---

## Audit trail

Every egress validation call writes to the same `audit_log` and
`quarantine_events` tables as ingest, tagged `direction = 'egress'`.
This means you can query both directions in one place:

```sql
-- All events (ingest + egress) for a contract
SELECT * FROM audit_log
WHERE contract_id = '…'
ORDER BY created_at DESC;

-- Egress-only audit rows
SELECT * FROM audit_log
WHERE contract_id = '…' AND direction = 'egress'
ORDER BY created_at DESC;
```

---

## Examples

### block (default): partial success

```bash
curl -X POST https://your-instance/egress/{contract_id} \
  -H "x-api-key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '[{"user_id":"alice","event_type":"purchase","timestamp":1712000000,"amount":49.99},
       {"user_id":"bob","event_type":"invalid_type","timestamp":1712000001}]'
# → 207, payload has 1 record, 1 blocked
```

### fail: atomic gate

```bash
curl -X POST "https://your-instance/egress/{contract_id}?disposition=fail" \
  -H "x-api-key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '[{"user_id":"alice","event_type":"purchase","timestamp":1712000000,"amount":49.99},
       {"user_id":"bob","event_type":"invalid_type","timestamp":1712000001}]'
# → 422, entire batch rejected
```

### tag: pass-through with flags

```bash
curl -X POST "https://your-instance/egress/{contract_id}?disposition=tag" \
  -H "x-api-key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '[{"user_id":"alice","event_type":"purchase","timestamp":1712000000,"amount":49.99},
       {"user_id":"bob","event_type":"invalid_type","timestamp":1712000001}]'
# → 207, both records in payload, 1 tagged with violations
```

### dry run: validate without persisting

```bash
curl -X POST "https://your-instance/egress/{contract_id}?dry_run=true" \
  -H "x-api-key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"user_id":"alice","event_type":"purchase","timestamp":1712000000}'
# → 200, no audit or quarantine rows written
```

### pin a specific version

```bash
# Path suffix
curl -X POST "https://your-instance/egress/{contract_id}@1.2.3" …

# Header (takes precedence)
curl -X POST "https://your-instance/egress/{contract_id}" \
  -H "X-Contract-Version: 1.2.3" …
```

---

## JavaScript / TypeScript client

```ts
import { egressValidate } from "@/lib/api";

// block mode (default)
const result = await egressValidate(contractId, payload);

// fail mode
const result = await egressValidate(contractId, payload, { disposition: "fail" });

// tag mode, pinned version
const result = await egressValidate(contractId, payload, {
  disposition: "tag",
  version: "1.2.3",
});

console.log(`${result.passed} passed, ${result.failed} failed`);
result.outcomes.filter(o => o.action === "tagged").forEach(o => {
  console.warn(`Record ${o.index} failed:`, o.violations);
});
```

---

## Related

- [RFC-029](rfcs/029-egress-validation.md) — design rationale and acceptance criteria.
- [RFC-030](rfcs/030-egress-pii-guard.md) — egress PII leakage transforms (future).
- [RFC-031](rfcs/031-provider-scorecard.md) — per-provider data-quality scorecard using `direction` data (future).
