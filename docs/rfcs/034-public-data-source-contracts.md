# RFC-034: Public Data Source Contracts + Forking

**Status:** Draft
**Date:** 2026-05-15
**Author:** Alex Suarez

---

## Problem

Customers who want to work with public data sources (government APIs, open data
portals) today must download the full dataset, filter it locally, and then
validate it against their own contract. This wastes bandwidth, adds pipeline
complexity, and pushes filtering logic outside ContractGate where it cannot be
audited.

There is no way to share a canonical schema for a well-known public source so
that multiple customers can build on it without reinventing it.

---

## Goals

1. Ship a **Public Catalog** — a curated set of ContractGate-managed contracts
   describing government / open data APIs, starting with the ACS 5-Year Census
   estimates (2023). Second candidate: NYC Taxi trip data.
2. Let any customer **fork** a public contract into their own org, customize it
   (field subset + row predicates), and **export** clean, policy-compliant data.
   ContractGate applies all rules at export time — the published contract stays
   pristine.
3. Keep the validation engine under the existing <15 ms p99 budget for
   fork-applied filtering (upstream fetch time excluded from this budget).

---

## Non-Goals

- Full SQL query layer (deferred to RFC-035+). Predicates in this RFC are
  simple field-equality and range comparisons only.
- User-submitted / community public contracts (admin-curated only for now).
- Binary or streaming protocols on upstream sources (HTTP + JSON/CSV only).
- Caching upstream responses beyond a short TTL de-dupe window.

---

## Decisions

| # | Question | Decision |
|---|---|---|
| Q1 | Who can publish public contracts? | ContractGate admins only (curated registry). Community publishing deferred. |
| Q2 | Filter depth at fork time | Field subsetting + row-level equality/range predicates. No full SQL yet. |
| Q3 | Where does filtering happen? | Gateway-side: ContractGate fetches upstream, applies fork filter, returns filtered payload to client. |
| Q4 | Fork storage | Fork is a new contract row with `parent_contract_id` + `fork_filter` JSONB. Parent version pinned at fork time; consumer can opt in to track latest. |
| Q5 | Upstream auth | Public sources only (no auth required). Authenticated upstream sources deferred. |
| Q6 | Route namespace | `/public-contracts` for registry listing; `/contracts/:id/fork` for fork creation; `/contracts/:id/export` for filtered export. |
| Q7 | Export formats | Parquet and CSV initially. Format selected via `?format=parquet\|csv` param. |
| Q8 | Publishing mechanism | Public toggle on existing sharing infra (RFC-032). No new sharing UI needed in v1. |

---

## Data Model

### `public_contracts` table (new)

```sql
create table public_contracts (
  id          uuid primary key default gen_random_uuid(),
  name        text not null,
  description text,
  source_url  text not null,          -- upstream HTTP endpoint
  source_format text not null         -- "json" | "csv"
    check (source_format in ('json','csv')),
  contract_yaml text not null,        -- canonical ContractGate YAML
  version     text not null default '1.0',
  created_at  timestamptz not null default now(),
  updated_at  timestamptz not null default now()
);
```

### `contracts` table additions

```sql
alter table contracts
  add column parent_public_contract_id uuid references public_contracts(id),
  add column fork_filter jsonb;        -- null when not a fork
```

### `fork_filter` shape (JSONB)

```jsonb
{
  "fields": ["user_id", "event_type", "timestamp"],   // field subset; null = all
  "predicates": [
    { "field": "country", "op": "eq",  "value": "US" },
    { "field": "amount",  "op": "gte", "value": 0    },
    { "field": "amount",  "op": "lte", "value": 10000 }
  ]
}
```

