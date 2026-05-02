# RFC-021: REST/HTTP Bulk Ingest Endpoint (`POST /v1/ingest/{contract_id}`)

| Field         | Value                                        |
|---------------|----------------------------------------------|
| Status        | **Proposed — awaiting Alex sign-off**        |
| Author        | ContractGate team                            |
| Created       | 2026-05-01                                   |
| Target branch | `nightly-maintenance-2026-05-01`             |
| Tracking      | Phase 1 #4 — public self-serve demo backbone |

---

## Summary

Add a public, versioned bulk-ingest HTTP API at `POST /v1/ingest/{contract_id}`.
This is ContractGate's universal connector: anything that can make an HTTP
request can validate events against a contract. It is also the backbone of the
planned "5-minute self-serve demo".

The new route is a **net-new handler** that reuses the existing validation
engine, API key auth middleware, and quarantine flow unchanged. It does not
modify or replace `POST /ingest/:raw_id` (which remains for internal/legacy
use).

---

## Goals

1. Accept JSON arrays and NDJSON event batches over HTTP with a single call.
2. Return per-event validation results + aggregate accepted/rejected counts.
3. Enforce documented size limits (body, batch, per-event).
4. Idempotent replay: same key + same body → cached response; same key +
   different body → 422.
5. Per-API-key rate limiting with `X-RateLimit-*` headers.
6. Rejected events quarantined through the existing flow; quarantine IDs
   returned per-event.
7. OpenAPI spec generated from code (utoipa); published at `/openapi.json`.

## Non-goals

- Async / job-based ingest with status polling (Phase 2).
- Header-based contract identification (`X-Contract-Id` header).
- Streaming chunked ingest, gRPC, webhooks, signed URLs.
- Any new auth surface (API keys only; no SSO/OAuth).
- Changes to the existing `/ingest/:raw_id` handler.

---

## Endpoint shape

```
POST /v1/ingest/{contract_id}
```

### Path

| Segment       | Type | Notes                                |
|---------------|------|--------------------------------------|
| `contract_id` | UUID | Must identify an existing contract.  |

### Query parameters

| Param     | Type   | Default         | Notes                                      |
|-----------|--------|-----------------|--------------------------------------------|
| `version` | string | latest `stable` | Semver string. RFC-002 resolution applies. |
| `dry_run` | bool   | `false`         | Validate without persisting.               |
| `atomic`  | bool   | `false`         | All-or-nothing batch semantics.            |

Version resolution order (RFC-002):
1. `?version=` query param  
2. Latest `stable` by `promoted_at DESC`

(`X-Contract-Version` header is Phase 2 / internal only.)

### Request headers

| Header            | Required | Notes                                         |
|-------------------|----------|-----------------------------------------------|
| `X-Api-Key`       | Yes      | Existing API key auth. Unchanged.             |
| `Content-Type`    | Yes      | `application/json` or `application/x-ndjson`  |
| `Idempotency-Key` | No       | Opaque string, max 255 chars. See §Idempotency.|

### Request body

**`application/json`** — JSON array of event objects:
```json
[
  { "user_id": "u_123", "event_type": "purchase", "timestamp": 1714000000, "amount": 49.99 },
  { "user_id": "u_456", "event_type": "login",    "timestamp": 1714000001 }
]
```

**`application/x-ndjson`** — one JSON object per line, newline-terminated:
```
{"user_id":"u_123","event_type":"purchase","timestamp":1714000000,"amount":49.99}
{"user_id":"u_456","event_type":"login","timestamp":1714000001}
```

Single-object JSON bodies (not wrapped in an array) are also accepted and
treated as a 1-event batch — matching the existing `/ingest` behaviour.

### Response

**200 OK** — all events passed:
```json
{
  "total": 2,
  "passed": 2,
  "failed": 0,
  "dry_run": false,
  "atomic": false,
  "resolved_version": "1.2.0",
  "version_pin_source": "query_param",
  "results": [
    {
      "index": 0,
      "passed": true,
      "violations": [],
      "validation_us": 312,
      "forwarded": true,
      "contract_version": "1.2.0",
      "quarantine_id": null,
      "transformed_event": { "user_id": "u_123", "event_type": "purchase", "timestamp": 1714000000, "amount": 49.99 }
    },
    {
      "index": 1,
      "passed": true,
      "violations": [],
      "validation_us": 198,
      "forwarded": true,
      "contract_version": "1.2.0",
      "quarantine_id": null,
      "transformed_event": { "user_id": "u_456", "event_type": "login", "timestamp": 1714000001 }
    }
  ]
}
```

**207 Multi-Status** — mixed pass/fail batch.

**422 Unprocessable Entity** — all failed, or `atomic=true` + any failure, or
deprecated-version pin.

