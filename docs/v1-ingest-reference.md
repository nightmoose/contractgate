# POST /v1/ingest/{contract_id} — Endpoint Reference

The bulk HTTP ingest endpoint is DataContractGate's universal connector: anything
that can make an HTTP POST can validate events against a contract.

---

## Request

```
POST /v1/ingest/{contract_id}
Host: app.datacontractgate.com
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
| `Content-Type`    | Yes      | `application/json` or `application/x-ndjson`. Other types (e.g. `text/plain`, `application/xml`) get 415; a missing header is read as JSON. |
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
| `results[].violations` | array | Validation violations (empty on pass). See the violation shape below. |
| `results[].quarantine_id` | UUID \| null | ID of the quarantine row for rejected events. Use with the replay API. Always `null` on `dry_run=true` and on an atomic rejection, which store no quarantine rows. |
| `results[].transformed_event` | object | Post-transform payload that was persisted (RFC-004).      |
| `quarantined` | integer | Failures stored in quarantine. `0` on a dry run. |
| `dropped` | integer | Failures counted and not stored. `0` on a dry run. |
| `on_fail_mode` | string | `quarantine` (default) or `drop`. |
| `on_fail_source` | string | `request`, `contract`, `default`, or `auto_trip`. |
| `auto_trip` | object | Present when a safety cap is enforcing or would have tripped. |

`?on_fail=drop` counts failing records and skips both the quarantine row and the audit row for those records. Quarantine stays the default. See [Drop mode](drop-mode-reference.md).

### Violation shape

Each entry in `results[].violations` is a JSON object with these fields.
The three actionable fields (`received`, `expected`, `suggestion`) are omitted
when the check doesn't have a natural value for them, so consumers must
tolerate their absence.

```json
{
  "field": "timestamp",
  "message": "Field 'timestamp' expected type Integer, got string",
  "kind": "type_mismatch",
  "received": "2026-08-23T15:00:00Z",
  "expected": "integer",
  "suggestion": "Change the producer to emit 'timestamp' as integer, or update the contract field to `type: string` if the producer is correct."
}
```

| Field         | Type            | Description                                                                                |
|---------------|-----------------|--------------------------------------------------------------------------------------------|
| `field`       | string          | Dot-separated path to the offending field (e.g. `user.address.zip`).                       |
| `message`     | string          | Human-readable explanation.                                                                |
| `kind`        | string enum     | Machine-readable category. One of `missing_required_field`, `type_mismatch`, `pattern_mismatch`, `enum_violation`, `range_violation`, `length_violation`, `metric_range_violation`, `undeclared_field`, `leakage_violation`, `completeness_violation`, `freshness_violation`, `uniqueness_violation`. |
| `received`    | any (optional)  | The offending value as it appeared in the event.                                           |
| `expected`    | string (optional) | Contract-YAML-friendly description of the required value (e.g. `"integer"`, `">= 0"`, `"one of [click, view]"`). |
| `suggestion`  | string (optional) | One-line remediation an agent or human can act on directly.                              |

### HTTP status codes

| Code | Meaning                                                              |
|------|----------------------------------------------------------------------|
| 200  | All events passed.                                                   |
| 207  | Mixed — some passed, some failed.                                    |
| 400  | Malformed body, empty batch, size limit exceeded.                    |
| 401  | Missing or invalid API key.                                          |
| 413  | Body > 10 MB or a single event > 1 MB.                              |
| 415  | `Content-Type` is not JSON or NDJSON.                                |
| 422  | All events failed, `atomic` rejection, idempotency conflict, or deprecated version pin. |
| 429  | Per-key rate limit exceeded. See `X-RateLimit-*` headers.            |

### Rate-limit headers (always present)

| Header                  | Value                                            |
|-------------------------|--------------------------------------------------|
| `X-RateLimit-Limit`     | Burst capacity: requests that can go through back to back (Free 20, Pro 200). The bucket refills at the plan rate (Free 10/s, Pro 100/s). |
| `X-RateLimit-Remaining` | Requests that can still go through right now; drops by one per request. |
| `X-RateLimit-Reset`     | Unix timestamp when the bucket is full again (Remaining back at Limit). Sent on every authenticated route. |

Plan limits: **Free 10 req/sec sustained, bursts up to 20; Pro 100 req/sec, bursts up to 200.** Contact us for design-partner overrides.

The window is **per second, not per minute**: a token bucket refilling
continuously at the plan rate. An empty Free bucket is full again after 2 s,
so `X-RateLimit-Reset` is at most `burst / rate` (2 s) ahead and `Retry-After`
on a 429 is usually `1`. Bursts of ≤20 spaced ≥2 s apart never see a 429.
Because refill is continuous, a parallel burst that takes `t` seconds to
arrive admits `20 + floor(10·t)` on Free (e.g. 21 of 30 when the requests
span 100–200 ms). The same bucket applies to every authenticated route
(`/v1/ingest`, `/egress`, …), one per API key.

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
POST /v1/ingest/{contract_id}@1.0.0        # same thing (D1, 2026-10-14)
```

`?version=` wins if both are given. The same `{id}@{version}` form works on
`/ingest` and `/egress`.

### Deprecated pin — same 422 on every route

Pinning a `deprecated` version on `/v1/ingest`, `/ingest` or `/egress`
returns **422** with `"code": "contract_version_deprecated"`, the pinned
version, the latest stable version (where known) and a message saying what to
do. Ingest quarantines the whole batch (nothing on `dry_run`); egress forwards
nothing. Each per-event result carries one violation with
`kind: "contract_version_deprecated"`:

```json
{
  "error": "Version 1.0 of contract … is deprecated and no longer accepts events (every event was quarantined). Remove the version pin to use the latest stable version, or pin a stable version (e.g. …@X.Y or ?version=X.Y).",
  "code": "contract_version_deprecated",
  "deprecated_version": "1.0",
  "version_pin_source": "pinned_deprecated",
  "total": 1, "passed": 0, "failed": 1, "dry_run": false,
  "results": [ … ]
}
```

---

## OpenAPI spec

The machine-readable spec is available at `/openapi.json` (no auth required).

Passing events written to `forwarded_events` are read back with
[`GET /v1/contracts/{id}/passed`](passed-events-reference.md).
