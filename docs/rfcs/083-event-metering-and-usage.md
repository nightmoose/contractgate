# RFC-083 — Per-org event metering + usage API

**Status:** Draft
**Date:** 2026-07-15
**Branch:** nightly-maintenance-2026-07-15-rfc083
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

### Phase 2 — enforcement (next push, separate RFC-scoped review)

- Cached counter table `org_monthly_usage(org_id, period, events)` incremented
  once per ingest **batch** (single UPSERT — marginal vs the audit write already
  on that path), so enforcement reads O(1) instead of counting.
- Ingest returns **429** with a clear JSON body
  (`{error, plan, limit, used, period, upgrade_url}`) when the org is at/over its
  cap. Batch-granularity check (a batch that crosses the cap is allowed once).
- Migration + `EXPECTED_MIGRATION_COUNT` bump + sentinel.
- Load-check p99 stays < 15 ms before merge (the reason this is its own push).

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

- Counting *audit_log* rows = billable validated events (accepted into
  validation), matching the RFC-081 metering intent. Idempotent-retry dedup lands
  with the Phase-2 counter.
- Self-hosted (no org) is unmetered; `/usage` requires org context (401 in prod
  without it).
