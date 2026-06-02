# RFC-038: MRI API Contracts for Findigs Integration

**Status:** Draft  
**Date:** 2026-05-16  
**Author:** Alex Suarez  

---

## Problem

Findigs uses the MRI MIX REST API (and other sources) to exchange property, unit,
and tenancy data. ContractGate has no contracts covering MRI's response shapes today.
Before the demo next week, we need:

1. At least two production-quality contracts that gate MRI API responses.
2. A test harness that can fire synthetic MRI-shaped payloads against the ingest
   endpoint so the demo shows real violations being caught and quarantined.

MRI responses arrive as a batch envelope — `{ success, data: [...], pagination }` —
not as individual records. The existing streaming contracts validate per-record;
this RFC also introduces the `envelope` stanza so contracts can declare how to
unwrap a batch before per-record validation runs.

---

## Goals

1. Define the `envelope` stanza in the contract YAML spec — optional, backward-compat.
2. Ship `mri_property_listing` contract — gates `GET /properties/{id}/units` batch responses.
3. Ship `mri_tenancy_event` contract — gates `GET /tenancies/{id}` batch responses.
4. Add fixture files + a `scripts/demo_mri.sh` smoke script that posts valid and
   invalid payloads and shows pass / quarantine split in the dashboard.
5. Both contracts multi-currency from day one (USD-first).

---

## Non-Goals

- Live MRI credentials in CI (sandbox only; secrets stay in Supabase Vault).
- Full MRI API coverage (properties, financials, work orders, documents deferred).
- MRI OAuth flow — Basic Auth is already supported by the ingest route.
- Changes to the streaming per-record contracts already in production (envelope stanza is additive/opt-in).

---

## Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Which endpoints to contract-gate first | `GET /units` (property listing) and `GET /tenancies/{id}` — highest Findigs data volume |
| D2 | Envelope handling | `envelope` stanza in contract YAML (Option A). Engine unwraps `records_path` key, validates each record against `ontology`, returns batch result with per-record violation indices. Opt-in — existing contracts unaffected. |
| D3 | Envelope wrapper validation | `validate_wrapper: true` in both MRI contracts. Checks `success: bool` is present and `pagination` shape is valid alongside record validation. |
| D4 | Contract naming | `mri_property_listing` and `mri_tenancy_event` — matches Findigs internal topic names |
| D5 | Currency | Multi-currency from day one: `["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]`. No GBP bias. |
| D6 | Demo fixture source | Synthetic JSON that mirrors real MRI shapes including envelope; no live MRI server needed for demo |
| D7 | Violation demo | Omit `unit_id` (required) in one fixture → expect quarantine with row index; all-valid fixture → expect pass |
| D8 | Batch ingest response shape | Engine returns `{ passed: N, quarantined: N, violations: [{ record_index, field, reason }] }` for envelope contracts |

---

## Contracts

### Contract 1: `mri_property_listing`

Gates the unit array returned inside `data` from `GET /api/v1/properties/{propertyId}/units`.

```yaml
version: "1.0"
name: "mri_property_listing"
description: "Contract for MRI MIX API property unit listing responses (Findigs integration). Validates the raw MRI envelope from GET /api/v1/properties/{propertyId}/units."

envelope:
  records_path: data          # unwrap data[] before per-record validation
  validate_wrapper: true      # also assert success:bool + pagination shape

ontology:
  entities:
    - name: unit_id
      type: string
      required: true
      pattern: "^[A-Za-z0-9_-]+$"
    - name: property_id
      type: string
      required: true
    - name: unit_number
      type: string
      required: true
    - name: bedrooms
      type: integer
      required: true
      min: 0
      max: 20
    - name: bathrooms
      type: number
      required: true
      min: 0
    - name: rent_amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]
    - name: status
      type: string
      required: true
      enum: ["available", "occupied", "maintenance", "reserved"]
    - name: floor_area_sqft
      type: number
      required: false
      min: 0
    - name: available_from
      type: string
      required: false
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"

glossary:
  - field: unit_id
    description: "MRI internal unit identifier"
    constraints: "alphanumeric, hyphens, underscores only"
  - field: rent_amount
    description: "Monthly rent in the specified currency"
    constraints: "must be non-negative"
  - field: currency
    description: "ISO 4217 currency code"
    constraints: "one of: USD, EUR, GBP, CAD, AUD, MXN"
  - field: status
    description: "Current occupancy status of the unit"
    constraints: "one of: available, occupied, maintenance, reserved"

metrics:
  - name: avg_rent
    formula: "avg(rent_amount)"
  - name: available_units
    formula: "count(*) where status = 'available'"
  - name: occupied_rate
    formula: "count(*) where status = 'occupied' / count(*)"
```

