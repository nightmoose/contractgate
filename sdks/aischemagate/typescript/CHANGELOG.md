# Changelog

All notable changes to `@nightmoose/aischemagate`.

## 0.4.1 — 2026-10-28

Requires `@nightmoose/contractgate-sdk >= 0.4.1`.

- `aischemagateGuardrail` is a Vercel AI SDK `LanguageModelV4Middleware` (`wrapGenerate`) that validates structured JSON with `POST /egress/{id}` and, when `autoRepair` is set, re-prompts on semantic violations (`min`, `max`, `pattern`, `enum`, `compliance_mode`). `validateStructuredOutput` is the same check without a model. No new runtime dependency.

## 0.4.0 — 2026-10-13

Requires `@nightmoose/contractgate-sdk >= 0.4.0`: `ingest` uses `/v1/ingest` and returns failed batches as results. `LLMGate` is unchanged.

## 0.3.2 — 2026-10-13

Requires `@nightmoose/contractgate-sdk >= 0.3.2`: `llm` is optional for validate-only use.

## 0.3.1 — 2026-10-13

Versions now move in lockstep with `@nightmoose/contractgate-sdk`. Requires `@nightmoose/contractgate-sdk >= 0.3.1`,
which fixes contract-name lookup (Python) and returns failed validations as
`ok=False` results instead of raising.

## 0.1.0 — 2026-10-11

Initial release. Pure alias over `@nightmoose/contractgate-sdk >= 0.3.0`.

- Re-exports every public symbol of `@nightmoose/contractgate-sdk`
  (main entry).
- Re-exports `LLMGate`, `LLMResponseError`, `extractJson`,
  `MAX_RETRIES_CAP`, and the `LLMGateResult` / `LLMGateOptions` /
  `CompletionOptions` / `MessageOptions` types from
  `@nightmoose/contractgate-sdk/llm` (RFC-094).
- No behavior of its own. Breaking changes are shipped in the
  underlying SDK; this package tracks its minimum-version pin.
