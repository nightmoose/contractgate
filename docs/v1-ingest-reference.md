# POST /v1/ingest/{contract_id} — Endpoint Reference

The bulk HTTP ingest endpoint is ContractGate's universal connector: anything
that can make an HTTP POST can validate events against a contract.

---

## Request

```
POST /v1/ingest/{contract_id}
Host: contractgate.io
X-Api-Key: cg_live_<your_key>
Content-Type: application/json          # or application/x-ndjson
Idempotency-Key: <opaque_string>        # optional
```

### Path parameter

| Parameter     | Type | Required | Description                          |
|---------------|------|----------|--------------------------------------|
| `contract_id` | UUID | Yes      | ID of the contract to validate against. |

### Query parameters

| Parameter | Type    | Default         | Description                                                      |
|-----------|---------|-----------------|------------------------------------------------------------------|
| `version` | string  | latest `stable` | Semver pin (e.g. `1.2.0`). Defaults to the latest stable version. |
| `dry_run` | boolean | `false`         | Validate without persisting to audit log, quarantine, or forward. |
| `atomic`  | boolean | `false`         | All-or-nothing semantics: if any event fails, nothing is persisted. |

### Request headers

| Header            | Required | Description                                                                                  |
|-------------------|----------|----------------------------------------------------------------------------------------------|
| `X-Api-Key`       | Yes      | API key in `cg_live_…` format. Must be authorized for the contract's project.               |
| `Content-Type`    | Yes      | `application/json` or `application/x-ndjson`.                                               |
| `Idempotency-Key` | No       | Opaque string (max 255 chars). Guarantees at-most-once processing within the 24-hour window. |

### Body formats

**JSON array** (`application/json`):
```json
[
  { "user_id": "u_123", "event_type": "purchase", "timestamp": 1714000000, "amount": 49.99 },
  { "user_id": "u_456", "event_type": "login",    "timestamp": 1714000001 }
]
```

**Single JSON object** (`application/json`) — treated as a 1-event batch:
```json
{ "user_id": "u_123", "event_type": "login", "timestamp": 1714000001 }
```

**NDJSON** (`application/x-ndjson`) — one object per line:
```
{"user_id":"u_123","event_type":"purchase","timestamp":1714000000,"amount":49.99}
{"user_id":"u_456","event_type":"login","timestamp":1714000001}
```

### Limits

| Limit              | Value   | Error code        |
|--------------------|---------|-------------------|
| Max request body   | 10 MB   | `body_too_large`  |
| Max events / batch | 1 000   | `batch_too_large` |
| Max per event      | 1 MB    | `event_too_large` |

---

## Response

### Success (200 / 207)

```json
{
  "total": 2,
  "passed": 2,
  "failed": 0,
  "dry_run": false,
  "atomic": false,
  "resolved_version": "1.2.0",
  "version_pin_source": "default_stable",
  "results": [
    {
      "index": 0,
      "passed": true,
      "violations": [],
      "validation_us": 312,
      "forwarded": true,
      "contract_version": "1.2.0",
      "quarantine_id": null,
      "transformed_event": { "user_id": "u_123", "event_type": "purchase",
                             "timestamp": 1714000000, "amount": 49.99 }
    }
  ]
}
```

| Field                | Type    | Description                                                          |
|----------------------|---------|----------------------------------------------------------------------|
| `total`              | integer | Total events submitted.                                              |
| `passed`             | integer | Events that passed validation.                                       |
| `failed`             | integer | Events that failed validation.                                       |
| `dry_run`            | boolean | Whether this was a dry run.                                          |
| `atomic`             | boolean | Whether atomic mode was requested.                                   |
| `resolved_version`   | string  | Contract version actually used.                                      |
| `version_pin_source` | string  | `"query_param"` or `"default_stable"`.                              |
| `results[].index`    | integer | Zero-based position in the submitted batch.                          |
| `results[].passed`   | boolean | Whether this event passed.                                           |
| `results[].violations` | array | Validation violations (empty on pass).                             |
| `results[].quarantine_id` | UUID \| null | ID of the quarantine row for rejected events. Use with the replay API. |
| `results[].transformed_event` | object | Post-transform payload that was persisted (RFC-004).      |

### HTTP status codes

| Code | Meaning                                                              |
|------|----------------------------------------------------------------------|
| 200  | All events passed.                                                   |
| 207  | Mixed — some passed, some failed.                                    |
| 400  | Malformed body, empty batch, size limit exceeded.                    |
| 401  | Missing or invalid API key.                                          |
| 413  | Body > 10 MB or a single event > 1 MB.                              |
| 422  | All events failed, `atomic` rejection, idempotency conflict, or deprecated version pin. |
| 429  | Per-key rate limit exceeded. See `X-RateLimit-*` headers.            |

### Rate-limit headers (always present)

| Header                  | Value                                            |
|-------------------------|--------------------------------------------------|
| `X-RateLimit-Limit`     | Requests per second allowed for this key.        |
| `X-RateLimit-Remaining` | Tokens remaining after this request.             |
| `X-RateLimit-Reset`     | Unix timestamp when the bucket has ≥ 1 token.   |

Default limits: **100 req/sec sustained, 1 000 burst**. Contact us for design-partner overrides.

---

## Idempotency

Add the `Idempotency-Key` header to make any request idempotent within a
24-hour window. The key is opaque — use a UUID, transaction ID, or any
unique string (max 255 chars).

**Same key + same body → cached response:**
```
HTTP/1.1 200 OK
X-Idempotency-Replay: true
```

**Same key + different body → 422 conflict:**
```json
{ "error": "idempotency_conflict",
  "detail": "A different request body was already submitted with this Idempotency-Key." }
```

Dry-run requests (`?dry_run=true`) are never stored in the idempotency cache.

---

## Quarantine and replay

Rejected events are automatically quarantined. The `quarantine_id` in each
failing event result is the UUID of the quarantine row. Pass it to the replay
endpoint when the underlying data has been fixed:

```
POST /contracts/{contract_id}/quarantine/replay
```

---

## Version pinning

By default the endpoint validates against the latest `stable` version of the
contract. Pin a specific version with the `?version=` query parameter:

```
POST /v1/ingest/{contract_id}?version=1.0.0
```

Pinning a `deprecated` version quarantines the entire batch with a
`deprecated_contract_version` violation.

---

## OpenAPI spec

The machine-readable spec is available at `/openapi.json` (no auth required).
