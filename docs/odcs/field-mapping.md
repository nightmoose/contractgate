# ODCS v3.1.0 Field Mapping — ContractGate Native → ODCS

**Status:** Week 1 deliverable  
**Date:** 2026-05-01  
**ODCS pin:** v3.1.0 (`apiVersion: v3.1.0`)  
**Spec base URL:** https://bitol-io.github.io/open-data-contract-standard/v3.1.0/

---

## How to read this table

| Column | Meaning |
|--------|---------|
| **CG Path** | Dot-path in ContractGate native YAML or Rust type |
| **CG Type** | Rust/YAML type |
| **CG Card.** | required / optional |
| **ODCS Path** | Dot-path in an ODCS v3.1.0 document |
| **ODCS Type** | YAML type per spec |
| **ODCS Card.** | M = Mandatory · R = Recommended · O = Optional |
| **Status** | Direct match · Renamed · Restructured · No equivalent |
| **ODCS §** | Spec section link |
| **Notes** | Round-trip risks, naming decisions, caveats |

---

## Section 1 — Contract Top-Level

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `version` | string | required | `version` | string | M | **Direct match** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | Both carry the semantic version of the contract content (e.g. "1.0"). No transformation needed. |
| `name` | string | required | _(none at top level)_ | — | — | **Restructured** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) / [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | ODCS has no single `name` field at the contract level. The equivalent contract identity is expressed via `dataProduct` (Fundamentals) plus `schema[].name` (the dataset name). On export CG `name` should populate **both** `dataProduct` and `schema[0].name`. On import, `schema[0].name` wins. **Round-trip risk.** |
| `description` | string (optional) | optional | `description.purpose` | string (inside object) | O | **Restructured** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | ODCS `description` is an object with sub-keys `purpose`, `limitations`, `usage`. CG's flat string maps to `description.purpose`. On import, CG should concatenate any present sub-keys with a delimiter. |
| `compliance_mode` | bool (default false) | optional | _(none)_ | — | — | **No equivalent** | — | RFC-004 per-version enforcement flag. No ODCS home. Must be exported as `x-contractgate-compliance-mode`. See extension-strategy.md. |
| `ontology` | Ontology object | required | `schema` (array) | array | R | **Restructured** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | CG flattens all field definitions under a single `ontology.entities[]` list. ODCS nests field definitions inside one or more `schema[]` dataset objects (tables, topics). On export, all CG entities are placed under a single synthesised `schema[0]`. On import, only `schema[0].properties[]` is processed unless explicitly extended. |
| `glossary` | GlossaryEntry[] (default []) | optional | _(none as top-level block)_ | — | — | **No equivalent** | — | ODCS has no dedicated glossary section. Per-field semantic data is distributed across `schema[].properties[].description`, `.businessName`, and `.authoritativeDefinitions`. Full glossary must be preserved as `x-contractgate-glossary`. See gap-analysis.md §CG-GAP-003. |
| `metrics` | MetricDefinition[] (default []) | optional | _(none as top-level block)_ | — | — | **No equivalent** | — | ODCS has no formula-metrics section. Closest available landing zones are `schema[].quality[]` (for threshold rules) and `slaProperties[]` (for SLO-style bounds). Formula metrics have no structural home. Full block must be preserved as `x-contractgate-metrics`. See gap-analysis.md §CG-GAP-004. |

---

## Section 2 — FieldDefinition (ontology.entities[n])

Mapped to ODCS `schema[0].properties[n]`.

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `ontology.entities[].name` | string | required | `schema[].properties[].name` | string | M | **Renamed / Restructured** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | Direct semantic equivalent. Position in hierarchy changes (entities[] → properties[]). Safe round-trip. |
| `ontology.entities[].type` | FieldType enum | required | `schema[].properties[].logicalType` | string | R | **Renamed** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | CG `type` → ODCS `logicalType`. CG accepts `"number"` as alias for `"float"`; ODCS uses freeform strings (`"string"`, `"integer"`, `"date"`, etc.). CG `"float"` / `"number"` should export as `"double"` or `"float"` per ODCS convention. CG `"any"` has no ODCS equivalent logical type — export as `"any"` with an extension note. **Naming decision needed.** |
| `ontology.entities[].required` | bool (default true) | optional | `schema[].properties[].required` | bool | O | **Direct match** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | Same field name, same semantics. Safe round-trip. |
| `ontology.entities[].pattern` | string (regex) | optional | _(none)_ | — | — | **No equivalent** | — | ODCS has no regex-pattern constraint on properties. Closest: `schema[].properties[].quality[]` with `type: custom` and a SQL/regex expression. However, this loses the machine-parseable constraint on import. Must be preserved as `x-contractgate-pattern` in `customProperties`. **Round-trip risk.** |
| `ontology.entities[].enum` (YAML: `enum`) | Value[] | optional | _(none)_ | — | — | **No equivalent** | — | No allowed-values list in ODCS schema properties. Same situation as `pattern` — can be expressed as a quality rule but loses import fidelity. Must be preserved as `x-contractgate-enum`. **Round-trip risk.** |
| `ontology.entities[].min` | f64 | optional | _(none as schema constraint)_ | — | — | **No equivalent** | — | ODCS does not carry numeric range constraints in `schema[].properties[]`. Can be approximated via `schema[].properties[].quality[].mustBeGreaterThan` (losing the per-event real-time enforcement semantics). Must also be preserved as `x-contractgate-min`. |
| `ontology.entities[].max` | f64 | optional | _(none as schema constraint)_ | — | — | **No equivalent** | — | Same as `min`. Map to `quality[].mustBeLessThan` for ODCS consumers. Preserve as `x-contractgate-max`. |
| `ontology.entities[].min_length` | usize | optional | _(none)_ | — | — | **No equivalent** | — | No string-length constraint in ODCS. Preserve as `x-contractgate-min-length`. |
| `ontology.entities[].max_length` | usize | optional | _(none)_ | — | — | **No equivalent** | — | No string-length constraint in ODCS. Preserve as `x-contractgate-max-length`. |
| `ontology.entities[].properties` | FieldDefinition[] | optional | `schema[].properties[]` (nested via custom structure) | — | — | **Restructured** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | ODCS does not natively support nested property trees within a single `schema[]` entry. Nested object fields must be flattened to dot-notation names (e.g. `metadata.page`) or placed in a child `schema[]` object. **Structural decision needed before Week 2.** |
| `ontology.entities[].items` | Box\<FieldDefinition\> | optional | _(none)_ | — | — | **No equivalent** | — | ODCS schema properties do not describe array element types. The container field's `logicalType` can be `"array"` but element constraints are lost. Preserve as `x-contractgate-items`. |
| `ontology.entities[].transform` | Transform | optional | _(none)_ | — | — | **No equivalent** | — | CG's PII transform pipeline (mask/hash/drop/redact) has no structural home in ODCS. `schema[].properties[].encryptedName` is the closest field but covers only the storage-name alias, not the transform operation. Must be exported entirely as `x-contractgate-transform` inside `customProperties`. **Differentiator — do not drop.** |

---

## Section 3 — Transform sub-fields

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `ontology.entities[].transform.kind` | TransformKind enum (mask/hash/drop/redact) | required (when transform present) | _(none)_ | — | — | **No equivalent** | — | Preserve as `customProperties[].property: x-contractgate-transform-kind`. |
| `ontology.entities[].transform.style` | MaskStyle enum (opaque/format_preserving) | optional | _(none)_ | — | — | **No equivalent** | — | Preserve as `customProperties[].property: x-contractgate-transform-style`. |

---

## Section 4 — GlossaryEntry (glossary[n])

ODCS absorbs glossary data into per-property metadata within `schema[].properties[]`. There is no top-level glossary block.

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `glossary[].field` | string | required | `schema[].properties[].name` (implicit reference) | string | M | **Restructured** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | `field` is a pointer to an entity name. On export, glossary data for field `F` is merged into the `properties[]` entry where `name == F`. |
| `glossary[].description` | string | required | `schema[].properties[].description` | string | O | **Renamed / Restructured** | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | Direct semantic equivalent but distributed into per-property position. Merge strategy: if ontology entity already has a description, glossary description takes precedence on export (glossary is the authoritative business definition). |
| `glossary[].constraints` | string (optional) | optional | _(none)_ | — | — | **No equivalent** | — | Informational constraint prose. No ODCS field. Preserve as `customProperties[].property: x-contractgate-constraints` on the matching property. |
| `glossary[].synonyms` | string[] (optional) | optional | _(none)_ | — | — | **No equivalent** | — | Alternate names. No ODCS field. Preserve as `customProperties[].property: x-contractgate-synonyms`. May also partially map to `schema[].properties[].businessName` (a single alternate name). |

---

## Section 5 — MetricDefinition (metrics[n])

ODCS `quality[]` (at schema or property level) covers threshold validation rules. Formula-style aggregate metrics have no ODCS structural equivalent.

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `metrics[].name` | string | required | `schema[].properties[].quality[].metric` (partial) | string | M (within quality) | **Restructured** | [Data Quality](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/data-quality/) | CG metric name becomes the quality `metric` identifier. Applies only to field-bound metrics (`field` is set). Formula metrics have no ODCS home. |
| `metrics[].field` | string (optional) | optional | implied by placement in `schema[].properties[n].quality[]` | — | — | **Restructured** | [Data Quality](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/data-quality/) | The `field` value determines which `properties[]` entry hosts the quality rule. If `field` is absent (formula metric), the quality rule is placed at `schema[].quality[]` level with `type: custom`. |
| `metrics[].type` | MetricType (integer/float) | optional | _(none in quality)_ | — | — | **No equivalent** | — | ODCS quality rules do not carry a type annotation. Preserve as `customProperties[].property: x-contractgate-metric-type`. |
| `metrics[].formula` | string (optional) | optional | _(none)_ | — | — | **No equivalent** | — | Formula strings like `"sum(amount) where event_type = 'purchase'"` have no ODCS structural home. Preserve as `x-contractgate-metrics` top-level extension or as `customProperties[].property: x-contractgate-formula` on the quality rule. **Key differentiator. Do not drop.** |
| `metrics[].min` | f64 (optional) | optional | `schema[].properties[].quality[].mustBeGreaterThan` | number | O (within quality) | **Renamed / Restructured** | [Data Quality](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/data-quality/) | CG `min` → ODCS `mustBeGreaterThan` (or `mustBe` for equality). Semantics differ slightly: CG is inclusive (`>=`), ODCS `mustBeGreaterThan` is exclusive (`>`). Use ODCS `mustBe` + `mustBeGreaterThan` depending on value. **Off-by-one risk on round-trip.** |
| `metrics[].max` | f64 (optional) | optional | `schema[].properties[].quality[].mustBeLessThan` | number | O (within quality) | **Renamed / Restructured** | [Data Quality](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/data-quality/) | Same semantics caveat as `min`. Preserve exact CG bounds via `x-contractgate-max` alongside the quality representation. |

---

## Section 6 — DB-Level Fields (ContractIdentity / ContractVersion)

These fields exist only in the Supabase storage layer today, not in the YAML contract content. On ODCS export they must be surfaced.

| CG Path | CG Type | CG Card. | ODCS Path | ODCS Type | ODCS Card. | Status | ODCS § | Notes |
|---------|---------|----------|-----------|-----------|------------|--------|--------|-------|
| `ContractIdentity.id` (UUID) | UUID | required (DB) | `id` | string (UUID) | **M** | **Restructured** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | ODCS requires `id` at the top level of the YAML document. CG currently stores this in the DB identity row but does not embed it in YAML. **Must be added to YAML on export (or always round-tripped).** |
| `ContractVersion.state` (VersionState) | draft/stable/deprecated | required (DB) | `status` | string | **M** | **Renamed** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | CG `state` → ODCS `status`. Value mapping: `draft` → `proposed`, `stable` → `active`, `deprecated` → `deprecated`. ODCS also supports `retired` (no CG equivalent yet). **Round-trip risk: ODCS `proposed` must map back to CG `draft`.** |
| `ContractIdentity.multi_stable_resolution` | strict/fallback | optional (DB) | _(none)_ | — | — | **No equivalent** | — | CG internal routing policy. Preserve as `x-contractgate-multi-stable-resolution` if needed in exported documents. |
| `ContractIdentity.pii_salt` | bytes | required (DB) | _(none)_ | — | — | **No equivalent** | — | **Must NEVER be exported.** No ODCS home. Internal secret. |
| `ContractVersion.promoted_at` / `deprecated_at` | datetime (optional) | optional (DB) | `contractCreatedTs` (partial) | datetime string | O | **Restructured** | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | `contractCreatedTs` maps to `ContractVersion.created_at`. There is no ODCS field for promotion or deprecation timestamps. Preserve as `x-contractgate-promoted-at` / `x-contractgate-deprecated-at`. |

---

## Section 7 — ODCS Mandatory Fields ContractGate Does NOT Currently Produce

These fields are **required** by ODCS v3.1.0 but are absent from CG native YAML.

| ODCS Field | ODCS Path | ODCS Card. | CG Status | ODCS § | Week 2 Action |
|------------|-----------|------------|-----------|--------|---------------|
| `apiVersion` | top-level | **M** | Not present | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | Inject `"v3.1.0"` at export time. Do not add to CG native YAML. |
| `kind` | top-level | **M** | Not present | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | Inject `"DataContract"` at export time. |
| `id` | top-level | **M** | In DB only (`ContractIdentity.id`) | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | Embed in exported YAML. On native CG YAML the DB UUID is authoritative. |
| `status` | top-level | **M** | In DB only (`ContractVersion.state`) | [Fundamentals](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/fundamentals/) | Derive from `VersionState` on export using the mapping above. |
| `schema` (block) | top-level | **R** (Recommended) | Partially: CG has `ontology.entities[]` | [Schema](https://bitol-io.github.io/open-data-contract-standard/v3.1.0/schema/) | Synthesise `schema[0]` from `ontology.entities[]` on export. |

---

## Section 8 — ODCS Recommended / Optional Fields Worth Supporting

| ODCS Field | ODCS Path | ODCS Card. | Rationale | Priority |
|------------|-----------|------------|-----------|----------|
| `domain` | top-level | R | Enables catalog/discovery in multi-domain orgs | Week 3 |
| `dataProduct` | top-level | R | Natural contract identity complement to `name` | Week 3 |
| `tenant` | top-level | O | Hooks into future tenant model (RFC-001 post-sign-off) | Week 3 |
| `schema[].physicalName` | schema | R | Required for Kafka / dbt starter contracts to map to real topic/table names | Week 2 |
| `schema[].physicalType` | schema | R | `table`, `topic`, `view` — needed for Kafka + dbt starters | Week 2 |
| `schema[].properties[].classification` | schema | O | PII classification label (public/restricted/confidential) — complements CG transforms | Week 3 |
| `schema[].properties[].criticalDataElement` | schema | O | Flag for data-governance tooling integration | Week 4 |
| `tags` | top-level | O | Enables contract discovery via catalog tools | Week 3 |
| `authoritativeDefinitions` | top-level + per-property | O | Links to data dictionaries, Collibra, Atlan, etc. | Week 4 |
| `slaProperties` | top-level | O | SLA/SLO metadata for downstream consumers | Week 4 |
| `support` | top-level | O | Contact channels for contract owners | Week 4 |
| `contractCreatedTs` | top-level | O | Audit trail for contract origin | Week 2 (auto-inject from DB) |

---

## Summary Counts

| Status | Count |
|--------|-------|
| Direct match | 2 |
| Renamed | 3 |
| Restructured | 10 |
| No equivalent (CG → ODCS) | 17 |
| ODCS Mandatory missing in CG | 4 (`apiVersion`, `kind`, `id`, `status`) |
| ODCS Recommended missing in CG | 5 |
