# Public Data Catalog — API Reference

**RFC:** 034  
**Status:** Accepted  
**Added:** 2026-05-24  
**Plan:** Browse and fork — Free; Export — Free

---

## Overview

The Public Catalog is a curated collection of government and open-data sources
that have pre-built ContractGate contracts. Any user can browse the catalog and
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

Migration `022_public_catalog.sql` adds:

| Object | Type | Notes |
|---|---|---|
| `public_contracts` | TABLE | Curated catalog entries (id, name, source_url, source_format, contract_yaml, version) |
| `contracts.parent_public_contract_id` | column | UUID FK → `public_contracts.id`; NULL for non-forks |
| `contracts.fork_filter` | column | JSONB; NULL when no filter |

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

## Related

- [RFC-034](rfcs/034-public-data-source-contracts.md) — design rationale.
- [csv-inference-reference.md](csv-inference-reference.md) — infer a contract from your own CSV.
- [url-inference-reference.md](url-inference-reference.md) — infer a contract from a live URL.
