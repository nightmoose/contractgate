# Usage Reference

**RFC:** 083 — Per-org event metering  
**Status:** Phase 1 (API) + Phase 2 (ingest 429) + Phase 3 (widget)  
**Since:** nightly-2026-07-15 (Phase 2: `nightly-maintenance-2026-07-15-rfc083-phase2`)

Per-org event usage for the current calendar month against the plan limit. Backs
the **Usage this month** card on the account Billing page.

**Phase 2 enforcement:** when the org is already at/over its Free or Growth
monthly cap, `POST /ingest/*` and `POST /v1/ingest/*` return **429**
`plan_limit_exceeded` (see below). Enterprise is unlimited. Self-hosted (no org)
is unmetered. Metering **fails open** on DB errors (logs + allows). `?dry_run=true`
skips the cap check. Envelope contracts (MRI/Findigs) count toward the limit.
**Kafka / Kinesis ingress is not metered in v1** — prefer Enterprise for
streaming orgs; `/usage` may under-report Free/Growth stream traffic.

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

- `used` prefers the O(1) `org_monthly_usage` counter (migration 032), bootstrapped
  once per org/month from `audit_log` if the row is missing.
- Billable unit = validated HTTP events (pass or fail). Normal path increments
  after a successful audit write (same spawn). Envelope path increments from
  `passed + quarantined` without writing audit (legacy short-circuit).
- Not billable: `dry_run`, self-hosted (no org), Kafka/Kinesis streaming (v1).

### Plan-limit 429 body (Phase 2)

```json
{
  "error": "plan_limit_exceeded",
  "detail": "Plan event limit exceeded for plan 'free' (1000000/1000000 in 2026-07)",
  "plan": "free",
  "limit": 1000000,
  "used": 1000000,
  "period": "2026-07",
  "upgrade_url": "https://app.datacontractgate.com/pricing",
  "status": 429
}
```

- Counter table: `org_monthly_usage` (migration 032). First read in a month may
  bootstrap from `audit_log` once; subsequent checks are PK lookups.
- A batch that *crosses* the cap while still under is allowed once; the next
  request after `used >= limit` is blocked.
- Enterprise stays unlimited (`unlimited: true`, no plan-cap 429).
