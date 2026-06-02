# ODCS v3.1.0 Extension Strategy — ContractGate

**Status:** Week 1 deliverable  
**Date:** 2026-05-01  
**ODCS pin:** v3.1.0  
**Input:** field-mapping.md, gap-analysis.md

---

## 1. Why Extensions Are Needed

ContractGate is more expressive than ODCS in four areas that are core to its value proposition:

1. **Validation constraints** — regex patterns, enum allowlists, numeric bounds, string length bounds (enforced per-event at ingest time)
2. **PII transforms** — mask/hash/drop/redact operations with optional format-preserving style
3. **Formula metrics** — aggregate KPI definitions (`sum(amount) where event_type = 'purchase'`)
4. **Semantic glossary** — field synonyms, constraint prose, and business definitions as a first-class block

ODCS v3.1.0 has no structural home for any of these. The standard does provide two official extension points:

- **`customProperties[]`** — an array of `{property, value}` pairs available on most objects (schema entries, properties, quality rules). Suitable for per-field scalar extensions.
- **Top-level keys prefixed `x-`** — free-form extension keys at the document root. Used by tools like OpenAPI for vendor extensions. Suitable for block-level data structures.

ODCS also provides `customProperties` at the top level of the document, but its flat key-value structure is unwieldy for complex nested structures like glossary and metrics blocks.

This document defines the naming convention, placement rules, and round-trip contract for all ContractGate extensions.

---

## 2. Naming Convention

All ContractGate extension identifiers use the prefix **`x-contractgate-`** followed by a kebab-case name derived from the CG native YAML field path.

### Rules

1. Prefix: always `x-contractgate-`
2. Case: lowercase kebab-case after the prefix
3. Nesting: use hyphens, not dots, for path segments (e.g. `transform.kind` → `x-contractgate-transform-kind`)
4. Arrays: when the value is a complex structure, use a top-level `x-contractgate-*` key with the full array/object as value
5. Scalars: when the value is a scalar attached to an ODCS property, use `customProperties[].property: x-contractgate-*`

### Rationale for `x-contractgate-` prefix

- `x-` is the conventional vendor-extension prefix in YAML-based standards (OpenAPI, AsyncAPI, JSON Schema)
- `contractgate` is unambiguous and collision-resistant
- The prefix signals to ODCS-native tooling that these keys are safe to ignore
- The prefix enables CG import layer to detect extension keys reliably with a single prefix check

### Full Extension Identifier Registry

| CG Field | Extension Key | Placement | Value Type |
|----------|---------------|-----------|------------|
| `compliance_mode` | `x-contractgate-compliance-mode` | Top-level document key | bool |
| `ontology` (full block) | `x-contractgate-ontology` | Top-level document key | Ontology object (see §4) |
| `glossary` (full block) | `x-contractgate-glossary` | Top-level document key | GlossaryEntry[] |
| `metrics` (full block) | `x-contractgate-metrics` | Top-level document key | MetricDefinition[] |
| `multi_stable_resolution` | `x-contractgate-multi-stable-resolution` | Top-level document key | string (`strict`/`fallback`) |
| `promoted_at` | `x-contractgate-promoted-at` | Top-level document key | ISO 8601 datetime string or null |
| `deprecated_at` | `x-contractgate-deprecated-at` | Top-level document key | ISO 8601 datetime string or null |
| `entities[].pattern` | `x-contractgate-pattern` | `schema[].properties[n].customProperties[]` | string (regex) |
| `entities[].enum` | `x-contractgate-enum` | `schema[].properties[n].customProperties[]` | JSON array of values |
| `entities[].min` | `x-contractgate-min` | `schema[].properties[n].customProperties[]` | number |
| `entities[].max` | `x-contractgate-max` | `schema[].properties[n].customProperties[]` | number |
| `entities[].min_length` | `x-contractgate-min-length` | `schema[].properties[n].customProperties[]` | integer |
| `entities[].max_length` | `x-contractgate-max-length` | `schema[].properties[n].customProperties[]` | integer |
| `entities[].transform.kind` | `x-contractgate-transform-kind` | `schema[].properties[n].customProperties[]` | string (`mask`/`hash`/`drop`/`redact`) |
| `entities[].transform.style` | `x-contractgate-transform-style` | `schema[].properties[n].customProperties[]` | string (`opaque`/`format_preserving`) |
| `entities[].items` | `x-contractgate-items` | `schema[].properties[n].customProperties[]` | FieldDefinition object (JSON-serialised) |
| `glossary[].constraints` | `x-contractgate-constraints` | `schema[].properties[n].customProperties[]` | string |
| `glossary[].synonyms` | `x-contractgate-synonyms` | `schema[].properties[n].customProperties[]` | JSON array of strings |
| `metrics[].type` | `x-contractgate-metric-type` | quality rule `customProperties[]` | string (`integer`/`float`) |
| `metrics[].formula` | `x-contractgate-formula` | quality rule `customProperties[]` or top-level `x-contractgate-metrics` | string |
| foreign ODCS `id` (when different from CG UUID) | `x-contractgate-original-id` | Top-level document key | string (UUID) |