### Contract 2: `mri_tenancy_event`

Gates the tenancy object returned in `data` from `GET /api/v1/tenancies/{tenancyId}`.

```yaml
version: "1.0"
name: "mri_tenancy_event"
description: "Contract for MRI MIX API tenancy records (Findigs integration). Validates the raw MRI envelope from GET /api/v1/tenancies/{tenancyId}."

envelope:
  records_path: data          # unwrap data[] before per-record validation
  validate_wrapper: true      # also assert success:bool + pagination shape

ontology:
  entities:
    - name: tenancy_id
      type: string
      required: true
      pattern: "^[A-Za-z0-9_-]+$"
    - name: unit_id
      type: string
      required: true
    - name: property_id
      type: string
      required: true
    - name: tenant_contact_id
      type: string
      required: true
    - name: start_date
      type: string
      required: true
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"
    - name: end_date
      type: string
      required: false
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"
    - name: rent_amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]
    - name: status
      type: string
      required: true
      enum: ["active", "pending", "ended", "terminated"]
    - name: deposit_amount
      type: number
      required: false
      min: 0
    - name: payment_frequency
      type: string
      required: false
      enum: ["weekly", "monthly", "quarterly", "annually"]

glossary:
  - field: tenancy_id
    description: "MRI internal tenancy identifier"
    constraints: "alphanumeric, hyphens, underscores only"
  - field: start_date
    description: "Tenancy commencement date (ISO 8601)"
    constraints: "required; must precede end_date if provided"
  - field: currency
    description: "ISO 4217 currency code"
    constraints: "one of: USD, EUR, GBP, CAD, AUD, MXN"
  - field: status
    description: "Lifecycle state of the tenancy"
    constraints: "one of: active, pending, ended, terminated"
  - field: deposit_amount
    description: "Security deposit held in currency units"
    constraints: "must be non-negative"

metrics:
  - name: active_tenancies
    formula: "count(*) where status = 'active'"
  - name: avg_rent_active
    formula: "avg(rent_amount) where status = 'active'"
  - name: avg_deposit_ratio
    formula: "avg(deposit_amount / rent_amount) where deposit_amount is not null"
```

---

## Demo Test Harness

### Fixture files

All fixtures include the full MRI envelope (`success`, `data`, `pagination`).

**`scripts/fixtures/mri_property_listing_valid.json`**
```json
{
  "success": true,
  "data": [
    { "unit_id": "U-001", "property_id": "P-100", "unit_number": "1A", "bedrooms": 2, "bathrooms": 1.0, "rent_amount": 2400.00, "currency": "USD", "status": "available" },
    { "unit_id": "U-002", "property_id": "P-100", "unit_number": "2B", "bedrooms": 1, "bathrooms": 1.0, "rent_amount": 1800.00, "currency": "USD", "status": "occupied" },
    { "unit_id": "U-003", "property_id": "P-100", "unit_number": "3C", "bedrooms": 3, "bathrooms": 2.0, "rent_amount": 3200.00, "currency": "USD", "status": "reserved" }
  ],
  "pagination": { "page": 1, "limit": 50, "total": 3, "hasMore": false }
}
```

**`scripts/fixtures/mri_property_listing_invalid.json`** — record 2 missing `unit_id` → quarantine at index 1:
```json
{
  "success": true,
  "data": [
    { "unit_id": "U-001", "property_id": "P-100", "unit_number": "1A", "bedrooms": 2, "bathrooms": 1.0, "rent_amount": 2400.00, "currency": "USD", "status": "available" },
    { "property_id": "P-100", "unit_number": "2B", "bedrooms": 1, "bathrooms": 1.0, "rent_amount": 1800.00, "currency": "USD", "status": "occupied" }
  ],
  "pagination": { "page": 1, "limit": 50, "total": 2, "hasMore": false }
}
```

**`scripts/fixtures/mri_tenancy_event_valid.json`**
```json
{
  "success": true,
  "data": [
    { "tenancy_id": "T-001", "unit_id": "U-001", "property_id": "P-100", "tenant_contact_id": "C-500", "start_date": "2025-01-01", "rent_amount": 2400.00, "currency": "USD", "status": "active", "deposit_amount": 4800.00, "payment_frequency": "monthly" }
  ],
  "pagination": { "page": 1, "limit": 50, "total": 1, "hasMore": false }
}
```

