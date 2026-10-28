# Changelog

All notable changes to the `contractgate` Python SDK.

## 0.4.1 — 2026-10-28

Version alignment with DataContractGate 0.4.1. No SDK API changes.

## 0.4.0 — 2026-10-13

- `ingest` now posts to `/v1/ingest/{contract_id}` (was the legacy
  `/ingest`), so each result carries `index` and `quarantine_id`.
- **Behavior change:** a batch where every event fails, or a rejected
  `atomic` batch, returns a `BatchIngestResponse` with per-event results and violations instead of
  raising `ValidationFailedError` on the gateway's 422. Only a 422 without
  per-event results (an idempotency conflict) still raises.
- `version` is sent as the `version` query parameter; the
  `"<uuid>@<version>"` contract-id form is still accepted.
- `IngestEventResult` gains `index` and `quarantine_id` (optional, default `None`).

## 0.3.2 — 2026-10-13

- `LLMGate` / `AsyncLLMGate`: `llm` is now optional. `validate_only()` never
  calls a provider, so it no longer needs a dummy client; `completion()` and
  `message()` raise `ValueError` when no `llm` is bound.

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
