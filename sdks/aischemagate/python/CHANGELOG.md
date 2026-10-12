# Changelog

All notable changes to the `aischemagate` Python package.

## 0.1.0 — 2026-10-11

Initial release. Pure alias over `contractgate >= 0.3.0`.

- Re-exports every public symbol of `contractgate`.
- Re-exports `LLMGate`, `AsyncLLMGate`, `LLMGateResult`,
  `LLMResponseError`, `extract_json`, and `MAX_RETRIES_CAP` from
  `contractgate.llm` (RFC-094).
- No behavior of its own. Breaking changes are shipped in
  `contractgate`; this package tracks its minimum-version pin.
