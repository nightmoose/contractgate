# RFC-082 — Exportable pilot report

**Status:** Draft (implementation landed 2026-07-15, pending merge)
**Date:** 2026-07-15
**Branch:** nightly-maintenance-2026-07-15-rfc082
**Depends on:** RFC-047 (org scoping), RFC-028 (queryability)

---

## Problem

A design partner (and their boss) needs a one-shot "here's what ContractGate
caught for you" artifact: pass rate and the violations that were blocked, for one
contract over a time window, as JSON or CSV. Today the only surfaces are `/stats`
(point-in-time aggregate, no window, no violation breakdown) and `/audit` (raw
rows). Neither is an emailable pilot report. This is dual-sell item #11 — the
"value delivered" artifact.

## Goal

One org-scoped endpoint that returns a windowed report for a contract, in JSON
(default) or CSV (downloadable). Thin v1: totals + pass rate, per-version
breakdown, and top violations. No new storage columns.

## Non-goals

- Charting / PDF (the CSV drops straight into a sheet).
- Cross-contract or org-wide rollups (per-contract is the pilot unit).
- Scheduled delivery (a follow-up could wrap this in the scheduler).

## API

```
GET /contracts/{id}/report?from=<rfc3339>&to=<rfc3339>&format=json|csv
```

- Org-scoped: `get_contract_identity(id, org)` → 404 on wrong org, 401 if no org
  in production.
- `from` / `to` optional (both, either, or neither → all-time).
- `format` = `json` (default) or `csv`. CSV returns `text/csv` +
  `Content-Disposition: attachment` (mirrors `scorecard::export_handler`).

JSON shape:

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

CSV: three labeled sections (`# TOTALS`, `# BY VERSION`, `# TOP VIOLATIONS`),
`escape_csv` per the scorecard exporter.

## Implementation

- `src/storage/report.rs` (new): `contract_report(pool, contract_id, from, to)`
  → two queries:
  1. per-version counts (`GROUP BY contract_version`, `count(*) FILTER (WHERE
     passed)` etc.), windowed.
  2. top violations: `jsonb_array_elements(violation_details)` → `field` / `kind`
     → count, `WHERE NOT passed`, windowed, `LIMIT 20`.
  Totals are summed from (1) in the handler.
- `src/report.rs` (new): `report_handler` + JSON DTO + CSV builder.
- `src/main.rs`: `GET /contracts/{id}/report` on the protected router.
- `docs/pilot-report-reference.md`; STATUS + MAINTENANCE_LOG.

Uses the existing `idx_audit_contract_time (contract_id, created_at)` and the
partial violations index, so windowed reads stay cheap. Validation hot path
untouched.

## Testing

- Serde: JSON DTO field names locked.
- DB-backed (`migrations-check`): seed a contract with mixed pass/fail audit rows
  across two versions + a window; assert totals, `pass_rate`, per-version split,
  top-violation counts, and org isolation (other org's contract → 404).
- `cargo check && cargo test && cargo clippy --all-targets -- -D warnings`.