**400 Bad Request** — malformed body, empty batch, invalid `contract_id`,
exceeds batch/body/event size limit.

**401 Unauthorized** — missing or invalid `X-Api-Key`.

**413 Content Too Large** — body > 10 MB or any single event > 1 MB.

**422 Unprocessable Entity** — idempotency conflict (same key, different body).

**429 Too Many Requests** — rate limit exceeded.

The `index` field on each per-event result is new (not present on the legacy
`/ingest` response) and makes array-position unambiguous when NDJSON is used.

The `quarantine_id` field is new: UUID of the `quarantine_events` row created
for a rejected event, `null` for passing events. Enables callers to track
quarantined events directly.

---

## Size limits

All three enforced before validation; violation returns **413** with a
machine-readable error body:
```json
{ "error": "body_too_large", "detail": "Request body 11534337 bytes exceeds 10485760 byte limit." }
```

| Limit              | Value  | Error key         |
|--------------------|--------|-------------------|
| Max request body   | 10 MB  | `body_too_large`  |
| Max events / batch | 1 000  | `batch_too_large` |
| Max per-event size | 1 MB   | `event_too_large` |

The 1 000-event cap **replaces** the existing `MAX_BATCH_SIZE` constant in
`ingest.rs` — the constant is imported and reused here; no duplication.

Body-size enforcement uses `axum::extract::DefaultBodyLimit` (or
`tower_http::limit::RequestBodyLimitLayer`) at the router layer so it fires
before the handler reads any bytes.

Per-event size is checked during NDJSON line parsing; for JSON arrays it is
checked during the `serde_json::from_slice` pass over each element.

---

## Idempotency

### Storage

A new Supabase table (one new migration):

```sql
CREATE TABLE idempotency_keys (
    key          TEXT PRIMARY KEY,
    contract_id  UUID NOT NULL,
    body_hash    TEXT NOT NULL,       -- SHA-256 of raw request body, hex
    status_code  SMALLINT NOT NULL,
    response     JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);

-- TTL sweep: run nightly (or via pg_cron if available).
CREATE INDEX idempotency_keys_expires_at_idx ON idempotency_keys (expires_at);
```

Retention window: **24 hours** (`expires_at = now() + INTERVAL '24 hours'`).

### Behaviour

1. No header → no idempotency semantics. Request processed normally.
2. First request with key K:
   - Validate key length (≤ 255 chars). Reject > 255 with 400.
   - Hash request body with SHA-256.
   - Process normally. On completion, `INSERT INTO idempotency_keys` with key,
     `contract_id`, body hash, status code, and response body.
3. Repeat request with same key K + same body:
   - `SELECT` by key. Body hash matches → return cached status + response.
   - Response carries `X-Idempotency-Replay: true` header.
4. Repeat request with same key K + different body:
   - Body hash differs → **422** with:
     ```json
     { "error": "idempotency_conflict",
       "detail": "A different request body was already submitted with this Idempotency-Key." }
     ```
5. Expired keys: treated as absent (SELECT returns no row). Request processed
   fresh; old row replaced via `INSERT ... ON CONFLICT (key) DO UPDATE`.

### Race handling

Two concurrent requests with the same new key: the first to commit wins; the
second hits the unique constraint and should retry its SELECT to read the
cached response. Implement with `ON CONFLICT DO NOTHING` + immediate re-read.

---

## Rate limiting

### Algorithm

Per-API-key **token bucket**. State stored in `AppState` as a
`DashMap<Uuid, TokenBucket>` keyed by `api_key_id`. Each bucket tracks
`tokens: f64` and `last_refill: Instant`.

Default parameters (applied when `api_keys.rate_limit_rps IS NULL`):
- Sustained rate: **100 req/sec**
- Burst capacity: **1 000 tokens**

### Per-key overrides

Two nullable columns on `api_keys` (loaded alongside the existing row on auth):

```sql
ALTER TABLE api_keys
    ADD COLUMN rate_limit_rps   INT,   -- NULL = use default (100)
    ADD COLUMN rate_limit_burst INT;   -- NULL = use default (1000)
```

Overrides are propagated via `ValidatedKey`:
```rust
pub struct ValidatedKey {
    // existing fields ...
    pub rate_limit_rps:   Option<u32>,
    pub rate_limit_burst: Option<u32>,
}
```

The effective parameters for each request:
```
rps   = key.rate_limit_rps.unwrap_or(DEFAULT_RATE_LIMIT_RPS)
burst = key.rate_limit_burst.unwrap_or(DEFAULT_RATE_LIMIT_BURST)
```

### Response headers

Always returned on `/v1/ingest/*` responses:

