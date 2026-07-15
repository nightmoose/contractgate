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
- **Increment:** on the normal HTTP path, UPSERT runs in the *same* `tokio::spawn`
  as the audit write (only after audit succeeds) so crash/restart under-counts
  both together rather than diverging. Envelope path (no audit today) uses its
  own fire-and-forget spawn.
- **Envelope (MRI/Findigs) is billable:** `passed + quarantined` records are
  counted even though the legacy short-circuit still skips `audit_log` (pre-
  existing audit gap; meter independently so caps stay honest).
- **Kafka / Kinesis unmetered in v1:** streaming ingress does not check or
  increment the counter (429-ing a consumer loop is impractical). Prefer
  Enterprise for streaming orgs so the gap is moot; document that `/usage` may
  under-report for Free/Growth streaming traffic. Follow-up: increment only
  (no 429) on stream paths if Growth needs streaming.
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
  Envelope traffic is billable but not yet audited (pre-existing gap).
- Self-hosted (no org) is unmetered; `/usage` requires org context (401 in prod
  without it).
- Soft-quota v1 accepts rare async under-count (process death after response)
  and rare bootstrap over-count; optional later: reconcile job from `audit_log`.
