# Usage Reference

**RFC:** 083 — Per-org event metering (Phase 1)
**Since:** nightly-2026-07-15

Per-org event usage for the current calendar month against the plan limit. Backs
the dashboard usage widget. Read-only; no enforcement yet (see RFC-083 Phase 2).

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
- Enforcement (429 at the cap) and a cached counter arrive in RFC-083 Phase 2.
