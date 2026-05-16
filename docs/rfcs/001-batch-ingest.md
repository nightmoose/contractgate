# RFC-001: Batch Ingest

| Status        | Accepted (2026-04-17)                    |
|---------------|------------------------------------------|
| Author        | ContractGate team                        |
| Created       | 2026-04-17                               |
| Target branch | `nightly-maintenance-2026-04-17`         |
| Tracking      | Post-demo feedback item #1               |

## Summary

Elevate the existing `POST /ingest/{contract_id}` endpoint from a sequential
single-or-array handler (500-event cap, one-at-a-time validation) into a true
batch ingestion path: **1,000-event hard cap, parallel validation via `rayon`,
optional `atomic` mode for transactional semantics.**

This is an enhancement, not a greenfield feature. The endpoint already normalizes
array and single-event bodies into a batch and returns per-item results with
appropriate HTTP status codes (200 / 207 / 422). What's missing is (a) raising
the cap, (b) parallelism, (c) the atomic opt-in, and (d) scaling the audit-log /
quarantine write path so it doesn't thrash Postgres under load.

## Goals

1. Accept up to 1,000 events in a single request.
2. Validate those events in parallel across CPU cores.
3. Offer `?atomic=true` for all-or-nothing batch semantics.
4. Keep existing per-item default behavior for existing callers.
5. Remain within the <15 ms p99 latency budget per event, and land a batch of
   1,000 typical events in <100 ms end-to-end on a 4-core runner.

## Non-goals

- **Contract versioning.** Deferred to RFC-002. All events in a batch validate
  against the same (latest) compiled contract for this contract_id.
- **PII masking.** Deferred to RFC-004.
- **Batched forwarding to downstream destinations.** The per-event insert into
  `forwarded_events` stays as-is; optimizing that is a separate concern.
- **Kafka-fed batch intake.** The `demo` binary's Kafka pipeline is out of scope.
- **Per-contract rate limiting.** Global timeout middleware is already in place;
  rate limiting is a future concern.

## Current state (as of commit on `main`, 2026-04-17)

`src/ingest.rs::ingest_handler`:
- Accepts `Path(contract_id)`, `Query(IngestQuery { dry_run })`, JSON body.
- Normalizes body: `Value::Array → Vec<Value>`, single value → `vec![v]`.
- Enforces `MAX_BATCH_SIZE = 500`.
- Iterates sequentially: `for event in &events { validate(...); ... }`.
- Per-event side effects are fire-and-forget via `tokio::spawn`:
  - Audit log write (`storage::log_audit_entry`)
  - Quarantine write on failure (`storage::quarantine_event`)
  - `forward_event` is awaited inline (sequential)
- Returns `BatchIngestResponse` with `total / passed / failed / dry_run / results[]`.
- Status: 200 (all pass) / 207 (mixed) / 422 (all fail).

The shape is close to what we want; the bottlenecks are sequential validation
and the per-event DB task explosion.

## Design

### 1. Endpoint surface

Keep the single endpoint. No new path — less API surface, and the current
"auto-detect single vs. array" pattern is already documented in the dashboard.

```
POST /ingest/{contract_id}
Query: ?dry_run=true       (existing)
       ?atomic=true        (new)
```

Behavior matrix:

| Body     | atomic | Outcome                                                          |
|----------|--------|------------------------------------------------------------------|
| single   | false  | Existing single-event behavior. `atomic` ignored with a warning. |
| single   | true   | Same as above — `atomic` is a no-op on a 1-item batch.           |
| array    | false  | Per-item results. Partial success returns 207.                   |
| array    | true   | All-or-nothing. Any violation → 422, nothing persisted.          |

### 2. Request limits

```rust
const MAX_BATCH_SIZE: usize = 1_000;
```

Raised from 500. A rejected-as-too-large request returns `400 Bad Request` with
a message listing the submitted size and the cap (matches current behavior).

### 3. Response schema

Extend `BatchIngestResponse` with a single optional field:

```rust
pub struct BatchIngestResponse {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub atomic: bool,              // NEW — echoes the query flag
    pub results: Vec<IngestEventResult>,
}
```

For `atomic=true` *and* any failure, `results` contains the per-item validation
outcomes (so the client can see which indices broke the batch) but `forwarded`
and any audit-log writes are suppressed. The response payload makes the
transactional semantics explicit.

### 4. Parallel validation

Use `rayon` (add to `Cargo.toml`). The validator is pure and already takes
`&CompiledContract + &Value` — trivially `Sync`:

```rust
use rayon::prelude::*;

let results: Vec<ValidationResult> = events
    .par_iter()
    .map(|event| validate(&compiled, event))
    .collect();
```

Because validation runs on the tokio runtime's worker thread, we want to fan
out onto rayon's pool (not block the async executor). Wrap with
`tokio::task::spawn_blocking` so the async reactor stays free:

```rust
let compiled_arc = Arc::clone(&compiled);
let events_arc = Arc::new(events);
let results = tokio::task::spawn_blocking(move || {
    events_arc
        .par_iter()
        .map(|e| validate(&compiled_arc, e))
        .collect::<Vec<_>>()
}).await?;
```

### 5. Atomic-mode semantics

When `atomic=true`:

1. Run validation (parallel, same as above).
2. If **any** event has `passed == false`:
   - **Do not** write audit log entries for the passing events (the batch is
     logically rejected as a unit).
   - **Do not** quarantine individual events.
   - **Do write one `batch_rejected` audit entry** per batch attempt with
     `violation_count = failing_indices.len()` and `violation_details = [{ index, violations: [...] }, ...]`.
     This preserves the "every decision is audited" property without creating
     per-event noise for a rejected batch.
   - Return **422** with the full per-item `results[]` so the client can debug.
