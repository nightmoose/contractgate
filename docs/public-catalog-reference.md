# Public Data Catalog — API Reference

**RFC:** 034  
**Status:** Accepted  
**Added:** 2026-05-24  
**Plan:** Browse and fork — Free; Export — Free

---

## Overview

The Public Catalog is a curated collection of government and open-data sources
that have pre-built DataContractGate contracts. Any user can browse the catalog and
inspect the YAML contract for each source. Authenticated users can fork a
catalog entry into their own org as an editable contract, or export the live
upstream data filtered through their fork's rules as a CSV file.

---

## Endpoints

### `GET /public-contracts`

List all catalog entries. No authentication required.

**Response `200 OK`:**

```json
[
  {
    "id": "a1b2c3d4-...",
    "name": "ACS 5-Year Population Estimates",
    "description": "US Census ACS 5-Year estimates by state",
    "source_format": "json_rows",
    "version": "2021",
    "created_at": "2026-05-15T00:00:00Z",
    "updated_at": "2026-05-15T00:00:00Z"
  }
]
```

Results are sorted by `name` ascending. `source_url` and `contract_yaml` are
not included in the list response; use the detail endpoint.

---

### `GET /public-contracts/{id}`

Get a single catalog entry including the full YAML contract. No authentication
required.

**Path parameter:** `id` — UUID of the public contract.

**Response `200 OK`:**

```json
{
  "id": "a1b2c3d4-...",
  "name": "ACS 5-Year Population Estimates",
  "description": "US Census ACS 5-Year estimates by state",
  "source_url": "https://api.census.gov/data/2021/acs/acs5?get=NAME,B01001_001E&for=state:*",
  "source_format": "json_rows",
  "contract_yaml": "version: \"1.0\"\nname: acs_population\n...",
  "version": "2021",
  "created_at": "2026-05-15T00:00:00Z",
  "updated_at": "2026-05-15T00:00:00Z"
}
```

**Response `404 Not Found`:** no catalog entry with this ID.

**Response `429 Too Many Requests`:** the caller's address exceeded the public per-IP limit (30 requests/second, burst 60). The response includes `Retry-After`. `GET /public-contracts/{id}/yaml` and `GET /public-contracts/{id}/odcs` share that limit. A request whose `X-DCG-SSR-Token` matches `CONTRACTGATE_TRUSTED_SSR_TOKEN` skips it. Successful anonymous responses are cached for 60 seconds.

---

### `POST /contracts/{id}/fork`

Fork a public catalog entry into the authenticated user's org. A new `contracts`
row is created with a draft version `1.0.0` containing a copy of the catalog
entry's YAML. The fork records a `parent_public_contract_id` reference and an
optional `fork_filter`.

**Auth:** required (`x-api-key` header).

**Path parameter:** `id` — UUID of the public contract to fork (from `GET /public-contracts`).

**Request body:**

