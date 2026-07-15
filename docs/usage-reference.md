# Usage Reference

**RFC:** 083 — Per-org event metering  
**Status:** Phase 1 (API) + Phase 3 (dashboard widget) shipped · Phase 2 (ingest 429) open  
**Since:** nightly-2026-07-15

Per-org event usage for the current calendar month against the plan limit. Backs
the **Usage this month** card on the account Billing page.

**Important:** this surface is **read-only**. Exceeding Free/Growth caps does **not**
yet return HTTP 429 on ingest (see RFC-083 Phase 2). Treat the widget as visibility
and upgrade prompting, not hard metering, until Phase 2 lands.

---

## Endpoint

```
GET /usage
```

Org-scoped (API key or Bearer JWT). Returns **401** in production without a
resolvable org. Self-hosted deployments (no org) are unmetered.

### Response

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

| Field | Notes |
|-------|-------|
| `plan` | `free`, `growth`, or `enterprise`. |
| `period_start` | First instant of the current UTC calendar month. |
| `used` | Events validated for this org since `period_start`. |
| `limit` | Monthly cap; `null` when unlimited (Enterprise). |
| `remaining` | `max(limit - used, 0)`; `null` when unlimited. |
| `pct` | Percent of cap used; `null` when unlimited. |
| `unlimited` | `true` for Enterprise. |

### Plan limits (backend-owned, canonical)

| Plan | Monthly event limit |
|------|---------------------|
| `free` | 1,000,000 |
| `growth` | 50,000,000 |
| `enterprise` | unlimited |

Defined in `src/plan.rs::monthly_event_limit`. The endpoint returns the limit so
clients don't hardcode it.

---

## Example

```bash
curl -H "x-api-key: $KEY" https://contractgate-api.fly.dev/usage
```

## Notes

- `used` is a live count over `audit_log` (billable validated events), backed by
  `audit_log_org_id_created_idx`. Cheap for a dashboard read; not on the ingest
  hot path.
- **Phase 2 (open):** when `used >= limit` for Free/Growth, ingest paths will
  return **429** with a clear JSON body (`plan`, `limit`, `used`, `period`,
  `upgrade_url`). Counter will be O(1) via a cached table (not a full
  `audit_log` count on every request). Requires p99 smoke before merge.
- Enterprise stays unlimited (`unlimited: true`, no 429 from plan caps).
