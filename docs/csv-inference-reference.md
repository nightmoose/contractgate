# CSV Contract Inference — API Reference

**RFC:** 035  
**Status:** Accepted  
**Added:** 2026-05-24  
**Plan:** Growth+ (see [plan-gating-reference.md](plan-gating-reference.md))

---

## Overview

`POST /contracts/infer/csv` accepts a CSV document and returns a draft YAML
contract describing its column types. The same shared inference engine powers
the existing JSON inference endpoint — CSV values are coerced to JSON types
first, then the engine runs identically.

The first row of the CSV must be the header row. Up to 1 000 data rows are
sampled; additional rows are silently ignored.

---

## Endpoint

```
POST /contracts/infer/csv
```

**Auth:** not required (inference does not write to the database).

### Request body (JSON)

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | Yes | Name embedded in the generated contract's `name` field. |
| `description` | string | No | Optional description string in the generated contract. |
| `csv_content` | string | One of `csv_content` or `base64` | Raw CSV as UTF-8 text. |
| `base64` | string | One of `csv_content` or `base64` | Base64-encoded CSV for binary-safe transport. |
| `delimiter` | string | No | Override auto-detection. Accepts `","`, `";"`, `"\t"`, or `"tab"`. |

Exactly one of `csv_content` or `base64` must be provided. If both are supplied,
`csv_content` takes precedence.

### Response `200 OK`

```json
{
  "yaml_content": "version: \"1.0\"\nname: \"my_contract\"\n...",
  "field_count": 5,
  "sample_count": 847
}
```

| Field | Type | Description |
|---|---|---|
| `yaml_content` | string | Complete draft YAML contract. |
| `field_count` | integer | Number of columns inferred. |
| `sample_count` | integer | Number of data rows sampled (≤ 1 000). |

### HTTP status codes

| Status | Meaning |
|---|---|
| `200 OK` | Inference succeeded. |
| `400 Bad Request` | Missing required fields, invalid base64, empty CSV, CSV > 10 MB, unsupported delimiter, duplicate column names, or CSV parse error. |

---

## Delimiter auto-detection

When `delimiter` is not specified the endpoint sniffs the first 4 096 bytes
(up to 20 lines) and scores comma, tab, and semicolon by column-count
consistency across those lines. The delimiter with the most consistently
uniform column count wins. Tiebreak order: comma > tab > semicolon.

If no candidate delimiter appears in the sniffed region the endpoint returns
`400 Bad Request` — use the explicit `delimiter` field.

---

## Type coercion order

CSV values are strings on the wire. Each value is coerced to a JSON type before
inference runs:

| Wire value | Coerced to |
|---|---|
| Empty string or whitespace-only | `null` (treated as absent) |
| `true` or `false` (any case) | `boolean` |
| Parses as 64-bit integer | `integer` |
| Parses as 64-bit float | `float` |
| Anything else | `string` |

After coercion the inference engine applies the same pattern/enum detection
used for JSON samples: UUID detection, ISO date format, and enum collapse when
a string column has ≤ 10 distinct values across the sample.

A column is marked `required: false` if any row in the sample has an empty
(coerced-to-null) value for that column.

---

## Limits

| Limit | Value |
|---|---|
| Max CSV body | 10 MB |
| Max sampled rows | 1 000 |
| Sniff window for delimiter detection | first 4 096 bytes, first 20 lines |

---

## Examples

### Minimal — comma-separated inline

```bash
curl -X POST https://your-instance/contracts/infer/csv \
  -H "Content-Type: application/json" \
  -d '{
    "name": "user_events",
    "csv_content": "user_id,event_type,amount\nu1,purchase,49.99\nu2,click,\n"
  }'
```

```json
{
  "yaml_content": "version: \"1.0\"\nname: user_events\nontology:\n  entities:\n  - name: user_id\n    type: string\n    required: true\n  - name: event_type\n    type: string\n    required: true\n  - name: amount\n    type: float\n    required: false\n",
  "field_count": 3,
  "sample_count": 2
}
```

### Base64-encoded CSV

```bash
CSV_B64=$(echo -n "id,score\n1,9.5\n2,8.0\n" | base64)
curl -X POST https://your-instance/contracts/infer/csv \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"scores\", \"base64\": \"$CSV_B64\"}"
```

### Tab-separated with explicit delimiter

```bash
curl -X POST https://your-instance/contracts/infer/csv \
  -H "Content-Type: application/json" \
  -d '{
    "name": "tsv_data",
    "csv_content": "col_a\tcol_b\n1\thello\n2\tworld\n",
    "delimiter": "tab"
  }'
```

---

## Edge cases

- **Header row required.** The first row is always treated as column names. A
  CSV with no header row produces columns named after the first data row's
  values — use a proper header row.
- **Duplicate column names** return `400 Bad Request`.
- **All-null column.** If every value in a column is empty, the column is
  inferred as `type: string`, `required: false`, with no pattern constraint.
- **Mixed types in a column.** The inference engine picks the most common type.
  A column with 900 integers and 100 strings becomes `type: string` (widening
  to the broadest compatible type).
- **Large files.** Files over 10 MB are rejected. Trim the file to a
  representative sample before sending, or use the `base64` field with
  server-side streaming if you have a custom ingress layer.

---

## Related

- [url-inference-reference.md](url-inference-reference.md) — fetch a remote CSV or JSON endpoint and infer a contract.
- [RFC-035](rfcs/035-csv-contract-inference.md) — design rationale.
- [RFC-037](rfcs/037-api-source-contract-creation.md) — URL-based inference.
