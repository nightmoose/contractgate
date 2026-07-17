# RFC-083 — Per-org event metering + usage API

**Status:** Implemented (Phase 1 + 2 + 3; Phase 2 on branch for review)
**Date:** 2026-07-15
**Branch:** nightly-maintenance-2026-07-15-rfc083-phase2
**Depends on:** RFC-045 (plan gating), RFC-047 (org scoping)
**Dual-sell:** #5 (counter), #6 (enforcement), #7 (dashboard widget)

---

## Problem

The pricing table promises Free 1M / Growth 50M / Enterprise unlimited events per
month, but nothing counts or enforces it. There's no usage surface for a customer
("how close am I to my limit?") and no monetization machinery for an acquirer
("tiers are real, not marketing"). Plans exist (`orgs.plan`: `free`/`growth`/
`enterprise`, migration 026) but are unmetered.

## Plan limits (canonical, backend-owned)

| Plan | Monthly event limit |
|---|---|
| `free` | 1,000,000 |
| `growth` | 50,000,000 |
| `enterprise` | unlimited (no cap) |

Defined once in `src/plan.rs::monthly_event_limit(plan) -> Option<i64>` (None =
unlimited; unknown plan → most restrictive). The `/usage` response returns the
limit so the dashboard never hardcodes it.

## Phasing (this RFC ships Phase 1 only)

Metering touches the validation hot path (p99 < 15 ms, non-negotiable), so it is
split:

### Phase 1 — usage read surface (this push)

- `GET /usage` (org-scoped): current UTC-calendar-month usage for the caller's
  org, computed as a **live count** over `audit_log`
  (`WHERE org_id = $1 AND created_at >= <month_start>`), backed by
  `audit_log_org_id_created_idx` (migration 007). No new table, **no hot-path
  change**.
- `src/plan.rs` limits map. `src/usage.rs` handler + `storage::get_org_plan` +
  `storage::monthly_event_count`.
- Backs the dashboard usage widget (#7 frontend, follow-up).

Response:

```json
{
  "plan": "free",
  "period_start": "2026-07-01T00:00:00Z",
  "used": 412334,
  "limit": 1000000,
  "remaining": 587666,
  "pct": 41.23,
  "unlimited": false
}
```

`limit`/`remaining`/`pct` are null and `unlimited: true` for Enterprise.

### Phase 2 — enforcement (landed on `nightly-maintenance-2026-07-15-rfc083-phase2`)

- Cached counter table `org_monthly_usage(org_id, period, events)` (migration 032).
- **Check:** `ensure_monthly_usage` (PK read; one-time audit bootstrap if missing)
  then `used >= limit` → **429** `plan_limit_exceeded` with
  `{error, plan, limit, used, period, upgrade_url, status}`.
- **Fail-open on infra errors:** plan/counter DB failures log and *allow* the
  request. Metering must never 500 the ingest hot path. Binary-before-migration
  means "not yet enforcing," not "every ingest 500s." Apply migration 032 first
  so enforcement actually works.
- **`dry_run` is not billable:** cap check is skipped when `?dry_run=true` so
  over-cap orgs can still validate.
- **Increment:** on the HTTP path (including envelope), UPSERT runs in the
  *same* `tokio::spawn` as the audit write (only after audit succeeds) so
  crash/restart under-counts both together rather than diverging.
- **Envelope (MRI/Findigs) is billable and audited:** per-record
  audit/quarantine/forward after `validate_envelope_batch` (P0 fix), then
  meter from audit row count.
- **Kafka / Kinesis unmetered in v1 (product decision):** streaming ingress does
  not enforce the cap and does not increment the counter (429 on a consumer
  loop is impractical). Stream audit rows currently store `org_id = NULL`, so
  they also do not seed `/usage`. **Prefer Enterprise** for streaming orgs
  (unlimited → gap moot). Growth may enable Kafka/Kinesis UI tabs, but
  Free/Growth stream traffic under-reports usage until a follow-up wires
  contract→org_id + increment-only on stream paths.
  **UPDATE 2026-07-16 (billing-integrity pass):** the Kafka/Kinesis consumers now
  resolve the contract's owning `org_id` and stamp it on their `audit_log` rows,
  so the up-only reconcile counts stream events per-org and `/usage` no longer
  under-reports them (eventually consistent within the reconcile interval). Real-time
  429 on streams remains out of scope by design.
- **Reconcile job:** background task (default every 6h, env
  `USAGE_RECONCILE_INTERVAL_SECS`) and CLI `usage-reconcile` raise counters
  with `events = GREATEST(events, audit_count)` for the current UTC month —
  up-only so fire-and-forget under-count is repaired without erasing higher
  counters.
- Batch that *crosses* the cap while still under is allowed once
  (`used < limit` before the batch).
- Enterprise / no-org (self-host): never blocked.
- CI: `EXPECTED_MIGRATION_COUNT=32` + Sentinel A7.

### Phase 3 — dashboard usage widget (#7)

- `dashboard/` component calling `/usage`: "events this period / plan limit" + a
  bar + upgrade link when `pct` is high. Frontend-only.

## Testing (Phase 1)

- Serde: `/usage` wire shape locked.
- DB-backed (`migrations-check`): seed an org + audit rows in/out of the current
  month; assert `used` counts only this month, `limit`/`remaining`/`pct` for each
  plan, and Enterprise → unlimited.
- `cargo check && clippy -D warnings && test`.

## Notes

- Counting *validated HTTP events* (pass or fail). Phase 1 live-count used
  `audit_log`; Phase 2 prefers `org_monthly_usage`, bootstrapped once from audit.
  Envelope traffic is billable and audited (per-record durable path).
- Self-hosted (no org) is unmetered; `/usage` requires org context (401 in prod
  without it).
- Soft-quota v1 accepts rare async under-count until the next reconcile cycle;
  rare bootstrap over-count remains bounded.