---

## 3. Placement Architecture

ContractGate extensions live at two levels:

### Level 1 — Top-level document keys (block data)

Used for CG-level constructs that are meaningless without their full internal structure.

```yaml
# Standard ODCS fields
apiVersion: v3.1.0
kind: DataContract
id: 53581432-6c55-4ba2-a65f-72344a91553a
version: 1.0.0
status: active
dataProduct: user_events
domain: product

# ContractGate block extensions
x-contractgate-compliance-mode: false
x-contractgate-multi-stable-resolution: strict
x-contractgate-ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"
      transform:
        kind: hash
x-contractgate-glossary:
  - field: user_id
    description: "Unique, stable identifier for a registered user"
    synonyms: ["uid", "userId"]
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative; maximum 1 000 000"
x-contractgate-metrics:
  - name: total_revenue
    formula: "sum(amount) where event_type = 'purchase'"
  - name: purchase_amount
    field: amount
    type: float
    min: 0.0
    max: 1000000.0
```

### Level 2 — `customProperties[]` on schema properties (scalar annotations)

Used for per-field constraints that annotate a specific `schema[].properties[]` entry.

```yaml
schema:
  - name: user_events
    physicalName: user_events
    physicalType: topic
    properties:
      - name: user_id
        logicalType: string
        required: true
        classification: restricted
        customProperties:
          - property: x-contractgate-pattern
            value: "^[a-zA-Z0-9_-]{3,64}$"
          - property: x-contractgate-transform-kind
            value: hash
      - name: amount
        logicalType: double
        required: false
        customProperties:
          - property: x-contractgate-min
            value: 0.0
          - property: x-contractgate-max
            value: 1000000.0
          - property: x-contractgate-enum
            value: null
```

---

## 4. The Canonical Round-Trip Strategy

ContractGate must support two export modes and one import mode.

### Export Mode A — ODCS Native (for interoperability)

Produces a document that ODCS-native tools can consume without modification. Extensions are present but ignorable.

**Steps:**
1. Write all mandatory ODCS fields: `apiVersion`, `kind`, `id`, `status`, `version`
2. Write recommended ODCS fields from CG data: `dataProduct` (← `name`), `domain` (if available), `contractCreatedTs`
3. Build `schema[0]` from `ontology.entities[]`: map `name`, `logicalType` (← `type`), `required`
4. For each entity, also emit ODCS-native approximations of constraints where possible: `quality[]` with `mustBeGreaterThan`/`mustBeLessThan` for `min`/`max`
5. For each entity, emit `description` from the matching glossary entry
6. Write all `x-contractgate-*` top-level extensions (complete, lossless)
7. Write all `x-contractgate-*` per-property `customProperties[]` (complete, lossless)
8. **Never export `pii_salt`**

### Export Mode B — ODCS Strict (future, Week 4)

Like Mode A but strips all `x-contractgate-*` keys. Used when producing a clean ODCS document for external audiences who should not see our extension keys. Inherently lossy — only use for documentation/publishing purposes, not for archival or re-import.