Supported `op` values: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`.

---

## New Routes

All routes require `x-api-key` except where noted.

### List public contracts

```
GET /public-contracts
```

Response: paginated list of public contract summaries (no auth required —
public registry is readable by anyone).

### Get public contract

```
GET /public-contracts/:id
```

Returns full YAML + source metadata.

### Fork a public contract

```
POST /contracts/:id/fork
```

`id` here is a `public_contracts.id`. Body:

```json
{
  "name": "us-census-age-filtered",
  "description": "Census data filtered to working-age population",
  "fork_filter": {
    "fields": ["state", "age", "population"],
    "predicates": [
      { "field": "age", "op": "gte", "value": 18 },
      { "field": "age", "op": "lte", "value": 65 }
    ]
  },
  "track_parent_version": false
}
```

Creates a new contract in the caller's org with `parent_public_contract_id`
set. Returns a standard `Contract` response.

### Export filtered data through a fork

```
POST /contracts/:id/export?format=csv
```

Supported formats: `csv` (default), `parquet`. `id` is the forked contract.
ContractGate:
1. Fetches upstream `source_url` from the parent public contract.
2. Parses JSON or CSV.
3. Applies `fork_filter` (field subset + predicates) in Rust — no external
   query engine.
4. Validates each resulting row against the fork's contract ontology.
5. Streams filtered, validated output in requested format + writes audit log entry.

---

## Filtering Engine (Rust)

New module: `src/fork_filter.rs`

```rust
pub struct ForkFilter {
    pub fields: Option<Vec<String>>,
    pub predicates: Vec<Predicate>,
}

pub struct Predicate {
    pub field: String,
    pub op: Op,
    pub value: serde_json::Value,
}

pub enum Op { Eq, Neq, Gt, Gte, Lt, Lte, In }

impl ForkFilter {
    /// Apply to a single JSON object row. Returns None if row is filtered out.
    pub fn apply(&self, row: &serde_json::Value) -> Option<serde_json::Value>;
}
```

Field subsetting runs after predicate evaluation to avoid retaining fields
only needed for the predicate.

---

## Performance

- Filtering is pure in-memory Rust — no DB round-trip per row.
- Upstream fetch is bounded by a configurable `UPSTREAM_TIMEOUT_MS` env var
  (default 5000 ms). The upstream fetch time is excluded from the <15 ms
  validation p99 budget; only the filtering + validation step is measured.
- Upstream responses over `MAX_UPSTREAM_BYTES` (default 50 MB) are rejected
  with `413`.

---

## Audit Log

Fork fetch events are logged to `audit_logs` with:
- `event_type = "fork_fetch"`
- `contract_id` = the fork contract id
- `metadata` JSONB includes `{ rows_fetched, rows_after_filter, parent_id }`.

---

## Seeded Dataset: ACS 5-Year Estimates (2023)

The first entry in the Public Catalog at launch. Source:
`https://api.census.gov/data/2023/acs/acs5` (US Census Bureau — public, no
auth required).

**Core fields (required in published contract):**

| Field | Type | Notes |
|---|---|---|
| `state` | string | State name |
| `state_fips` | string | 2-digit FIPS code |
| `county` | string | County name |
| `county_fips` | string | 3-digit FIPS code |
| `geoid` | string | Concatenated FIPS for join keys |
| `year` | integer | Vintage year (2023) |
| `population` | integer | Total population estimate |
| `median_household_income` | number | USD |
| `income_margin_of_error` | number | USD — optional, `required: false` |

**Contract settings:** `egress_leakage_mode: off`. No PII on core fields.
Good descriptions + quality rules in YAML (non-negative population,
income ≥ 0).

Second candidate dataset (v1 stretch / v2): NYC Taxi trip data (TLC).

---

## Migration

`017-public-contracts.sql`:
1. Create `public_contracts`.
2. Alter `contracts` to add `parent_public_contract_id` + `fork_filter`.
3. Add index on `contracts(parent_public_contract_id)`.

---

## Open Questions

- **OQ1:** Should forked contracts auto-update when the parent's `contract_yaml`
  changes if `track_parent_version = true`? Leaning yes with an explicit
  re-validation step, but deferred.
- **OQ2:** Rate-limiting upstream fetches per org to prevent abuse. Simple
  per-org quota (reuse existing quota infra) is the likely answer.
- **OQ3:** Caching upstream responses with a short TTL (e.g. 60 s) to avoid
  hammering public APIs. Deferred but reserved in the fetch handler.

---

## Acceptance Criteria

- [ ] `GET /public-contracts` returns seeded list including ACS 5-Year.
- [ ] `POST /contracts/:id/fork` creates a contract with `parent_public_contract_id` set.
- [ ] `POST /contracts/:id/export` returns filtered + validated data in CSV and Parquet.
- [ ] Field subsetting and all predicate `op` types covered by unit tests.
- [ ] Filtering + validation p99 < 15 ms on 10k-row payload (bench target, upstream fetch excluded).
- [ ] Published contract immutable — fork edits do not mutate parent.
- [ ] Public toggle works via existing sharing infra (no new sharing UI).
- [ ] Migration 017 runs clean on fresh DB.
- [ ] Audit log entry written for every export.
