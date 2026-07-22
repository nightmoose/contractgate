# RFC-087 — `null_as_absent` per-contract flag

**Status:** Implemented (backend) on branch; docs/dashboard follow-up
**Date:** 2026-07-21
**Depends on:** validation engine (`src/validation.rs`)
**Origin:** dogfood finding `ops/dogfood/findings/2026-07-20-null-optional-fields.md`

---

## Problem

Real open-data feeds routinely emit `"field": null` for sparse optional columns
(USGS `felt`/`alert`, GitHub `org_login`, NYC-311 `closed_date`). The validation
engine treats a **present** `null` as a value to type-check: for a field typed
`string`/`integer` with `required: false`, `null` → `type_mismatch`. Only an
**omitted** key skips validation for an optional field
(`validation.rs::validate_fields`, the `None` vs `Some(value)` arms).

Buyer intuition is "optional ≈ null or omit is fine," and many JSON producers
emit null rather than dropping the key. So the engine falsely rejects good data,
which reads as the tool being brittle — a UX and sales-honesty problem. During
dogfooding, sample batches failed at 100% until nulls were stripped from
fixtures.

## Decision

Add an **opt-in, per-contract** flag `null_as_absent` (default `false`).

- **Default (`false`, unchanged):** a present `null` is type-checked and fails a
  typed optional field. We keep catching nulls that a consumer may legitimately
  want flagged — no silent behavior change for existing contracts.
- **`true`:** a JSON `null` is treated **exactly like an omitted key**:
  - optional field + `null` → skipped (no violation);
  - required field + `null` → `missing_required_field` (not `type_mismatch`),
    for consistent "null means not there" semantics.

Rejected alternatives:

- **Change the default globally** — silently stops catching nulls everywhere and
  changes existing contracts' behavior. Too blunt; loses a real signal.
- **Docs only** ("omit, don't null") — cheapest, but the friction remains and
  producers can't always control null-vs-omit.

The flag is the honest middle: strict by default, explicit escape hatch, and we
can tell a buyer exactly what it does.

## Contract format

Top-level boolean on the contract YAML, alongside `compliance_mode` /
`egress_leakage_mode`:

```yaml
version: "1.0"
name: "usgs_earthquake"
null_as_absent: true      # treat JSON null on any field as if the key were omitted
ontology:
  entities:
    - name: felt
      type: integer
      required: false      # null or omit → skipped
```

`#[serde(default)]` → absent means `false`, so every existing contract is
unaffected.

## Implementation

`src/contract.rs::Contract` gains `pub null_as_absent: bool` (`#[serde(default)]`).
`validate_fields` / `validate_value` take a `null_as_absent: bool` threaded from
`validate()` (`compiled.contract.null_as_absent`). In `validate_fields`, a
present `Value::Null` is mapped to the absent branch when the flag is on:

```rust
let effective = match obj.get(&field.name) {
    Some(v) if null_as_absent && v.is_null() => None,
    other => other,
};
```

Nested objects and array items inherit the flag through the recursion. Egress
validation uses the same `validate()` entry, so it inherits the behavior for
free.

## Testing

`validation.rs` unit tests:
- strict (default): `null` on a typed optional → `type_mismatch`;
- `null_as_absent`: `null` on an optional → passes;
- `null_as_absent`: `null` on a required field → `missing_required_field`.

## Follow-up (not in this push)

- **Dashboard:** a "Treat null as absent" toggle in the contract editor / builder,
  and expose it on the contract read model.
- **Docs:** contract-format reference note + wizard copy.
- **Inference:** `infer_*` could set `null_as_absent: true` when a sample shows
  nulls on optional fields — deferred; default stays strict so inference output
  is honest about what it enforces.
