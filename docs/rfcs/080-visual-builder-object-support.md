# RFC-080 — Visual Builder nested-object support

**Status:** Implemented
**Date:** 2026-06-08
**Branch:** TBD
**Depends on / follows:** RFC-079 (unified inference on the Rust engine —
deferred this as Workstream 2)
**Severity:** P2 — UI gap; the RAG envelope is buildable via YAML and inferable
via Generate-from-Sample (post-RFC-079), just not via the Visual Builder.

---

## Problem

The Visual Builder (`dashboard/app/contracts/VisualBuilder.tsx`) cannot express
a `type: object` field with nested `properties`. Concretely:

- Its type union is flat: `type FieldType = "string" | "integer" | "number" |
  "boolean" | "date"` (line 12) — no `"object"` (and no `"array"`).
- `FieldState` (line ~15) is a flat record with no concept of child fields.
- `buildYaml(state)` (line ~94) walks a single, flat `state.fields` list and
  emits one level of entities.
- `updateField` / `removeField` key off a flat `id` against `state.fields`.

So the RFC-077 RAG envelope (`_cg` as a nested object with `source`, `doc_id`,
`ingested_at`, `pii_redacted`) **cannot be constructed in the Visual Builder at
all**. The only UI path to a nested contract today is hand-writing YAML in Start
Blank; post-RFC-079, Generate-from-Sample also produces it correctly via the
Rust engine.

This is the third and last surface in the "assisted contract creation" set that
lacks nesting (CSV is inherently flat; Generate-from-Sample fixed in RFC-079).

## Goal

Let a user build a `type: object` field with nested child fields in the Visual
Builder, and emit correct `type: object` + `properties` YAML — matching what the
Rust engine accepts and what `validate_fields` resolves.

## Non-goals

- Arrays of objects (`type: array` with object `items`). Possible follow-up;
  this RFC covers `object` nesting only, which is what the RAG envelope needs.
- Arbitrary nesting depth UI polish. Support recursion correctly, but the first
  cut can cap the *visual* affordance at 2–3 levels; deeper still works via YAML.
- Any backend change. The engine already handles nested objects.

## Design sketch

This is a genuine builder rework, not an additive flag. The pieces:

1. **Type union** — add `"object"` to `FieldType`. (Consider `"array"` too, but
   see non-goals.) Update `TYPE_COLORS` (line ~68) for the new type.

2. **`FieldState` becomes recursive** — add an optional
   `properties?: FieldState[]` populated only when `type === "object"`. All the
   scalar-specific fields (pattern, enum, min/max, transform) stay ignored for
   object-typed fields, mirroring how `buildYaml` already gates them by type.

3. **`defaultField()`** — unchanged default (string); when a user switches a
   field to `object`, initialize `properties: []`.

4. **Field-editor render becomes recursive** — the per-field row (mapped at line
   ~761) must, for `object` fields, render a nested, indented editor for
   `properties` with its own “+ add field” and per-child remove. This is the
   bulk of the work: the current render assumes a flat list.

5. **`updateField` / `removeField` / `addField`** — these currently match a flat
   `id` in `state.fields`. They need to operate on a path (parent chain) or be
   rewritten recursively so edits to a nested child update the right node.
   Simplest robust approach: a recursive `updateFieldTree(fields, id, patch)`
   that walks `properties`.

6. **`buildYaml` recursion** — factor the per-field emit into a function that
   takes an indent level and recurses into `properties` for object fields,
   emitting `properties:` then the indented children. The scalar emit logic
   (pattern/enum/min/max/transform) is reused per level.

7. **Round-trip note** — the builder is YAML-out only (it doesn't parse existing
   YAML back into builder state), so no inbound parser change is needed. Confirm
   that assumption still holds before implementing.

## Why this is deferred (not done in RFC-079)

RFC-079 fixed the actual reported bug (Generate-from-Sample emitting
`"[object Object]"`) by routing inference to the Rust engine — a small, safe,
type-clean change. This builder rework is the opposite risk profile: it touches
the field data model, three mutation helpers, the recursive render, and YAML
emission in an 848-line UI component, and it needs visual + `npm run build`
verification that can't be done blind. CLAUDE.md explicitly cautions against
casual frontend overhauls, so it earns its own RFC and a focused session.

## Testing

- `cd dashboard && npm run build` passes (maintainer-run).
- Manual: build a contract with a nested `object` field (the `_cg` envelope);
  confirm emitted YAML is `type: object` + indented `properties`, and that
  pasting that YAML into the Playground validates a matching record (closes the
  loop against the real engine).
- Regression: existing flat contracts still build and emit byte-identical YAML
  (the scalar emit path must be unchanged for non-object fields).

## Rollout

Frontend-only (Vercel). No migration, no backend deploy, no API change.
Backward compatible: flat contracts unaffected; object support is additive.