### Import (from ODCS, either origin)

**Steps:**
1. Read `x-contractgate-ontology` if present → restore `Contract.ontology` verbatim. **This is authoritative.**
2. If `x-contractgate-ontology` is absent (foreign ODCS doc): build `ontology.entities[]` from `schema[0].properties[]` using `name`, `logicalType` → `type`, `required`. Per-property `x-contractgate-*` `customProperties` are read back to restore constraints.
3. Read `x-contractgate-glossary` if present → restore `Contract.glossary` verbatim.
4. If absent: assemble glossary entries from `properties[].description` and `properties[].businessName`. Emit a warning that `constraints` and `synonyms` were not recoverable.
5. Read `x-contractgate-metrics` if present → restore `Contract.metrics` verbatim.
6. If absent: attempt to reconstruct field-bound metrics from `quality[]` rules. Formula metrics cannot be recovered — emit a warning.
7. Read `x-contractgate-compliance-mode` → restore `compliance_mode`. Default `false` if absent.
8. Read `x-contractgate-multi-stable-resolution` → restore value. Default `strict` if absent.
9. Map `status` → `VersionState` using the value table in gap-analysis.md §ODCS-GAP-004.
10. Use ODCS `id` as `ContractIdentity.id` if the document is being imported for the first time; if re-importing an export, the UUID should match.

**The invariant to test:** `roundtrip(contract)` where `roundtrip(c) = import(export(c))` must produce a contract that is functionally identical to `c` (same validation behaviour, same PII transforms, same glossary, same metrics).

---

## 5. What Happens if an ODCS Consumer Strips Unknown Keys

This is the main risk. ODCS-native tools (dbt-contracts, Soda, etc.) may read a CG-exported ODCS file and write it back out, stripping all `x-contractgate-*` keys.

**Consequence:** The stripped file, if re-imported into CG, will produce a contract with:
- No regex/enum validation constraints
- No PII transforms (CRITICAL compliance risk)
- No formula metrics
- Reduced glossary

**Mitigation:**
1. CG must detect stripped documents on import (absence of `x-contractgate-ontology` key).
2. When a stripped document is detected, CG import should emit a **conformance warning** listing exactly which fields could not be recovered.
3. CG should never silently import a stripped document as a replacement for an existing contract version. It should create a **new draft** and flag it for human review.
4. CG dashboard should display a "Reduced fidelity — review before promoting" badge on contracts imported from stripped ODCS documents.

---

## 6. Versioning the Extension Schema

The extension schema (the set of `x-contractgate-*` keys and their value shapes) is itself versioned. This prevents forward-compatibility issues when CG adds new extension fields.

**Convention:**

```yaml
x-contractgate-version: "1.0"
```

This top-level key declares the version of the CG extension schema used in this document. Import layer checks this field:
- If absent: assume v1.0 (first released version)
- If higher than current: emit a warning and attempt best-effort import

**Extension schema changelog:**
- `1.0` (Week 2): initial set as defined in this document

---

## 7. Full Annotated Example

The following shows a ContractGate `user_events` contract exported to ODCS v3.1.0 format with all extensions in place.