| Header                  | Value                                          |
|-------------------------|------------------------------------------------|
| `X-RateLimit-Limit`     | Effective rps for this key                     |
| `X-RateLimit-Remaining` | Tokens remaining after this request            |
| `X-RateLimit-Reset`     | Unix timestamp (seconds) when bucket refills   |

On 429:
```json
{ "error": "rate_limit_exceeded",
  "detail": "Rate limit of 100 req/sec exceeded. Retry after 1s.",
  "retry_after_ms": 1000 }
```

---

## Quarantine integration

Rejected events flow through `storage::quarantine_events_batch` unchanged.
The `quarantine_events.id` assigned to each rejected row is returned in the
per-event `quarantine_id` field of the response. This ID can be passed to the
existing `POST /contracts/:id/quarantine/replay` endpoint for replay.

No changes to the quarantine schema or replay handler.

---

## NDJSON parsing

`Content-Type: application/x-ndjson` body is read as raw bytes, split on `\n`,
and each non-empty line is parsed as a JSON object. Blank lines (e.g. trailing
newline) are silently skipped. A line that is not valid JSON returns **400**
with the line index and parse error. A line whose parsed JSON exceeds 1 MB
returns **413**.

---

## OpenAPI

Add `utoipa` + `utoipa-axum` as dependencies. Annotate:
- `V1IngestRequest` (both content-type variants documented via `content`)
- `V1IngestResponse` and `V1IngestEventResult`
- All error response bodies
- Path, query params, and request headers

Expose `GET /openapi.json` on the **public** (unauthenticated) router. The
spec is generated at startup and served as a static `application/json`
response.

---

## Auth / API key scope check

The existing `require_api_key` middleware runs unchanged. After key validation,
the handler checks `ValidatedKey.allowed_contract_ids`:

```rust
if let Some(ref allowed) = key.allowed_contract_ids {
    if !allowed.contains(&contract_id) {
        return Err(AppError::Unauthorized);
    }
}
```

This mirrors the implicit scoping already present in the legacy ingest handler
but makes it explicit for the `/v1` surface.

---

## Implementation sketch

### New files / modules

| Path                           | Purpose                                      |
|--------------------------------|----------------------------------------------|
| `src/v1_ingest.rs`             | New handler + request/response types         |
| `src/rate_limit.rs`            | Token-bucket state + `check_rate_limit` fn   |
| `src/idempotency.rs`           | `check_idempotency` + `store_idempotency` fns|
| `supabase/migrations/NNN_…sql` | `idempotency_keys` table + `api_keys` columns|

### Router changes (`main.rs`)

```rust
// Public routes (no auth)
let public = Router::new()
    // ... existing ...
    .route("/openapi.json", get(openapi_handler));   // NEW

// Protected routes
let protected = Router::new()
    // ... existing ...
    .route("/v1/ingest/:contract_id", post(v1_ingest::handler));  // NEW
```

Body-size limit applied at the `/v1/ingest/*` subrouter level via
`tower_http::limit::RequestBodyLimitLayer` (10 MB).

### Key data types (`src/v1_ingest.rs`)

```rust
#[derive(Debug, Serialize)]
pub struct V1IngestEventResult {
    pub index:             usize,
    pub passed:            bool,
    pub violations:        Vec<Violation>,
    pub validation_us:     u64,
    pub forwarded:         bool,
    pub contract_version:  String,
    pub quarantine_id:     Option<Uuid>,     // NEW
    pub transformed_event: Value,
}

#[derive(Debug, Serialize)]
pub struct V1IngestResponse {
    pub total:              usize,
    pub passed:             usize,
    pub failed:             usize,
    pub dry_run:            bool,
    pub atomic:             bool,
    pub resolved_version:   String,
    pub version_pin_source: &'static str,    // "query_param" | "default_stable"
    pub results:            Vec<V1IngestEventResult>,
}
```

### Execution order inside the handler

1. Extract API key extension → `ValidatedKey`.
2. Check rate limit (`rate_limit::check`). If exceeded, return 429 + headers.
3. Check `Idempotency-Key` header. If present + cached → return replay.
4. Read body bytes (limit already enforced by middleware).
5. Parse body (JSON or NDJSON). Enforce per-event 1 MB limit.
6. Enforce ≤ 1 000 events.
7. Load contract identity. Check key scope.
8. Resolve version (`?version=` or latest stable).
9. Validate (parallel, rayon in `spawn_blocking`).
10. Apply RFC-004 transforms.
11. Persist (audit + quarantine + forward) — fire-and-forget.
12. Collect quarantine IDs for rejected events.
13. Store idempotency response (if key provided).
14. Return response with `X-RateLimit-*` headers.

---

## Test plan

### Unit tests (`src/v1_ingest.rs` + `src/rate_limit.rs` + `src/idempotency.rs`)

