# Changelog

All notable changes to `@nightmoose/aischemagate`.

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
