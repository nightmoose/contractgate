# ODCS v3.1.0 Gap Analysis — ContractGate

**Status:** Week 1 deliverable  
**Date:** 2026-05-01  
**ODCS pin:** v3.1.0  
**Input:** field-mapping.md (read that first)

---

## Overview

Gaps fall into two directions:

- **CG→ODCS gaps:** ContractGate fields that have no ODCS v3.1.0 home. These are features that make ContractGate more expressive than ODCS. Losing them on export would break round-trip fidelity and strip our differentiators.
- **ODCS→CG gaps:** ODCS mandatory/recommended fields that ContractGate does not currently produce. These are blockers for the "ODCS Conformant" claim.

Each gap has a proposed remedy. Remedies are one of:
- **Inject at export** — the data exists in CG; just needs to be serialized to ODCS format
- **Extension namespace** — no ODCS home; preserve via `x-contractgate-*`
- **Schema change** — CG native YAML needs a new optional field
- **Drop on export** — safe to omit from ODCS output (no semantic loss for downstream consumers)
- **Deferred** — needed for full conformance but not a Week 2 blocker

---

## Part A — CG Features with No ODCS Equivalent

### CG-GAP-001 · `compliance_mode` (RFC-004 strict field enforcement)

**CG field:** `Contract.compliance_mode` (bool, default false)  
**What it does:** When true, the validator rejects any event field not declared in `ontology.entities`. This is a per-contract, per-version enforcement policy that fires at ingestion time.  
**ODCS status:** No equivalent. ODCS does not define runtime enforcement behaviour — it is a schema description standard, not a validator.  
**Round-trip risk:** HIGH. Importing an ODCS contract into CG with no `compliance_mode` signal would silently default to `false`, changing enforcement behaviour for contracts that originated in CG with `compliance_mode: true`.  
**Proposal:** Preserve via `x-contractgate-compliance-mode: true` at the ODCS document top level. On import, CG reads this extension key and restores the flag. On export it is always written regardless of value (write `false` explicitly to avoid import defaulting to wrong value).  
**Extension placement:** Top-level document key.  
**Week 2 action:** Export layer writes `x-contractgate-compliance-mode`. Import layer reads it.

---

### CG-GAP-002 · Validation constraints: `pattern`, `enum`, `min`, `max`, `min_length`, `max_length`

**CG fields:** Per-entity on `FieldDefinition`  
**What they do:** Machine-executable constraints that fire at per-event validation time. They are the core of ContractGate's value proposition — the validation engine relies on them directly.  
**ODCS status:** No structural equivalent in `schema[].properties[]`. ODCS `quality[]` rules can express some of these (threshold `mustBe`, `mustBeGreaterThan`, etc.) but:
1. They are intended as dataset-level quality metrics, not per-event schema enforcement.
2. They use SQL-expression evaluation models, not the CG validator's field-level constraint model.
3. `pattern` (regex), `enum`, `min_length`, and `max_length` have no quality-rule equivalents at all.

**Round-trip risk:** CRITICAL. If these constraints are dropped on export, re-importing the ODCS document into CG produces a contract with no validation rules — behavioural change, potential data quality failures downstream.  
**Proposal:**
- For `min` / `max`: additionally emit as `quality[].mustBeGreaterThan` / `mustBeLessThan` on the matching property so ODCS-native tooling benefits from the constraint information (use dimension: `validity`).
- For all six: always preserve originals as `customProperties` extensions on the matching `properties[]` entry:

```yaml
customProperties:
  - property: x-contractgate-pattern
    value: "^[a-zA-Z0-9_-]{3,64}$"
  - property: x-contractgate-enum
    value: ["click", "view", "purchase"]
  - property: x-contractgate-min
    value: 0.0
  - property: x-contractgate-max
    value: 1000000.0
  - property: x-contractgate-min-length
    value: 3
  - property: x-contractgate-max-length
    value: 64
```

**Extension placement:** `schema[].properties[n].customProperties[]`  
**Week 2 action:** Export layer writes all six extension keys when present. Import layer reads them back and populates `FieldDefinition` fields. `min`/`max` are also written to quality rules for ODCS consumers.  
**Note on semantic mismatch:** ODCS `mustBeGreaterThan` is exclusive (`>`), CG `min` is inclusive (`>=`). Export layer must use `mustBe` for equality or document the off-by-one and use `mustBeGreaterThanOrEqualTo` if the ODCS v3.1.0 quality vocabulary supports it (verify against spec before Week 2).

---

### CG-GAP-003 · `glossary` block