**`scripts/fixtures/mri_tenancy_event_invalid.json`** — `status = "evicted"` (not in enum) → quarantine at index 0:
```json
{
  "success": true,
  "data": [
    { "tenancy_id": "T-002", "unit_id": "U-002", "property_id": "P-100", "tenant_contact_id": "C-501", "start_date": "2024-06-01", "rent_amount": 1800.00, "currency": "USD", "status": "evicted", "payment_frequency": "monthly" }
  ],
  "pagination": { "page": 1, "limit": 50, "total": 1, "hasMore": false }
}
```

### Smoke script

`scripts/demo_mri.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

BASE="${CONTRACTGATE_BASE_URL:-http://localhost:3001}"
KEY="${CONTRACTGATE_API_KEY:-demo-key}"
DIR="$(dirname "$0")/fixtures"

echo "=== MRI Demo: property listing ==="
echo "-- valid payload (expect PASS) --"
curl -s -X POST "$BASE/api/v1/ingest/mri_property_listing" \
  -H "x-api-key: $KEY" \
  -H "Content-Type: application/json" \
  -d @"$DIR/mri_property_listing_valid.json" | jq .

echo "-- invalid payload (expect QUARANTINE: missing unit_id) --"
curl -s -X POST "$BASE/api/v1/ingest/mri_property_listing" \
  -H "x-api-key: $KEY" \
  -H "Content-Type: application/json" \
  -d @"$DIR/mri_property_listing_invalid.json" | jq .

echo ""
echo "=== MRI Demo: tenancy event ==="
echo "-- valid payload (expect PASS) --"
curl -s -X POST "$BASE/api/v1/ingest/mri_tenancy_event" \
  -H "x-api-key: $KEY" \
  -H "Content-Type: application/json" \
  -d @"$DIR/mri_tenancy_event_valid.json" | jq .

echo "-- invalid payload (expect QUARANTINE: bad status enum) --"
curl -s -X POST "$BASE/api/v1/ingest/mri_tenancy_event" \
  -H "x-api-key: $KEY" \
  -H "Content-Type: application/json" \
  -d @"$DIR/mri_tenancy_event_invalid.json" | jq .
```

---

## Implementation Plan

| Step | What | Notes |
|------|------|-------|
| 1 | Implement `envelope` stanza parsing in the validation engine | Read `records_path` from contract YAML; unwrap before per-record validation. Reuse existing envelope-detection logic from `infer_url.rs`. |
| 2 | Extend batch ingest response to include `{ passed, quarantined, violations: [{ record_index, field, reason }] }` | Only when contract has `envelope` stanza. |
| 3 | Deploy `mri_property_listing` via `POST /contracts/deploy` | After step 1 lands. |
| 4 | Deploy `mri_tenancy_event` via `POST /contracts/deploy` | After step 1 lands. |
| 5 | Write 4 fixture JSON files under `scripts/fixtures/` | Inline above — copy to files. |
| 6 | Write `scripts/demo_mri.sh` smoke script | See below. |
| 7 | Run smoke script; verify pass/quarantine split in dashboard | Screenshot for demo slide. |

Steps 3-7 are gated on step 1 (envelope parsing). Steps 3-7 require no Rust changes beyond step 1.

---

## Acceptance Criteria

- [ ] `envelope` stanza parsed by validation engine; `records_path: data` unwraps correctly.
- [ ] `validate_wrapper: true` rejects payloads missing `success` or malformed `pagination`.
- [ ] Batch ingest response includes `{ passed, quarantined, violations: [{ record_index, field, reason }] }`.
- [ ] `mri_property_listing` contract deployed; `GET /contracts?name=mri_property_listing` returns it.
- [ ] `mri_tenancy_event` contract deployed; `GET /contracts?name=mri_tenancy_event` returns it.
- [ ] Valid property listing fixture → `passed: 3, quarantined: 0`.
- [ ] Invalid property listing fixture → `quarantined: 1, violations[0].record_index = 1, violations[0].field = "unit_id"`.
- [ ] Valid tenancy event fixture → `passed: 1, quarantined: 0`.
- [ ] Invalid tenancy event fixture → `quarantined: 1, violations[0].field = "status"`.
- [ ] Existing per-record contracts (no `envelope` stanza) continue to work unchanged.
- [ ] `scripts/demo_mri.sh` runs end-to-end with exit code 0.
- [ ] Dashboard shows correct pass/quarantine counts for both contracts.

---

## Resolved Questions

- **OQ1 (closed):** Findigs sends the full raw MRI `{ success, data, pagination }` envelope. Contracts must accept and validate the envelope wrapper. Resolved by the `envelope` stanza (D2/D3 above). Per-record streaming contracts are unaffected.
- **OQ2 (closed):** Multi-currency from day one. US-based company, no GBP-first assumption. Currency enum: `["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]`.
