# RFC-105 — ODCS v3.2 property enums, MCP dual-brand metadata, AISchemaGate guardrails

**Status:** Implemented
**Date:** 2026-10-23

## Problem

Four gaps showed up together:

1. The MCP registry listing described only DataContractGate ingest. A developer searching for AISchemaGate, structured-output validation, or quarantine replay did not see that `egress_validate` is already a tool. Smithery's default gateway URL pointed at the dashboard host.
2. ODCS v3.2.0 adds a property-level `enum` (a scalar list, or objects with `value`). Import had to accept that list and `apiVersion: v3.2.0` without changing the v3.1.0 export or the rule that `x-cg-enum` wins.
3. `@nightmoose/aischemagate` had `LLMGate` for OpenAI and Anthropic clients, and no helper for a Vercel AI SDK `wrapGenerate` flow that checks generated JSON with `POST /egress/{contract_id}` and feeds semantic violations back into a bounded repair.
4. There was no copy-pasteable recipe for a webhook receiver that forwards only the masked ingest payload, for gating a batch before a warehouse load, or for inspecting quarantine and replaying a group of rows.

## Fix

- Smithery's default `contractgateBaseUrl` is `https://api.datacontractgate.com`. `mcp/server.json` keeps the registry `description` within 100 characters. Keywords and the longer summary name both products. Technical identifiers (`CONTRACTGATE_*`, `cg_live_`, package names, `X-ContractGate-*` headers) are unchanged.
- `import_odcs` accepts `apiVersion` `v3.1.0` and `v3.2.0` (any other `v3.*` string still imports; a missing `apiVersion` still imports; a non-3.x value is rejected). Precedence for the field constraint is `x-cg-enum`, then the property `enum`, then `logicalTypeOptions.enum`, then a custom property named `enum`, then `invalidValues` / `validValues`. An empty `enum` array does not block a later `validValues` list. Scalar items and `{ value: ... }` items are both read. Integer and boolean values keep their types. Nested objects are not allowed values. Duplicate entries stay in order. Export still writes v3.1.0 and does not emit a property `enum` key.
- `@nightmoose/aischemagate` exports `aischemagateGuardrail` (structural `LanguageModelV4Middleware`, no `ai` dependency) and `validateStructuredOutput`. Both call `POST /egress/{contract_id}`. Auto-repair runs only for semantic violations (`min`, `max`, `pattern`, `enum`, `compliance_mode`) and is capped at 5 extra attempts. Malformed JSON does not call egress and does not start a repair loop.
- Docs: `docs/integrations/`. Recipes: `examples/recipes/webhook_quarantine_firewall.py`, `examples/recipes/pipeline_guardrail.py`, `examples/recipes/quarantine_replay.py`.

## Testing

- `cargo test` — ODCS v3.1 round-trips stay in `src/tests.rs`. Edge cases live in `src/odcs.rs`: both enum forms, empty arrays, mixed and duplicate values, `x-cg-enum` / property `enum` / `validValues` precedence, and `apiVersion` v3.1.0 and v3.2.0.
- `npm test` and `npm run build` in `mcp/`.
- `npm test` and `npm run build` in `sdks/aischemagate/typescript` (pass, semantic repair, max-retry exhaustion, malformed JSON).
- `python3 examples/recipes/<recipe>.py --self-check` for each recipe.

## Rollout

No migration. Export shape is unchanged. A foreign document that has both a property `enum` and `invalidValues.validValues` keeps the property `enum` unless `x-cg-enum` is present.
