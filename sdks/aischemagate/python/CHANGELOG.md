# Changelog

All notable changes to the `aischemagate` Python package.

## 0.5.0 — 2026-10-28

Optional framework adapters. The base install still does not import them.

- `aischemagate[langchain]`: `AISchemaGateParser`, a LangChain `BaseOutputParser`. `parse` checks the model text with `POST /egress/{contract_id}` (a contract name is resolved with `GET /contracts`) and returns the egress payload, or raises `OutputParserException`.
- `aischemagate[llamaindex]`: the same parser for LlamaIndex `BaseOutputParser`. A rejection raises `AISchemaGateRejected`.
- `aischemagate[instructor]`: `AISchemaGateHook.guard` runs that structural check before Pydantic field validation on the response model Instructor validates.
- Token streams are rejected. Streaming chunk validation is not included.

## 0.4.0 — 2026-10-13

Requires `contractgate >= 0.4.0`: `ingest` uses `/v1/ingest` and returns failed batches as results. `LLMGate` is unchanged.

## 0.3.2 — 2026-10-13

Requires `contractgate >= 0.3.2`: `llm` is optional for validate-only use.

## 0.3.1 — 2026-10-13

Versions now move in lockstep with `contractgate`. Requires `contractgate >= 0.3.1`,
which fixes contract-name lookup (Python) and returns failed validations as
`ok=False` results instead of raising.

## 0.1.0 — 2026-10-11

Initial release. Pure alias over `contractgate >= 0.3.0`.

- Re-exports every public symbol of `contractgate`.
- Re-exports `LLMGate`, `AsyncLLMGate`, `LLMGateResult`,
  `LLMResponseError`, `extract_json`, and `MAX_RETRIES_CAP` from
  `contractgate.llm` (RFC-094).
- No behavior of its own. Breaking changes are shipped in
  `contractgate`; this package tracks its minimum-version pin.