**CG field:** `Contract.glossary` (Vec\<GlossaryEntry\>)  
**What it does:** Carries business definitions, constraints text, and synonyms for each field. This is the semantic layer that makes contracts human-readable to data stewards and AI systems that consume the contract.  
**ODCS status:** No top-level glossary section. ODCS distributes business meaning across per-property fields: `description`, `businessName`, and `authoritativeDefinitions`.  
**Round-trip risk:** MEDIUM. The semantic content can be partially re-assembled from `properties[].description`, but `constraints` and `synonyms` are lost entirely. Any downstream CG tooling (AI classification, contract diff, semantic search) that reads glossary entries would break on re-import.  
**Proposal — two-part strategy:**
1. **Lossy ODCS representation** (for ODCS consumers): merge `glossary[].description` → `schema[].properties[n].description`; merge `glossary[].synonyms[0]` → `schema[].properties[n].businessName`.
2. **Lossless round-trip preservation**: write the entire glossary block verbatim as `x-contractgate-glossary` at the top level of the exported document.

```yaml
x-contractgate-glossary:
  - field: user_id
    description: "Unique, stable identifier for a registered user"
    synonyms: ["uid", "userId"]
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative; maximum 1 000 000"
```

**Extension placement:** Top-level document key.  
**Week 2 action:** Export layer writes both representations. Import layer reads `x-contractgate-glossary` if present; falls back to assembling from `properties[].description` if not.

---

### CG-GAP-004 · `metrics` block (formula metrics in particular)

**CG field:** `Contract.metrics` (Vec\<MetricDefinition\>)  
**What it does:** Two sub-types:
- **Field-bound metrics** (`field` + `min`/`max`): real-time per-event bounds validation. Ingest pipeline enforces these.
- **Formula metrics** (`formula` string): aggregate KPI definitions for downstream analytics. CG stores these for reference; they are not evaluated at ingest time.

**ODCS status:** Field-bound metric bounds partially map to `quality[]` rules. Formula metrics have no ODCS structural home whatsoever — there is no formula expression language in the spec.  
**Round-trip risk:** HIGH for formula metrics (complete information loss if dropped). MEDIUM for field-bound metrics (bounds recoverable from quality rules; `type` annotation lost).  
**Proposal:**
- **Field-bound** `min`/`max`: emit as `schema[].properties[n].quality[]` (see CG-GAP-002 rationale) AND preserve full metric definition in `x-contractgate-metrics`.
- **Formula metrics**: no ODCS representation possible. Preserve exclusively in `x-contractgate-metrics`.

```yaml
x-contractgate-metrics:
  - name: total_revenue
    formula: "sum(amount) where event_type = 'purchase'"
  - name: purchase_amount
    field: amount
    type: float
    min: 0.0
    max: 1000000.0
```

**Extension placement:** Top-level document key.  
**Week 2 action:** Export always writes `x-contractgate-metrics`. Import reads it as authoritative; ignores `quality[]` for round-trip (quality rules are for ODCS consumers only).

---

### CG-GAP-005 · PII Transforms (`transform.kind`, `transform.style`)

**CG field:** `FieldDefinition.transform` (optional, on string-typed fields)  
**What it does:** Declares a post-validation PII transformation to apply at ingest: `mask`, `hash`, `drop`, or `redact`. The `pii_salt` in `ContractIdentity` is the keying material for `hash` and `format_preserving` mask.  
**ODCS status:** No equivalent. ODCS `schema[].properties[].encryptedName` is the closest field — it stores the alternate name of an encrypted column, not the transformation operation. ODCS `transformLogic` / `transformDescription` describe lineage transforms, not runtime PII operations.  
**Round-trip risk:** CRITICAL. If transforms are dropped on export and re-import, PII fields will no longer be masked/hashed/redacted at ingest. Potential compliance failure (GDPR, CCPA). This is ContractGate's most important differentiator.  
**Proposal:** Preserve transform declaration as `customProperties` on the matching property. Additionally populate `classification: restricted` (or `classification: confidential`) on the property to signal sensitivity to ODCS-native tools.

```yaml
schema:
  - properties:
      - name: user_id
        logicalType: string
        required: true
        classification: restricted
        customProperties:
          - property: x-contractgate-transform-kind
            value: mask
          - property: x-contractgate-transform-style
            value: opaque
```

**Extension placement:** `schema[].properties[n].customProperties[]`  
**Week 2 action:** Import layer must treat missing `x-contractgate-transform-*` as a warning, not a silent no-op. If a field had `classification: restricted` but no transform extension, surface a conformance warning asking the user to declare a transform.  
**Note on `pii_salt`:** The salt is a DB-level secret and must NEVER be exported to ODCS. The transform declaration is exported; the keying material is not. This is correct and safe.