| Test                                    | What it covers                                    |
|-----------------------------------------|---------------------------------------------------|
| `json_array_body_parsed`                | Normal JSON array → correct event count           |
| `ndjson_body_parsed`                    | NDJSON lines → correct event count                |
| `ndjson_trailing_newline_ignored`       | Blank line at EOF does not count as event         |
| `ndjson_bad_line_returns_400`           | Malformed JSON line → 400 with line index         |
| `body_over_10mb_returns_413`            | RequestBodyLimitLayer fires before handler        |
| `event_over_1mb_returns_413`            | Per-event check with `event_too_large` key        |
| `batch_over_1000_returns_400`           | `batch_too_large` error key                       |
| `version_query_param_resolves`          | `?version=1.0.0` pins to that version             |
| `version_defaults_to_latest_stable`     | No param → latest stable selected                 |
| `idempotency_replay_same_body`          | Same key + same body → cached response, 200       |
| `idempotency_conflict_diff_body`        | Same key + different body → 422                   |
| `idempotency_expired_key_reprocessed`   | Expired key treated as new request                |
| `rate_limit_exceeded_returns_429`       | Bucket empty → 429 with Retry-After               |
| `rate_limit_headers_always_present`     | `X-RateLimit-*` on 200 and 429                    |
| `per_key_override_applied`              | Key with `rate_limit_rps=200` uses 200 rps        |
| `quarantine_id_returned_for_rejected`   | Failed event result has non-null `quarantine_id`  |
| `quarantine_id_null_for_passing`        | Passed event result has null `quarantine_id`      |
| `api_key_contract_scope_enforced`       | Key scoped to other contract → 401                |
| `dry_run_no_idempotency_stored`         | `dry_run=true` → no row in `idempotency_keys`     |

### Integration tests (`tests/v1_ingest_integration.rs`)

1. **Post → reject → quarantine → replay round trip**: submit one invalid event,
   confirm quarantine row created, replay via existing replay endpoint, confirm
   revalidation result.
2. **Idempotency end-to-end**: two identical requests with same key against a
   real DB → second returns `X-Idempotency-Replay: true`.
3. **Rate limit burst**: fire 1 001 requests synchronously → first 1 000 pass,
   remainder return 429.

### Load test (`tests/load_v1_ingest.rs` or external script)

- 1 000-event JSON batch, 50 concurrent connections.
- Target: p99 < 15 ms per-event, <100 ms end-to-end for 1k-event batch.
- Tooling: `drill` or `oha` CLI (no binary dep in CI; script stored in
  `ops/load/v1_ingest.yaml`).

---

## Schema migrations

New migration file `supabase/migrations/NNN_v1_ingest.sql`:

```sql
-- Idempotency cache
CREATE TABLE IF NOT EXISTS idempotency_keys (
    key          TEXT PRIMARY KEY CHECK (char_length(key) <= 255),
    contract_id  UUID NOT NULL,
    body_hash    TEXT NOT NULL,
    status_code  SMALLINT NOT NULL,
    response     JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX idempotency_keys_expires_at_idx ON idempotency_keys (expires_at);

-- Per-key rate limit overrides
ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS rate_limit_rps   INT CHECK (rate_limit_rps > 0),
    ADD COLUMN IF NOT EXISTS rate_limit_burst INT CHECK (rate_limit_burst > 0);
```

---

## Dependencies to add

| Crate            | Version | Purpose                                      |
|------------------|---------|----------------------------------------------|
| `utoipa`         | 4       | OpenAPI spec generation from code            |
| `utoipa-axum`    | 0.1     | Axum router integration for utoipa           |
| `dashmap`        | 5       | Concurrent `HashMap` for per-key rate buckets|

`dashmap` is `Send + Sync` and lock-free for reads — appropriate for hot-path
rate-limit checks. No `RwLock` wrapping needed.

---

## Rollout

1. **Migration** — apply before deploying new binary (additive only; existing
   rows unaffected).
2. **Binary** — single PR to `nightly-maintenance-2026-05-01`. No existing
   route changes.
3. **Backward compat** — `/ingest/:raw_id` untouched. No client changes needed.
4. **Feature flag** — not required (new path, no conflict).

---

## Open questions (resolved)

| Question                                | Decision                                    |
|-----------------------------------------|---------------------------------------------|
| Idempotency storage: Supabase vs memory | **Supabase table** — survives restarts      |
| Rate limit overrides: where stored      | **`api_keys` table columns** — loaded on auth|
| Endpoint path                           | **`/v1/ingest/{contract_id}`** — new prefix |

---

## Milestones

| Date       | Target                                                        |
|------------|---------------------------------------------------------------|
| 2026-05-08 | Endpoint live in staging, OpenAPI published, internal sign-off|
| 2026-05-15 | Examples + docs live, 5-min demo using endpoint, load passed  |