```yaml
# ── ODCS v3.1.0 mandatory ──────────────────────────────────────────────────
apiVersion: v3.1.0
kind: DataContract
id: 53581432-6c55-4ba2-a65f-72344a91553a
version: "1.0"
status: active
contractCreatedTs: "2026-04-28T00:00:00+00:00"

# ── ODCS recommended ───────────────────────────────────────────────────────
dataProduct: user_events
domain: product
description:
  purpose: "User behavior events — click, view, purchase, signup"

# ── ODCS schema (ODCS-native representation, lossy) ───────────────────────
schema:
  - name: user_events
    physicalName: user_events
    physicalType: topic
    description: "User behavior events"
    properties:
      - name: user_id
        logicalType: string
        required: true
        classification: restricted
        description: "Unique, stable identifier for a registered user in the system"
        quality:
          - metric: pattern
            type: custom
            description: "Must match ^[a-zA-Z0-9_-]{3,64}$"
            dimension: validity
            severity: error
        customProperties:
          - property: x-contractgate-pattern
            value: "^[a-zA-Z0-9_-]{3,64}$"
          - property: x-contractgate-min-length
            value: 3
          - property: x-contractgate-max-length
            value: 64
          - property: x-contractgate-transform-kind
            value: hash
      - name: event_type
        logicalType: string
        required: true
        description: "High-level category describing the action a user performed"
        customProperties:
          - property: x-contractgate-enum
            value: ["click","view","purchase","signup","logout"]
      - name: timestamp
        logicalType: integer
        required: true
        description: "Unix epoch timestamp (seconds) when the event occurred"
        quality:
          - metric: minValue
            mustBeGreaterThan: -1
            dimension: validity
            severity: error
        customProperties:
          - property: x-contractgate-min
            value: 0
      - name: amount
        logicalType: double
        required: false
        description: "Monetary amount in USD associated with a purchase event"
        quality:
          - metric: minValue
            mustBeGreaterThan: -0.001
            dimension: validity
            severity: error
          - metric: maxValue
            mustBeLessThan: 1000000.001
            dimension: validity
            severity: error
        customProperties:
          - property: x-contractgate-min
            value: 0.0
          - property: x-contractgate-max
            value: 1000000.0
          - property: x-contractgate-constraints
            value: "must be non-negative; maximum 1 000 000"

# ── ContractGate block extensions (lossless round-trip) ───────────────────
x-contractgate-version: "1.0"
x-contractgate-compliance-mode: false
x-contractgate-multi-stable-resolution: strict

x-contractgate-ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"
      min_length: 3
      max_length: 64
      transform:
        kind: hash
    - name: event_type
      type: string
      required: true
      enum: ["click","view","purchase","signup","logout"]
    - name: timestamp
      type: integer
      required: true
      min: 0
    - name: amount
      type: number
      required: false
      min: 0.0
      max: 1000000.0

x-contractgate-glossary:
  - field: user_id
    description: "Unique, stable identifier for a registered user in the system"
    synonyms: ["uid", "userId"]
  - field: event_type
    description: "High-level category describing the action a user performed"
    synonyms: ["action", "eventName"]
  - field: amount
    description: "Monetary amount in USD associated with a purchase event"
    constraints: "must be non-negative; maximum 1 000 000"

x-contractgate-metrics:
  - name: purchase_amount
    field: amount
    type: float
    min: 0.0
    max: 1000000.0
  - name: total_revenue
    formula: "sum(amount) where event_type = 'purchase'"
```

---

## 8. Decisions Required from Alex Before Week 2

The following decisions affect the Week 2 implementation directly. Each one has a recommended option.

| # | Decision | Options | Recommendation |
|---|----------|---------|---------------|
| D-001 | How to handle nested object fields (`type: object` with `properties[]`) on export | A: flatten to dot-notation names in `schema[].properties[]`; B: create a child `schema[]` object | **A (flatten)** — simpler, avoids multi-schema complexity. Preserve original structure in `x-contractgate-ontology`. |
| D-002 | On import of a foreign ODCS doc with no `x-contractgate-*` keys, should CG allow promotion to stable? | Allow with warning; Block entirely | **Block promotion, require human review.** Stripped documents should not silently become enforcing contracts. |
| D-003 | Should `x-contractgate-ontology` (the verbatim native block) always be written, or only when there is content that would otherwise be lost? | Always write; Write only when lossy | **Always write** — simpler import logic, clearer round-trip guarantee. Accept the verbosity. |
| D-004 | When exporting, should `name` populate `dataProduct`, `schema[0].name`, or both? | Both; only `dataProduct`; only `schema[0].name` | **Both** — maximises compatibility with ODCS consumers that look in different places. |
| D-005 | For the ODCS quality rules generated from CG `min`/`max`, should CG document the off-by-one (inclusive vs exclusive) explicitly in the export, or attempt to compensate with epsilon adjustment? | Document only; Epsilon compensate | **Document only** — epsilon adjustment introduces floating-point noise and is hard to reverse on import. The `x-contractgate-min/max` extensions carry the exact values. |