---

### CG-GAP-006 · `multi_stable_resolution` (strict/fallback routing)

**CG field:** `ContractIdentity.multi_stable_resolution`  
**What it does:** Controls which stable version is used when a contract has multiple stable versions: `strict` enforces latest-stable-only, `fallback` tries in promoted_at DESC order.  
**ODCS status:** No equivalent. This is an operational routing policy internal to the CG gateway.  
**Round-trip risk:** LOW. Affects only CG-internal routing; no downstream ODCS tool cares about it.  
**Proposal:** Export as `x-contractgate-multi-stable-resolution` at top level. Import reads it; default `strict` if absent.  
**Week 2 action:** Write the extension on export; read on import.

---

### CG-GAP-007 · `ontology.entities[].items` (Array element constraints)

**CG field:** `FieldDefinition.items` (Box\<FieldDefinition\>)  
**What it does:** Constrains the element type of array-typed fields.  
**ODCS status:** No equivalent. ODCS `logicalType: array` marks a field as an array but cannot describe element types.  
**Round-trip risk:** LOW (array fields are uncommon in current contracts). Constraints on array elements are lost on export.  
**Proposal:** Preserve as `customProperties[].property: x-contractgate-items` with the serialized FieldDefinition as the value.  
**Week 2 action:** Implement, low priority relative to CG-GAP-001–005.

---

### CG-GAP-008 · Nested object properties (`ontology.entities[].properties`)

**CG field:** `FieldDefinition.properties` (for `type: object` fields)  
**What it does:** Defines the sub-fields of a nested JSON object (e.g. `metadata.page`, `metadata.referrer`).  
**ODCS status:** ODCS `schema[]` is a flat list of tables/datasets; `properties[]` within a schema object is a flat list of columns. There is no recursive property nesting in the standard.  
**Round-trip risk:** MEDIUM. Nested object structure is lost on a naïve export. Sub-fields would need to be either: (a) flattened to dot-notation names, or (b) represented as a nested `schema[]` object referencing the parent.  
**Decision needed (before Week 2):** Choose strategy A (flatten) or B (child schema). Recommendation: Strategy A for simplicity. Prefix sub-field names with parent name and dot (e.g., `metadata` becomes `metadata.page`, `metadata.referrer` as separate properties). Preserve original nested structure in `x-contractgate-properties` on the parent property.  
**Week 2 action:** Decide and implement before building the export serialiser.

---

## Part B — ODCS Mandatory Fields Missing in ContractGate

### ODCS-GAP-001 · `apiVersion` (MANDATORY)

**ODCS path:** top-level  
**Required value:** `"v3.1.0"`  
**CG status:** Not present anywhere.  
**Impact:** Without this field, the document is not valid ODCS v3.1.0.  
**Proposal:** Inject at export time. Value is always `"v3.1.0"` (pinned to Week 1 analysis; update only when CG explicitly upgrades ODCS support). Never add to CG native YAML — it belongs only in the ODCS output format.  
**Week 2 action:** Export layer always writes this.

---

### ODCS-GAP-002 · `kind` (MANDATORY)

**ODCS path:** top-level  
**Required value:** `"DataContract"`  
**CG status:** Not present.  
**Proposal:** Inject at export time, always `"DataContract"`.  
**Week 2 action:** Export layer always writes this.

---

### ODCS-GAP-003 · `id` (MANDATORY)

**ODCS path:** top-level  
**Required value:** UUID string  
**CG status:** UUID exists as `ContractIdentity.id` in the DB, but is not embedded in the YAML contract content.  
**Impact:** Without a stable `id`, ODCS consumers cannot reference or link to the contract by identity.  
**Round-trip risk:** HIGH. On import of a foreign ODCS document (one not originating from CG), there is no existing `ContractIdentity` to provide a UUID. The import layer must either generate a new UUID or accept the ODCS `id` as the CG identity UUID.  
**Proposal:**
- On **export** of an existing CG contract: write `ContractIdentity.id` as `id`.
- On **import** of a foreign ODCS document: use the ODCS `id` value if it is a valid UUID; otherwise generate a new UUID and store the original value as `x-contractgate-original-id`.  
**Week 2 action:** Export serialiser pulls `id` from `ContractIdentity`. Import deserialiser reads `id` and either matches or creates identity row.

---

### ODCS-GAP-004 · `status` (MANDATORY)