```json
{
  "name": "my_census_contract",
  "description": "Population data filtered to Northeast states",
  "fork_filter": {
    "include_columns": ["NAME", "B01001_001E"],
    "where": {
      "field": "state",
      "op": "in",
      "values": ["09", "23", "25", "33", "44", "50"]
    }
  }
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | Yes | Name for the new contract in the caller's org. |
| `description` | string | No | Optional description. |
| `fork_filter` | object | No | Column projection and row filter applied at export time. See [Fork Filter](#fork-filter) below. |

**Response `201 Created`:**

```json
{
  "contract_id": "f9e8d7c6-...",
  "name": "my_census_contract",
  "parent_public_contract_id": "a1b2c3d4-...",
  "fork_filter": { ... },
  "created_at": "2026-05-20T14:00:00Z"
}
```

The returned `contract_id` is the UUID of the new contract in the caller's org.
Use it with the standard contracts API to promote, edit versions, or set up ingest.

Hosted Free allows 3 non-deleted contracts. A fork that would pass that cap returns `402` `plan_limit` and writes nothing.

---

### `POST /public-contract-families/{id}/fork`

Fork a whole public catalog family into the caller's org. `{id}` is the family UUID or slug (`stripe-charges-pipeline`, `shopify-orders-pipeline`, `ga4-events-pipeline`).

One transaction creates one private contract per link — the same mechanics as `POST /contracts/{id}/fork`, including `parent_public_contract_id` — and one private family with the same chain and boundaries. The body may be `{}`. Optional `name` renames the private family.

If the Free contract cap would be exceeded, the response is `402` `plan_limit` and nothing is written. See [contract-families-reference.md](contract-families-reference.md).

---

### `POST /contracts/{id}/export`

Fetch the live upstream data source, apply the fork's filter, and return the
result as a CSV. The contract ID must belong to a fork (a contract with a
`parent_public_contract_id`).

**Auth:** required (`x-api-key` header).

**Path parameter:** `id` — UUID of the forked contract in the caller's org.

**Query parameters:**

| Parameter | Values | Default | Description |
|---|---|---|---|
| `format` | `csv` | `csv` | Output format. `parquet` is reserved but returns `501 Not Implemented`. |

**Response `200 OK`:**

```
Content-Type: text/csv; charset=utf-8
Content-Disposition: attachment; filename="export.csv"
```

Body: RFC 4180 CSV with a header row, up to 100 000 rows.

**Response `400 Bad Request`:** contract is not a fork, or an unsupported format was requested.

**Response `501 Not Implemented`:** `format=parquet` was requested (not yet implemented).

---

## Source formats

| `source_format` | Upstream shape | Notes |
|---|---|---|
| `json_rows` | `[["col1","col2",…],[val,val,…],…]` | Census API default. First element is the header row. |
| `json` | `[{"col": val}, …]` | Standard array of objects. |
| `csv` | Delimited text | Delimiter auto-detected (comma, tab, semicolon). |

---

## Fork Filter

The `fork_filter` object controls column projection and row filtering applied
when exporting data. It is stored on the `contracts` row as JSONB and applied
at export time against the live upstream fetch.

```json
{
  "include_columns": ["col_a", "col_b"],
  "where": {
    "field": "state",
    "op": "in",
    "values": ["09", "25"]
  }
}
```

Both `include_columns` and `where` are optional. Omitting `fork_filter`
entirely returns all columns and all rows.

---

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `UPSTREAM_TIMEOUT_MS` | `5000` | Timeout in milliseconds for fetching the upstream source URL during export. |

---

## Limits

| Limit | Value |
|---|---|
| Max upstream response body | 50 MB |
| Max exported rows | 100 000 |

---

## Database objects

Migration `022_public_catalog.sql` adds the table. Migration `047_public_catalog_expansion.sql`
publishes the 12 launch rows. Migration `048_public_catalog_seed_refresh.sql` upserts
those same slugs from `catalog/seeds/` when a seed contract, sample, or citation
changes. 047 is not rewritten after it has been applied. Migration
`050_contract_families.sql` adds contract families and seeds the Stripe charges
pipeline (`stripe-charges-raw` → `stripe-charges-staged` → `revenue-daily-mart` →
`partner-payout-feed`) as the public family `stripe-charges-pipeline`. Those four
rows are in addition to the 12 launch slugs, including the existing `stripe-charges`
contract. `052_shopify_and_ga4_contract_families.sql` adds `shopify-orders-pipeline`
(raw CSV → typed order → fulfillment notice) and `ga4-events-pipeline` (raw event →
cleaned event → audience export). Those six rows are in addition to `shopify-orders`
and `ga4-bigquery-events`. 050 is not rewritten. See [contract-families-reference.md](contract-families-reference.md).

`POST /public-contract-families/{id}/fork` copies a family into the caller's org.

`022_public_catalog.sql` adds:

| Object | Type | Notes |
|---|---|---|
| `public_contracts` | TABLE | Curated catalog entries (id, name, source_url, source_format, contract_yaml, version) |
| `contracts.parent_public_contract_id` | column | UUID FK → `public_contracts.id`; NULL for non-forks |
| `contracts.fork_filter` | column | JSONB; NULL when no filter |
| `public_contract_families` | TABLE | A named catalog chain (migration 050) |
| `public_contract_family_links` | TABLE | `derived_from_public_contract_id`, `boundary` (`ingest`, `stage`, `egress`), `position` |

---

## Edge cases

- **Fork with no filter.** Exporting a fork with no `fork_filter` returns all
  upstream rows and columns (up to the 100 000 row cap).
- **Upstream changes schema.** The fork's YAML contract was derived from the
  catalog entry at fork time. If the upstream source adds or removes columns,
  the contract becomes stale. Re-infer from the URL or edit the YAML manually.
- **Parquet export.** Requesting `format=parquet` returns `501 Not Implemented`.
  Supply `format=csv` or omit the parameter (CSV is the default).
- **Non-fork contract.** Calling `/export` on a contract that is not a fork
  returns `400 Bad Request: "contract is not a fork of a public data source"`.

---

## Live smoke test

`scripts/catalog_live_smoke.py` is a read-only check of the production catalog.
Run it after a catalog seed migration deploys:

```bash
cargo build --bin contractgate
python3 scripts/catalog_live_smoke.py --wait 1800
```

It waits up to `--wait` seconds for the ACS row to show `B19013_001M`. Then it
checks four things. All 12 slugs return 200 (JSON, `/yaml`, `/odcs`, and the
app's `/catalog/<slug>` page). The catalog review fixes are live. Every
published sample passes `contractgate test`. Live rows match `catalog/seeds`.
Flags: `--api`, `--app`, `--cg`, `--repo`, `--interval`. Exit code 1 on any
failure. Needs PyYAML.

---

## Related

- [RFC-034](rfcs/034-public-data-source-contracts.md) — design rationale.
- [csv-inference-reference.md](csv-inference-reference.md) — infer a contract from your own CSV.
- [url-inference-reference.md](url-inference-reference.md) — infer a contract from a live URL.