3. If **all** events pass, proceed as non-atomic: write audits, forward, return 200.

### 6. Scaling the write path

At 1,000 events per request, fire-and-forget `tokio::spawn` per event means up
to 2,000 concurrent Postgres connections in flight per batch (audit + potential
quarantine). This will exhaust the pool fast. Replace with a single multi-row
insert per batch:

```rust
// storage.rs — new helpers
pub async fn log_audit_entries_batch(
    pool: &PgPool,
    entries: &[AuditEntryInsert],
) -> AppResult<()> { ... }

pub async fn quarantine_events_batch(
    pool: &PgPool,
    entries: &[QuarantineInsert],
) -> AppResult<()> { ... }
```

Implementation uses `UNNEST` of typed arrays — one roundtrip regardless of
batch size:

```sql
INSERT INTO audit_log (id, contract_id, passed, violation_count, violation_details,
                       raw_event, validation_us, source_ip, created_at)
SELECT gen_random_uuid(), $1, p.passed, p.vc, p.vd, p.re, p.us, $2, NOW()
FROM UNNEST($3::bool[], $4::int[], $5::jsonb[], $6::jsonb[], $7::bigint[])
  AS p(passed, vc, vd, re, us);
```

The batch handler collects all audit/quarantine inserts into vectors and calls
the batch helpers once after validation completes. Both are still spawned onto
the async runtime (not awaited inline in the response path) so the HTTP
response is not blocked on durability — matches existing fire-and-forget policy.

### 7. Forwarded events

The existing per-event `INSERT INTO forwarded_events` also moves to a multi-row
insert for the same reason. This is a smaller change — one new helper
`forward_events_batch`.

### 8. Error model

No new `AppError` variants needed. The existing `BadRequest("Batch too large...")`
covers the cap. Any DB failure in the batch write helpers is logged via
`tracing::warn!` (same as current fire-and-forget behavior) and does not affect
the response, except under `atomic=true` where the batch-rejected audit write
*must* succeed — if it fails, return 500 and let the client retry.

### 9. Metrics

Add to the `tracing::info!` emitted from the handler (so existing log-based
observability picks it up):

- `batch_size`
- `parallel_validation_us` (total wall clock across the parallel stage)
- `atomic_mode` (bool)
- `atomic_rejected` (bool — only when atomic=true and failed>0)

Future work: expose a Prometheus counter + histogram. Out of scope for this RFC.

## Test plan

Unit tests (`src/tests.rs` → new `mod batch`):

1. `batch_of_1_behaves_like_single` — 1-event array matches single-event result.
2. `batch_all_pass_returns_200` — 5 valid events, no `atomic`, expect 200.
3. `batch_all_fail_returns_422` — 5 invalid events, expect 422.
4. `batch_mixed_returns_207` — 3 valid + 2 invalid, expect 207, `results` ordered.
5. `batch_over_cap_rejected` — 1,001 events → 400 with message mentioning the cap.
6. `batch_empty_rejected` — `[]` → 400.
7. `atomic_all_pass_returns_200` — 5 valid + `atomic=true` → 200, all audited.
8. `atomic_any_fail_returns_422` — 4 valid + 1 invalid + `atomic=true` → 422,
   zero `forwarded: true`, zero passing-event audit rows, one batch-rejected audit row.
9. `atomic_on_single_event_noop` — 1-event body + `atomic=true` behaves identically
   to atomic=false (no surprising semantics on small batches).
10. `dry_run_batch_skips_writes` — `dry_run=true` + array → no audit / quarantine /
    forward, results still populated.
11. `parallel_order_preserved` — 100-event batch, verify `results[i]` matches
    input event `i` (rayon `par_iter().map().collect()` preserves order, but
    pin this as a regression guard).

Integration / benchmark (manual + optional CI step):

- `cargo run --release` + `curl` a 1,000-event batch of known-good events →
  measure end-to-end latency. Target: <100 ms on 4-core CI runner.
- Repeat with 50% invalid events and `atomic=false` → same latency budget.
- Repeat with `atomic=true` and one forced failure → verify nothing persisted.

## Rollout / migration

- **Code:** single PR to `nightly-maintenance-2026-04-17`. No schema changes.
- **Frontend:** `dashboard/lib/api.ts::BatchIngestResponse` needs the new
  `atomic` field (optional on the type so older responses still parse). Add
  an "atomic" checkbox to the Playground page if there's time in the PR —
  otherwise defer to a follow-up.
- **Backward compat:** existing callers sending single events or arrays without
  `?atomic` get identical behavior. The only observable change for them is the
  cap going from 500 → 1,000, which is monotonically more permissive.

## Open questions

1. **Should the batch-rejected audit entry under atomic mode be a new row type,
   or encoded as a regular `audit_log` row with `passed=false` and a sentinel
   violation kind?** Leaning toward the latter — no schema change, and queries
   that group-by `passed` continue to work. Flagging for review.
2. **Do we want a per-contract override for `MAX_BATCH_SIZE`?** Possibly —
   some contracts are low-volume / large payloads and a smaller cap makes
   sense. Deferring to RFC-002 (versioning) since contract-level config will
   naturally land there.

## Dependencies / follow-ups

- **RFC-002 (versioning)** — will introduce contract-level config that might
  subsume `MAX_BATCH_SIZE` override and `retention_days`.
- **Prometheus metrics** — tracked as post-RFC-001 polish.
- **Audit log partitioning** — when batch ingest lands in production we'll
  want to partition `audit_log` by `created_at` to keep writes fast. Not
  blocking; tracked separately.
