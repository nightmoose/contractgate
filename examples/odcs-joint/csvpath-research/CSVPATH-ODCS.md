# DCG ↔ CsvPath ODCS Portability Findings

> **UPDATE 2026-10-15 (after `feat/odcs-strict`, PR #282 → staging):** see **"After"** at the end. Short version: a raw DCG export now passes CsvPath's official ODCS JSON Schema check with no shim. CsvPath's how-to example now imports into DCG with pattern, enum, min/max, unique, format and required intact.

**Date:** 2026-10-15 (ET)  
**DCG branch:** `staging` @ `cb06b52` (fresh clone `/workspace/cg-csvpath`)  
**CsvPath:** `csvpath==0.0.630` (pip; Python 3.12)  
**Local DCG:** localhost:8080, `CONTRACTGATE_DEV_NO_AUTH=1`, demo org `cccccccc-…`  
**Probe contract:** `csvpath_portability_test` id `9c6c9453-b977-4de4-9640-5df2e8428747` (nested objects, enums, patterns, min/max, min/max length, PII transform, quality, compliance_mode)  
**Artifacts:** `artifacts/csvpath-odcs/`

How-to used: https://www.csvpath.org/topics/how-tos/validating-with-odcs  
Pattern: `OdcsContractLoader` → `OdcsConverter` → `CsvPath().collect(...)`

---

## Executive summary (Alex / blog / LinkedIn)

DataContractGate and CsvPath both speak ODCS, but they do not yet speak the *same dialect*. A raw DCG export fails CsvPath’s official ODCS JSON Schema gate before any row is checked: root `x-contractgate-*` extensions, string `description`, `logicalType: double`, and DCG quality `type` values (`uniqueness` / `completeness`) are all rejected. After a thin shim (drop root extensions, wrap description, map `double`→`number`, and rewrite constraints from `x-cg-*` customProperties into native `logicalTypeOptions` / `enum`), CsvPath converts the contract and enforces pattern, enum, bounds, and length on flat CSV — proving the *semantics* travel if the *shape* is standardized. Nested DCG fields exported as dotted names (`customer.id`) also confuse CsvPath’s header matcher; flatten to underscores (or set `physicalName`) for tabular runs. The reverse path is worse for partners: Mode B import into DCG only restores constraints from `x-cg-*` customProperties, so a CsvPath-idiomatic ODCS (native `logicalTypeOptions` + `enum`) arrives as types-and-required-only, with `requires_review=true`. Same class of gap as OpenMetadata: DCG parks validation power in extensions that strict ODCS consumers ignore or strip. The highest-leverage DCG fixes are emit `number` not `double`, emit constraints in `logicalTypeOptions` *in addition to* customProperties, and accept those native fields on Mode B import — not a one-liner PR today.

---

## What worked

1. **Local DCG reuse.** Existing OM-era gateway on `:8080` with Postgres; created/promoted a rich probe contract; `GET /contracts/{id}/versions/{v}/export` returned ODCS v3.1.0 with flattened nested fields, `x-cg-*` customProperties, and full `x-contractgate-ontology` (Mode A lossless block).
2. **CsvPath install.** `pip install csvpath` needs Python ≤3.12 (3.13 failed on `marquez-python`); `mysqlclient` build can be skipped if you install core deps and use ODCS + local CSV only. Import path: `from csvpath.odcs import OdcsContractLoader, OdcsConverter`.
3. **CsvPath’s own example.** Their how-to `orders` contract (`apiVersion: v3.2.0`, `logicalTypeOptions.pattern/minimum/maximum`, `enum: [{value:…}]`, schema-level `quality.metric: rowCount`) loads, converts, and produces a clean csvpath statement with regex, `in(...)`, and bounds.
4. **Shimmed DCG → CsvPath validation.** After remapping DCG constraints into native ODCS fields and bumping to `apiVersion: v3.2.0`:
   - Converter emits regex / enum / integer bounds / string lengths / uniqueness (`distinct`) / missingValues threshold.
   - Flat CSV with underscore headers: good rows match (2/2); bad rows match 0 (bounds + pattern/enum fire).
5. **Reverse import API.** `POST /contracts/import` with `{odcs_yaml, target_contract_id}` accepts CsvPath ODCS as **Mode B** (`import_source=odcs_stripped`, `requires_review=true`) and keeps field names + logical types + `required`.

---

## What CsvPath rejected or ignored

### Hard reject (schema validation — no conversion)

Raw DCG export against CsvPath’s bundled ODCS JSON Schema (`v3.1.0`):

| Issue | DCG emits | CsvPath / ODCS schema expects |
| --- | --- | --- |
| Root extensions | `x-contractgate-version`, `-ontology`, `-glossary`, `-metrics`, `-compliance-mode` | Not allowed (`additionalProperties` / `unevaluatedProperties` false) |
| `description` | string (`"TEST"`) | object (`purpose` / `usage` / `limitations`) |
| Float logical type | `logicalType: double` | one of `string\|date\|timestamp\|time\|number\|integer\|object\|array\|boolean` |
| Quality `type` | `uniqueness`, `completeness`, `validity`, `freshness` | `text` \| `library` \| `sql` \| `custom` (library rules use `metric:`) |

### Silently ignored (after minimal schema fixes)

If you only drop `x-*`, wrap description, map `double`→`number`, and clear illegal quality entries — **keeping** `customProperties`:

- CsvPath **accepts** the document and converts.
- It **does not read** `customProperties` / `x-cg-pattern` / `x-cg-enum` / `x-cg-min` / `x-cg-max` / `x-cg-*-length` / transform metadata.
- Generated csvpath only has types + `required` (`.notnone`). Patterns, enums, bounds, lengths are **not enforced**.
- Skipped report stays empty — extensions are not even listed as “skipped features.”

### Operational friction

- **Dotted property names** (`customer.id`, `customer.address.country`): CsvPath header matching treats `.` specially; validation fails even when CSV columns match the dotted names. Use flat/`physicalName` headers for tabular ODCS runs.
- **CsvPath is tabular-first:** docs explicitly say no deep hierarchical JSON/XML validation; nested objects only work as flattened columns (aligned with DCG’s D-001 flatten, but naming must be CSV-safe).
- Standalone converter sets `validation-mode: print, no-raise, no-fail, no-stop` — `path.is_valid` can stay true while zero lines match; use matched line count / printouts for pass/fail.

### What DCG parks only in extensions (CsvPath never sees)

- All six scalar constraints (pattern, enum, min, max, min_length, max_length) live under `customProperties` as `x-cg-*`, not under ODCS-native `logicalTypeOptions` / `enum`.
- PII `classification: pii` + `x-cg-transform-*` — classification may survive schema; transform is ignored for validation.
- Glossary, metrics, compliance_mode — only in root `x-contractgate-*` (stripped for CsvPath).

---

## Reverse direction (CsvPath ODCS → DCG import)

Imported the how-to example (version bumped to `2.0.0-csvpath`) onto the probe contract.

**Kept**

- Field names, `logicalType` → CG types (including `date`), `required` flags.
- Contract created as draft with `import_source: odcs_stripped`, `requires_review: true`.

**Dropped / lost**

| CsvPath / ODCS native field | After DCG Mode B |
| --- | --- |
| `logicalTypeOptions.pattern` | `pattern: null` |
| `logicalTypeOptions.minimum` / `maximum` | `min` / `max: null` |
| `enum: [{value: …}]` | `enum: null` |
| `unique: true` | not restored (no uniqueness quality unless ODCS quality[] uses DCG’s completeness/uniqueness *type* vocabulary) |
| `logicalTypeOptions.format: email` / date format | dropped |
| Schema-level `quality: [{metric: rowCount, …}]` | `quality: []` (DCG only maps quality entries with `type` in `completeness\|validity\|freshness\|uniqueness`) |
| Object-form `description` / top-level `name` | description often `null` (importer wants string `description`) |

**Root cause:** `odcs_prop_to_field` only restores constraints from `customProperties` (`x-cg-*`). It does **not** read `logicalTypeOptions` or native `enum`. Partner ODCS written the way CsvPath documents it therefore arrives constraint-empty.

New-identity import without `target_contract_id` hit a DB not-null path in this local env when org context was ambiguous; importing as a new version on an existing contract worked.

---

## Comparison to known OpenMetadata ODCS gaps

| Gap | OpenMetadata (prior work) | CsvPath (this probe) |
| --- | --- | --- |
| String top-level `description` | Rejected (HTTP 400); shim wraps `{purpose:…}` | Rejected by JSON Schema |
| Property `customProperties` (`x-cg-*`) | Rejected; shim drops them → constraints lost unless ontology block kept | Schema-ok on v3.2 after other fixes, but **converter ignores them** → same practical loss |
| Root `x-contractgate-*` | OM may strip / not round-trip; DCG Mode A needs them for lossless re-import | **Hard reject** by CsvPath schema validation |
| `logicalType: double` | Not called out as OM blocker previously | **Hard reject**; must be `number` |
| Quality vocabulary | OM-specific | DCG `type: uniqueness\|completeness` ≠ ODCS `type: library` + `metric` |
| Preferred constraint home | OM keeps `logicalTypeOptions` better than customProperties (noted in OM README) | CsvPath **only** enforces `logicalTypeOptions` + native `enum` |
| Shared remediation | `dcg_odcs_for_om` shim | Would need `dcg_odcs_for_csvpath` (or better: fix export/import natively) |

Bottom line: OM and CsvPath both punish DCG’s “constraints in customProperties + CG quality types + string description + double” dialect. Fixing export to dual-write native ODCS fields helps **both** consumers.

---

## Is any tiny fix worth shipping?

**No PR opened.** Prefer report-only as requested.

| Candidate | Size | Worth? |
| --- | --- | --- |
| `FieldType::Float => "number"` (today `"double"`) | One-liner in `src/odcs.rs` | **Yes, small and clearly correct** for ODCS + CsvPath + general conformance. Import already accepts `number`. Needs a quick export fixture update. Still not shipped here. |
| Export `description` as `{purpose: …}` | Small | Helps OM + CsvPath; confirm UI/docs still OK with object form on re-export. |
| Dual-write constraints to `logicalTypeOptions` / `enum` **and** keep `x-cg-*` | Medium (not a one-liner) | **Highest leverage** for CsvPath + OM; preserves Mode A round-trip. |
| Mode B: read `logicalTypeOptions` + `enum` on import | Medium | Required for partner ODCS (CsvPath how-to) to be useful in DCG. |
| Map quality completeness/uniqueness ↔ library metrics | Medium | Needed for schema-valid exports consumers can load. |

Ship recommendation for a later PR: start with `double`→`number`, then dual-write native constraint fields; add Mode B readers in the same change set so export/import stay symmetric.

---

## Repro notes

```bash
# Export
curl -s -H "x-org-id: cccccccc-cccc-cccc-cccc-cccccccccccc" \
  "http://localhost:8080/contracts/9c6c9453-…/versions/1.0.0/export" > dcg.odcs.yaml

# CsvPath (after schema shim — see artifacts/csvpath-odcs/dcg-shimmed-v32.odcs.yaml)
python - <<'PY'
from csvpath import CsvPath
from csvpath.odcs import OdcsContractLoader, OdcsConverter
odcs = OdcsContractLoader.from_path(path="dcg-shimmed-v32.odcs.yaml")
conv = OdcsConverter(contract=odcs).convert(path="orders_flat.csv")
path = CsvPath()
lines = path.collect(next(iter(conv.csvpaths.values())))
print(path.is_valid, len(lines))
PY
```

CsvPath docs (validated): supports ODCS v3.0.x / v3.1.0 / v3.2.0; tabular/JSONL/Excel/DataFrames; not deep JSON/XML; not SQL rules / relational multi-entity in standalone mode.


---

## After — `feat/odcs-strict` (2026-10-15)

Verified live against a local DCG built from the branch (localhost:8081, auth off, local Postgres) with **csvpath 0.0.630 on Python 3.12.15, no shim**. Unit tests check the same against the vendored official ODCS v3.1.0 JSON Schema, which is byte-identical to CsvPath's copy.

### Executive summary (after, blog/LinkedIn ready)

DataContractGate's ODCS export now passes the official ODCS v3.1.0 JSON Schema unmodified, the same check CsvPath Framework applies before it validates a row. DCG still carries its lossless extensions. They now sit in the spec's `customProperties`, and every constraint is also written in native ODCS: `logicalTypeOptions` pattern, length, min/max and format, `unique`, and standard library quality metrics. CsvPath therefore enforces DCG's rules directly: regex, enum, bounds, email format, uniqueness and the completeness threshold. The reverse also works. CsvPath's own how-to contract imports into DCG with pattern, enum, min/max, unique, format and required intact, and DCG enforces them on live events. DCG→DCG round trips stay lossless, and old exports still import.

### Before → after

| Check | Before | After |
|---|---|---|
| CsvPath `OdcsContractLoader` on a raw DCG export | **rejected**: root `x-contractgate-*`, string `description`, `logicalType: double`, quality `type: uniqueness` | **passes** |
| Constraints CsvPath enforces from a DCG export | types + required only (constraints hid in `x-cg-*`) | pattern, enum, min/max, `email` format, `unique`, completeness threshold, date format |
| CsvPath how-to → DCG import, no target | **500** (not-null `contracts.org_id`) | **201**, `odcs_stripped`, `requires_review: true` |
| …pattern / enum / min / max / unique / format / required | lost / lost / lost / lost / lost / lost / kept | **all kept** (format is a new optional DCG field; `email`/`uuid`/`uri` enforced) |
| …row-count rule | dropped | still dropped: DCG validates per event and has no dataset row-count rule (documented) |
| DCG → DCG (Mode A) | lossless | lossless (new and legacy extension locations, also string-encoded values) |
| Import with no org in auth-off mode | 500 | 400 naming the `x-org-id` header |

### What CsvPath still skips (reported in `conversion.report.skipped`)

- `minLength`/`maxLength` on a field that also has `format: email` (CsvPath's `email()` takes no length).
- Array columns (no CsvPath type, presence-only check).
- `type: custom, engine: contractgate` freshness: vendor engine by design.
- **Header matching:** DCG flattens nested objects to dot names (`customer.id`). CsvPath does not match dotted CSV headers, so those columns fail. With flat names (`customer_id`) the same export validated 2/2 good rows, and in the bad file only the valid row matched (1/3): the bounds, email, date and duplicate `order_id` rows were rejected. Documented in `docs/odcs/strict-odcs.md`. DCG keeps the dot shape, which it needs for JSON events.

### OpenMetadata re-check (local OM 2.0.5)

- DCG's object `description` and native enum (`invalidValues`) now survive DCG → OM → DCG.
- OM still drops `logicalTypeOptions` (pattern, min/max) and root `customProperties`.
- OM deviates from the spec in two places: nested `customProperties` must be objects, and custom-property values must be strings. `dcg_odcs_for_om` is now limited to those two workarounds (plus wrapping legacy string descriptions). `roundtrip_check.py`: OM import valid, no issues, DCG re-import OK.

### Left / follow-ups

- Row-count (dataset-level) rules have no DCG equivalent.
- Date `format` other than `yyyy-MM-dd` is stored but DCG's `date` type always checks YYYY-MM-DD.
- Exclusive bounds and date bounds are not mapped.
- Pattern and range rules don't survive OpenMetadata (OM drops `logicalTypeOptions`).
- A Mode B import of a stripped DCG export turns the enum restatement into an extra (harmless) validity rule.
- Export stays `apiVersion: v3.1.0`, so enum uses `invalidValues` rather than v3.2 `enum`.

### Joint example (pure standard ODCS v3.1, /workspace/odcs-joint/orders.odcs.yaml)

Imports into DCG as a new contract (201, Mode B) with all 11 fields' constraints intact: patterns,
min/max, min/max length, `format: email`, three enums from `invalidValues`/`validValues`, uniqueness
(duplicateValues + unique) and completeness (nullValues) rules, object-form description, name
`orders`, and `order_ts` kept as `timestamp` (new DCG field type, RFC 3339 validated). Only the
schema-level `rowCount > 0` rule is dropped (no per-event equivalent). Covered by
`odcs_strict_tests::joint_standard_odcs_orders_imports_with_constraints`, which also checks that
every constraint fires on a bad event and that the re-export passes the official v3.1.0 schema.
