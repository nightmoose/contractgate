# Strict ODCS export / import (feat/odcs-strict)

**Status:** current behaviour, 2026-10-15. Where this page conflicts with
`field-mapping.md` or `gap-analysis.md`, this page wins.

DCG exports (`GET /contracts/{id}/versions/{v}/export`) validate against the
**official ODCS v3.1.0 JSON Schema** with no post-processing. The schema is
vendored at `tests/fixtures/odcs-schema/` (Apache-2.0) and checked in
`src/tests.rs` (`odcs_strict_tests`). It is byte-identical to the copy CsvPath
Framework bundles.

## Export shape

| CG | ODCS (native) | DCG extension (authoritative on re-import) |
|---|---|---|
| identity description | `description: {purpose: ...}` | — |
| identity name | `name`, `dataProduct`, `schema[0].name` | — |
| `type: float` / `number` | `logicalType: number` | — |
| `type: any` | `logicalType` omitted | `x-cg-type: any` |
| `type: date` | `logicalType: date`, `logicalTypeOptions.format: yyyy-MM-dd` | — |
| `pattern` (string) | `logicalTypeOptions.pattern` | `x-cg-pattern` |
| `min_length` / `max_length` | `logicalTypeOptions.minLength` / `maxLength` | `x-cg-min-length` / `x-cg-max-length` |
| `min` / `max` (number/integer) | `logicalTypeOptions.minimum` / `maximum` | `x-cg-min` / `x-cg-max` |
| `format` (new, string) | `logicalTypeOptions.format` | `x-cg-format` |
| `enum` | `quality: [{type: library, metric: invalidValues, mustBe: 0, arguments: {validValues: [...]}}]` (v3.1.0 has no property `enum`), tagged `x-cg-derived: enum` | `x-cg-enum` |
| `required` | `required` | — |
| quality `uniqueness` | `unique: true` + `library` / `duplicateValues` / `dimension: uniqueness` | `x-cg-quality` on the entry |
| quality `completeness` (threshold t) | `library` / `missingValues`; `mustBe: 0`, or `mustBeLessOrEqualTo: (1-t)·100`, `unit: percent` | `x-cg-quality` |
| quality `validity` | `library` / `invalidValues` with `validValues` / `pattern` from the field | `x-cg-quality` |
| quality `freshness` | `type: custom`, `engine: contractgate`, `implementation: "freshness maxAgeSeconds=N"` | `x-cg-quality` |
| PII transform | `classification: pii` | `x-cg-transform-kind` / `-style` |
| `x-contractgate-version`, `-ontology`, `-glossary`, `-metrics`, `-compliance-mode` | — | entries in the **root `customProperties[]`** array (the schema forbids unknown root keys) |

Nested objects keep the D-001 shape: flattened to dot-notation leaf names
(`customer.address.country`). Array element schemas are nested natively under
`items` / `items.properties`.

## Import (`POST /contracts/import`)

* **Mode A (lossless):** `x-contractgate-version` is found in the root
  `customProperties[]` **or** as a bare root key (exports before this change).
  `x-contractgate-ontology` is the contract. A string value (YAML/JSON text, as
  produced for OpenMetadata) is parsed too.
* **Mode B (foreign / stripped):** native fields are read first:
  `logicalTypeOptions` (pattern, minLength, maxLength, minimum, maximum, format),
  ODCS v3.2 `enum` (`- value: x` or scalars), `invalidValues` `validValues` /
  `pattern`, `unique`, `required`, library metrics (`missingValues` /
  `nullValues` → completeness, `duplicateValues` → uniqueness, `invalidValues`
  → validity, percent bounds → threshold), and `engine: contractgate` freshness.
  **`x-cg-*` wins** when both exist; `x-cg-quality` wins over the mapped metric.
  `description` may be a string or the object form (`purpose` › `usage` ›
  `limitations`).
* **Not mapped:** schema-level `rowCount` (DCG validates per event and has no
  dataset row-count rule), exclusive bounds, non-numeric (date) bounds, and
  date formats other than `yyyy-MM-dd` (kept in `format`, but DCG's `date`
  type always checks `YYYY-MM-DD`). Transforms, glossary, and metrics need
  Mode A. Mode B imports stay `requires_review: true`.
* `format` is enforced by the DCG validator for `email`, `uuid`, and `uri`/`url`
  on string fields. Other formats are carried as metadata.
* Without `target_contract_id`, a new contract needs an org. With auth disabled
  (`CONTRACTGATE_DEV_NO_AUTH`) send `x-org-id`. With no org at all the API
  answers **400**, not the old 500 (`contracts.org_id` is NOT NULL).

## Consumer notes

* **CsvPath Framework (0.0.630):** a raw DCG export loads and converts with
  no shim. CsvPath enforces pattern, enum, bounds, lengths, `email` format,
  `unique`, and the completeness threshold. It skips `type: custom` (freshness)
  and array columns, and reports them in `conversion.report.skipped`.
  **Header matching:** CsvPath does not match CSV headers that contain dots,
  so DCG's flattened nested names (`customer.id`) fail on CSV. Use flat column
  names (rename properties, e.g. `customer_id`) when validating CSV files with
  CsvPath. DCG keeps dot-notation because it validates JSON events.
* **OpenMetadata 2.0.5:** see `integrations/openmetadata/README.md`. OM
  deserializes nested `customProperties` as an object (the spec says array) and
  requires string custom-property values, so `dcg_odcs_for_om` still strips
  nested ones and stringifies root values. OM keeps the description and the
  `invalidValues` (enum) rule. It drops `logicalTypeOptions` and root
  `customProperties`.

## Pure standard ODCS v3.1 (joint DCG/CsvPath example)

`tests/fixtures/odcs/joint-orders.odcs.yaml` (no `x-`, no `customProperties`) imports as a new
contract (201, Mode B) with every constraint intact: `logicalTypeOptions` pattern /
minimum / maximum / minLength / maxLength / format; `library` quality metrics
(`invalidValues` + `validValues` → enum, `duplicateValues` / `unique: true` → one uniqueness
rule, `nullValues` → completeness); object-form description; top-level name.
`logicalType: timestamp` maps to the DCG `timestamp` field type (RFC 3339 string, validated on
ingest; the ODCS `format` is kept as metadata and re-exported). The schema-level `rowCount`
rule has no per-event equivalent in DCG and is dropped (CsvPath enforces it on files).
An `x-org-id` that names no existing org (dev/no-auth) now returns 400, not a 500.
