# Provider Data-Quality Scorecard — API Reference

**RFC:** 031  
**Status:** Accepted  
**Added:** 2026-05-15

---

## Overview

The scorecard surfaces per-provider data-quality metrics aggregated from existing audit and quarantine data. No new writes touch the ingest path — the `<15ms p99` budget is untouched.

`source` is the provider identifier set when a contract version is deployed (the `source` field in `POST /contracts/deploy` or on a `contract_versions` row). Contracts with no `source` are binned under `(unsourced)`.

---

## Endpoints

### `GET /scorecard/{source}`

Returns the full scorecard for a provider as JSON.

**Path parameter:** `source` — the provider source name (URL-encoded if it contains spaces).

**Response `200 OK`:**

```json
{
  "source": "acme-pms",
  "summary": [
    {
      "source": "acme-pms",
      "contract_name": "rental_listings",
      "total_events": 12043,
      "passed": 11120,
      "quarantined": 923,
      "quarantine_pct": 7.66
    }
  ],
  "top_violations": [
    {
      "source": "acme-pms",
      "contract_name": "rental_listings",
      "field": "monthly_rent",
      "code": "range_violation",
      "violations": 541
    }
  ],
  "field_health": [ ... ],
  "drift_signals": [
    {
      "source": "acme-pms",
      "contract_name": "rental_listings",
      "field": "monthly_rent",
      "current_rate": 0.12,
      "baseline_rate": 0.0,
      "delta_pct": 12.0,
      "label": "↑ 12.0 pp since baseline"
    }
  ]
}
```

**Fields:**

| Field | Type | Description |
|---|---|---|
| `summary` | array | Per-contract pass/quarantine totals. `quarantine_pct` is `null` when `total_events = 0`. |
| `top_violations` | array | Up to 20 field-level violations ranked by count. |
| `field_health` | array | Full field-level breakdown (superset of `top_violations`). |
| `drift_signals` | array | Fields whose 24h violation rate deviates from the 30-day baseline by more than 5 percentage points. Empty until the baseline rollup job has run at least once. |

**Violation `code` values** map to `ViolationKind`:

| code | meaning |
|---|---|
| `missing_required_field` | Required field absent from event |
| `type_mismatch` | Field present but wrong JSON type |
| `pattern_mismatch` | String didn't match `pattern` regex |
| `enum_violation` | Value not in declared `enum` |
| `range_violation` | Numeric outside `min`/`max` bounds |
| `length_violation` | String outside `min_length`/`max_length` |
| `metric_range_violation` | Computed metric outside declared range |

---

### `GET /scorecard/{source}/drift`

Returns only the active drift signals for a provider.

**Response `200 OK`:** array of drift signal objects (same shape as `drift_signals` in the full scorecard).

Returns `[]` when no baseline has been seeded or no fields have drifted beyond the threshold.

---

### `GET /scorecard/{source}/export?format=csv`

Returns a flat CSV the provider can open without a ContractGate account.

**Query parameter:** `format` — only `csv` is supported in v1 (default: `csv`).

**Response `200 OK`:**

```
Content-Type: text/csv; charset=utf-8
Content-Disposition: attachment; filename="scorecard-acme-pms-2026-05-15.csv"
```

The CSV has two sections separated by a blank line:

```
# SCORECARD SUMMARY
source,contract_name,total_events,passed,quarantined,quarantine_pct
acme-pms,rental_listings,12043,11120,923,7.66

# FIELD VIOLATIONS
source,contract_name,field,violation_code,violation_count
acme-pms,rental_listings,monthly_rent,range_violation,541
acme-pms,rental_listings,unit_id,missing_required_field,382
```

Values containing commas, double-quotes, or newlines are wrapped in double-quotes per RFC 4180.

---

## Drift Detection

Drift signals compare a field's **24-hour violation rate** to its **30-day trailing baseline**.

- **Threshold:** 5 percentage points (absolute). A field whose rate moves from 0 % → 12 % fires; one that moves from 2 % → 5 % is suppressed.
- **Baseline window:** trailing 30 days; current window: last 24 hours.
- **Baseline source:** the `provider_field_baseline` table, populated by the daily rollup job.
- **Direction:** both increases (more violations) and decreases (improvement) fire so operators catch both degradation and unexpected schema changes.

---

## Daily Baseline Rollup Job

The drift detector requires a pre-computed baseline. Run the rollup once daily:

```bash
# via cron or CI
cargo run -- scorecard-rollup
```

Or set a cron entry:

```
0 2 * * * cd /opt/contractgate && DATABASE_URL=... cargo run -- scorecard-rollup
```

The job is **idempotent**: re-running for the same date updates existing rows rather than duplicating them. It reads only from `quarantine_events` and `audit_log` — no ingest writes are touched.

---

## Database Objects

Migration `019_provider_scorecard.sql` adds:

| Object | Type | Purpose |
|---|---|---|
| `provider_scorecard` | VIEW | Per-provider, per-contract pass/quarantine summary |
| `provider_field_health` | VIEW | Per-provider, per-field violation breakdown |
| `provider_field_baseline` | TABLE | Rolling 30-day baseline for drift detection |

The views read from `audit_log`, `quarantine_events`, `contracts`, and `contract_versions`. They inherit the RLS policies of their underlying tables.

---

## Authentication

All scorecard endpoints require a valid `x-api-key` header (same as every other protected route).

---

## Integration with RFC-033

RFC-033 (Provider–Consumer Collaboration) will add scoped read access so a provider can see *its own* scorecard without org-level access. Until RFC-033 ships, scorecard access requires a full org API key.