**ODCS path:** top-level  
**CG status:** Exists as `ContractVersion.state` (draft/stable/deprecated) in the DB, not in YAML.  
**Value mapping:**

| CG `VersionState` | ODCS `status` | Notes |
|-------------------|---------------|-------|
| `draft` | `proposed` | ODCS uses `proposed` for in-progress contracts |
| `stable` | `active` | Direct semantic equivalent |
| `deprecated` | `deprecated` | Direct match |
| _(none)_ | `retired` | ODCS `retired` = permanently decommissioned; no CG equivalent. Import as `deprecated`. |

**Round-trip risk:** MEDIUM. The `proposed` ↔ `draft` rename is the only lossy step. `retired` on import silently becomes `deprecated`.  
**Week 2 action:** Export layer derives `status` from `VersionState`. Import layer maps back using the table above. Document the `retired` → `deprecated` lossy conversion in the import warning log.

---

## Part C — ODCS Recommended Fields Assessment

| ODCS Field | Verdict | Rationale |
|------------|---------|-----------|
| `domain` | **Add (Week 3)** | Important for multi-team deployments. Add as an optional top-level CG native YAML field; export directly. |
| `dataProduct` | **Add (Week 3)** | Natural home for the CG contract `name` in ODCS. Add as optional field; map from/to `name` on import. |
| `tenant` | **Deferred** | Blocked on RFC-001 (tenancy model) sign-off. Wire the ODCS field to CG `tenant` once RFC-001 is implemented. |
| `schema[].physicalName` | **Add (Week 2)** | Needed for Kafka and dbt starter contracts to function correctly in ODCS tooling. Add optional field on `Contract` or per-schema config. |
| `schema[].physicalType` | **Add (Week 2)** | Same rationale as `physicalName`. Values: `table`, `topic`, `view`, `event`. |
| `schema[].properties[].classification` | **Add (Week 3)** | Complements PII transforms. Values: `public`, `internal`, `restricted`, `confidential`. CG infers this from `transform` presence but an explicit declaration is better. |
| `tags` | **Add (Week 3)** | Simple string array. Add optional `tags` field to `Contract`. Export directly. |
| `authoritativeDefinitions` | **Deferred** | Catalog integration (Collibra, Atlan, etc.). Low priority for v0.1.x. |
| `slaProperties` | **Deferred** | SLA metadata is a future roadmap item. No current CG data to populate this. |
| `support` | **Deferred** | Contact channels. Low urgency for Week 2. |
| `contractCreatedTs` | **Add (Week 2)** | Free — derive from `ContractVersion.created_at`. Zero cost to export. |

---

## Part D — Round-Trip Risk Register

The following issues, if unresolved before Week 2 implementation starts, will cause import→export→import round-trips to produce semantically different contracts.

| ID | Risk | Severity | Resolution Required By |
|----|------|----------|------------------------|
| RT-001 | `name` loses contract-level identity on export; `schema[0].name` on import may differ from original `name` if a consumer renames the dataset. | HIGH | Week 2 design |
| RT-002 | Validation constraints (`pattern`, `enum`, `min`, `max`, `min_length`, `max_length`) are lost on export unless `customProperties` extensions are written and read back. ODCS-native tools that strip unknown `customProperties` will destroy this data. | CRITICAL | Week 2 design |
| RT-003 | PII transforms are lost without `x-contractgate-transform-*` extensions. An ODCS consumer that strips unknown customProperties would produce a CG contract with no PII protection on re-import. | CRITICAL | Week 2 design + test coverage |
| RT-004 | Formula metrics are unrepresentable in ODCS without the `x-contractgate-metrics` top-level extension. | HIGH | Week 2 design |
| RT-005 | `compliance_mode: true` defaults to `false` on import if the extension key is absent. | HIGH | Week 2 design |
| RT-006 | Nested object properties must be flattened or represented as child schemas. Strategy choice affects round-trip fidelity. | MEDIUM | Structural decision before Week 2 |
| RT-007 | ODCS `status: retired` maps to CG `deprecated` on import — lossy. | LOW | Document in import log |
| RT-008 | ODCS quality rule `mustBeGreaterThan` is exclusive; CG `min` is inclusive. Off-by-one on round-trip if not handled. | LOW | Week 2 implementation detail |
| RT-009 | Foreign ODCS contracts (not originated from CG) lack `x-contractgate-*` extensions entirely. Import layer must never fail-hard on missing extensions; it must produce a valid (if reduced-fidelity) CG contract. | MEDIUM | Week 2 import error handling |
