# RFC-105 — ODCS v3.2 property enums, MCP dual-brand metadata, AISchemaGate guardrails

**Status:** Implemented
**Date:** 2026-10-23

## Problem

Four gaps showed up together:

1. The MCP registry listing (`mcp/server.json`, npm, Smithery, Glama) described only DataContractGate ingest. Developers searching for AISchemaGate, LLM output validation, structured outputs, or guardrails did not find the server, even though `egress_validate` is already a tool. Smithery's default gateway URL pointed at the dashboard host.
2. ODCS v3.2.0 adds a property-level `enum` (scalar strings, or objects with `value` and `title`). Import understood the object form in some fixtures, but a quality `invalidValues` rule and a property `enum` were not ordered so the quality rule wins, and `apiVersion` was not explicitly accepted for both v3.1.0 and v3.2.0.
3. `@nightmoose/aischemagate` had `LLMGate` for OpenAI and Anthropic clients, and no helper for Vercel AI SDK `wrapGenerate` / `generateObject` flows that feeds semantic violations (`min`, `max`, `pattern`, `enum`, `compliance_mode`) back into a bounded repair.
4. There was no copy-pasteable recipe for a webhook receiver that forwards only the masked ingest payload, or for gating a batch before a DuckDB / MotherDuck write, or for Pydantic / Instructor / LangGraph.

## Fix

- Smithery's default `contractgateBaseUrl` is `https://api.datacontractgate.com`. Registry descriptions and npm keywords name DataContractGate and AISchemaGate. Technical identifiers (`CONTRACTGATE_*`, `cg_live_`, package names) are unchanged.
- `import_odcs` accepts `apiVersion` `v3.1.0` and `v3.2.0` (any other `v3.*` string still imports; non-3.x is rejected). A property-level `enum` maps to the field `enum` only when an `invalidValues` rule has not already set `validValues`. Export still writes v3.1.0 so documents stay valid against the vendored v3.1.0 JSON Schema.
- `@nightmoose/aischemagate` exports `createAISchemaGateMiddleware` (structural `LanguageModelV4Middleware`, no `ai` dependency) and `validateOrRepairObject`. Both call `client.egress` (`POST /egress/{contract_id}`).
- Docs: `docs/integrations/vercel-ai-sdk.md`, `docs/integrations/pydantic-instructor-langgraph.md`. Recipes: `examples/recipes/webhook-quarantine-gate/`, `examples/recipes/dagster-prefect-guardrail/`.

## Testing

- `cargo test` — existing ODCS v3.1 round-trips plus new cases in `src/odcs.rs` for both enum forms, quality-rule precedence, and `apiVersion: v3.2.0`.
- `npm test` and `npm run build` in `mcp/`.
- `npm test` and `npm run typecheck` in `sdks/aischemagate/typescript` (pass, repair, exhaustion).
- `python app.py --self-check` and `python gate.py --self-check` for the recipes.

## Rollout

No migration. Export shape is unchanged. Import of a document that has both a property `enum` and `invalidValues.validValues` now keeps the quality-rule values.
