# Changelog

All notable changes to `@nightmoose/contractgate-sdk`.

## 0.4.0 — 2026-10-13

- `ingest` now posts to `/v1/ingest/{contract_id}` (was the legacy
  `/ingest`), so each result carries `index` and `quarantine_id`.
- **Behavior change:** a batch where every event fails, or a rejected
  `atomic` batch, resolves to a `BatchIngestResponse` with per-event results and violations instead of
  throwing `ValidationFailedError` on the gateway's 422. Only a 422 without
  per-event results (an idempotency conflict) still throws.
- `version` is sent as the `version` query parameter; the
  `"<uuid>@<version>"` contract-id form is still accepted.
- `IngestEventResult` gains `index` and `quarantine_id` (`number | null`, `string | null`).

## 0.3.2 — 2026-10-13

- `LLMGate`: `llm` is now optional. `validateOnly()` never calls a provider, so
  it no longer needs a dummy client; `completion()` and `message()` throw
  `TypeError` when no `llm` is bound.

## 0.3.1 — 2026-10-13

Fixes from end-to-end testing against production. No API changes.

- Fix: a rejected LLM output now resolves to `{ ok: false, violations: [...] }`
  instead of throwing `ValidationFailedError`. The gateway answers 422 when every
  event fails, which is always true for a gate's single-event batch, so
  `validateOnly` and the `autoRepair` loop never saw the violations. A 422
  without an egress body (e.g. a deprecated version pin) still throws.
- Default `baseUrl` is `https://api.datacontractgate.com` (was the dashboard host).

## 0.3.0 — 2026-10-11

RFC-094. Adds the `./llm` subpath export for LLM output validation. No
breaking changes to the main entry point.

- New: `@nightmoose/contractgate-sdk/llm` exports `LLMGate`,
  `LLMResponseError`, `LLMGateResult`, `extractJson`, `MAX_RETRIES_CAP`.
- `LLMGate` wraps `/egress/{contract_id}` with response-shape extraction
  and a bounded auto-repair re-prompt loop. Caller binds their own LLM
  client (OpenAI / Anthropic / Ollama / vLLM) via `llm`; the SDK takes
  no runtime dependency on provider SDKs.
- `extractJson(response)` handles OpenAI chat JSON mode, OpenAI
  tool_calls, OpenAI Responses `output_parsed`, Anthropic `tool_use`
  and text-block JSON, raw strings (with markdown-fence tolerance),
  and plain-object passthrough.
- Streaming responses are rejected in v1 (`LLMResponseError`).
- `LLMGate.maskPrompt(payload, { promptContractId })` returns the
  RFC-004-transformed form of a prompt dict via
  `ingest({ dryRun: true })`.
- Sister alias package `@nightmoose/aischemagate` ships at the same cut.

## 0.2.0 — 2026-09-30

RFC-018 initial launch of the TypeScript SDK.

- `Client` with `ingest`, `egress`, `audit`, `stats`, contract/version
  reads, playground.
- Local validator mirroring the Rust engine (parity-locked via shared
  fixture corpus).
- Error hierarchy mapped from HTTP status.
