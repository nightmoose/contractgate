# Pilot Report Reference

**RFC:** 082 — Exportable pilot report  
**Status:** Shipped  
**Since:** nightly-2026-07-15

An org-scoped, windowed **"here's what ContractGate caught for you"** report for a
single contract: pass rate, per-version breakdown, and the top violations. JSON
(default) or a downloadable CSV — the artifact a design partner forwards to their
boss after a 2-week pilot.

**When to use it**

- End of a design-partner pilot (pair with `demo/hero` walkthrough)
- Weekly quality snapshot for a production contract
- Attach to sales follow-ups as measured value (not a deck claim)

---

## Endpoint

```
GET /contracts/{id}/report?from=<rfc3339>&to=<rfc3339>&format=json|csv
```

Auth: standard org-scoped auth (API key or Bearer JWT). A contract that isn't the
caller's org returns **404**; no resolvable org in production returns **401**.

### Query parameters

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `from` | RFC 3339 timestamp | — | Window start (inclusive). Omit for all-time. |
| `to` | RFC 3339 timestamp | — | Window end (inclusive). Omit for all-time. |
| `format` | `json` \| `csv` | `json` | `csv` returns `text/csv` + `Content-Disposition: attachment`. |

### JSON response

```json
{
  "contract_id": "uuid",
  "contract_name": "hero_events",
  "window": { "from": null, "to": null },
  "generated_at": "2026-07-15T12:00:00Z",
  "totals": { "total": 1500, "passed": 1440, "quarantined": 60, "pass_rate": 0.96 },
  "by_version": [
    { "contract_version": "1.1.0", "total": 1200, "passed": 1200, "quarantined": 0 },
    { "contract_version": "1.0.0", "total": 300,  "passed": 240,  "quarantined": 60 }
  ],
  "top_violations": [
    { "field": "method", "kind": "invalid_enum", "count": 60 }
  ]
}
```

- `pass_rate` is `passed / total` (0.0 when the window is empty).
- `by_version` is ordered by event count, newest-heaviest first.
- `top_violations` aggregates the `violation_details` of quarantined events by
  `(field, kind)`, top 20.

### CSV response

Three labeled sections, RFC 4180 quoting:

```
# TOTALS
contract_name,contract_id,from,to,total,passed,quarantined,pass_rate
hero_events,<uuid>,,,1500,1440,60,0.9600

# BY VERSION
contract_version,total,passed,quarantined
1.1.0,1200,1200,0
1.0.0,300,240,60

# TOP VIOLATIONS
field,kind,count
method,invalid_enum,60
```

---

## Examples

```bash
# JSON, all-time
curl -H "x-api-key: $KEY" \
  "https://contractgate-api.fly.dev/contracts/$CID/report"

# CSV for a pilot window, saved to a file
curl -H "x-api-key: $KEY" \
  "https://contractgate-api.fly.dev/contracts/$CID/report?from=2026-07-01T00:00:00Z&to=2026-07-31T23:59:59Z&format=csv" \
  -o pilot-report.csv
```

---

## Notes

- Backed by `idx_audit_contract_time (contract_id, created_at)` and the partial
  violations index, so windowed reads stay cheap.
- Point-in-time only; there is no scheduled delivery in v1 (a scheduled task
  could wrap this endpoint).
