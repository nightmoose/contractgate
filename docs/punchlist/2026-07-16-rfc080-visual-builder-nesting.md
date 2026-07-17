# Worklist 2026-07-16 — for Sonnet — RFC-080 Visual Builder nested objects

Full rationale + design: [`docs/rfcs/080-visual-builder-object-support.md`](../rfcs/080-visual-builder-object-support.md).
Frontend-only. One branch/PR. **Assumes RFC-079 has landed first** (same contracts UI area).

## Ground rules (CLAUDE.md + session-learned)

- **NO git operations of any kind.** Alex handles all git. Edit files, report.
- Be ultra-concise in comments; comment only non-obvious behavior.
- **Test before done:** `cd dashboard && npm run build` (tsc + build) must pass.
- **Preserve existing behavior byte-for-byte for flat contracts** — the scalar emit path must be unchanged; a flat contract must produce identical YAML to today. This is the #1 regression risk.
- This is an 848-line UI component rework — work carefully, keep the diff reviewable. If the file grows materially, flag it in your report (don't split it without asking; it's one cohesive component).
- Update `MAINTENANCE_LOG.md` when done.

## Target file

`dashboard/app/contracts/VisualBuilder.tsx`. Today it cannot express `type: object`:
its `FieldType` union is flat (`"string" | "integer" | "number" | "boolean" | "date"`, ~line 12), `FieldState` is flat (~line 15), `buildYaml` walks a flat list (~line 94), and `updateField`/`removeField`/`addField` key off a flat `id`.

## Steps (from RFC-080 design sketch)

1. **Type union** — add `"object"` to `FieldType`. Update `TYPE_COLORS` (~line 68) with a color for the new type. (Do NOT add `"array"` — out of scope this RFC.)

2. **`FieldState` becomes recursive** — add optional `properties?: FieldState[]`, populated only when `type === "object"`. Scalar-only fields (pattern, enum, min/max, transform) stay ignored for object fields, mirroring how `buildYaml` already gates by type.

3. **`defaultField()`** — unchanged default (string); when a user switches a field to `object`, initialize `properties: []`.

4. **Recursive field-editor render** — the per-field row (mapped ~line 761) must, for `object` fields, render a nested, indented editor for `properties` with its own "+ add field" and per-child remove. This is the bulk of the work. Visual affordance can cap at ~2–3 levels; deeper still works via YAML.

5. **Rewrite mutation helpers recursively** — `updateField`/`removeField`/`addField` currently match a flat `id` in `state.fields`. Replace with a recursive walk over `properties` (e.g. `updateFieldTree(fields, id, patch)`), or operate on a parent-path. Edits to a nested child must update the right node.

6. **`buildYaml` recursion** — factor the per-field emit into a function taking an indent level that recurses into `properties` for object fields, emitting `properties:` then indented children. Reuse the scalar emit logic (pattern/enum/min/max/transform) per level. Output must match what the Rust engine accepts and `validate_fields` resolves (see the locked contract YAML format in CLAUDE.md).

7. **Round-trip check** — the builder is YAML-out only (no inbound YAML→builder parser). Confirm this still holds; if so, no inbound parser change needed.

## Acceptance

- `cd dashboard && npm run build` passes.
- Build a contract with a nested `object` field (the `_cg` envelope: `source`, `doc_id`, `ingested_at`, `pii_redacted`) → emitted YAML is `type: object` + indented `properties`.
- Loop closed against the real engine: paste that YAML into the Playground and validate a matching record — it passes.
- **Regression:** an existing flat contract still builds and emits byte-identical YAML.

## Do NOT

- Change any backend/Rust — the engine already handles nested objects.
- Implement arrays-of-objects or arbitrary-depth UI polish (out of scope).
- Run any git command.
