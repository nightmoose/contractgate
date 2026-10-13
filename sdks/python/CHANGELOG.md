# Changelog

All notable changes to the `contractgate` Python SDK.

## 0.3.1 — 2026-10-13

Fixes from end-to-end testing against production. No API changes.

- Fix: `LLMGate(contract_id="<name>")` crashed with `KeyError: 'created_at'`.
  `GET /contracts` returns summary rows without timestamps;
  `ContractResponse.created_at` / `updated_at` are now optional.
- Fix: a rejected LLM output now returns `LLMGateResult(ok=False, violations=[...])`
  instead of raising `ValidationFailedError`. The gateway answers 422 when every
  event fails, which is always true for a gate's single-event batch, so
  `validate_only` and the `auto_repair` loop never saw the violations. A 422
  without an egress body (e.g. a deprecated version pin) is still raised.
- Default `base_url` is `https://api.datacontractgate.com` (was the dashboard host).

## 0.3.0 — 2026-10-11

RFC-094. Adds the `contractgate.llm` submodule for LLM output
validation. No breaking changes.

- New: `contractgate.llm.LLMGate` and `contractgate.llm.AsyncLLMGate`
  wrap `/egress/{contract_id}` with response-shape extraction and a
  bounded auto-repair re-prompt loop. Caller binds their own LLM
  client (OpenAI / Anthropic / Ollama / vLLM) via `llm=...`; the SDK
  takes no runtime dependency on provider SDKs.
- New: `contractgate.llm.extract_json(response)` handles OpenAI chat
  JSON mode, OpenAI tool_calls, OpenAI Responses `output_parsed`,
  Anthropic `tool_use` and text-block JSON, raw strings (with
  markdown-fence tolerance), and dict passthrough.
- New: `contractgate.llm.LLMResponseError` for streaming responses
  (rejected in v1) and unparseable shapes.
- New: `LLMGate.mask_prompt(payload, prompt_contract_id=...)` returns
  the RFC-004-transformed form of a prompt dict via
  `ingest(dry_run=True)`.
- Sister alias package `aischemagate` ships at the same cut.

## 0.1.0 — 2026-04-28

Initial scaffold per RFC-005.

- `Client` (sync) + `AsyncClient` (async) over httpx.
- `ingest`, `audit`, `get_contract`, `get_version`, `playground`.
- Local validator (`Contract.from_yaml`, `CompiledContract.validate`).
- Strict parity with Rust validator: same `ViolationKind`, same field
  paths, same message text. Locked via shared fixture corpus.
- No PII transforms in the local validator (RFC-004 invariant).
