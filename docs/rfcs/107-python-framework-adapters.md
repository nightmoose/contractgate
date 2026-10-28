# RFC-107 — Python framework adapters for AISchemaGate

**Status:** Implemented
**Date:** 2026-10-28
**Branch:** `cursor/python-framework-adapters-9a69`
**Depends on:** RFC-094 (`LLMGate` and the `aischemagate` package), RFC-029 (egress validation), RFC-093 (egress payload)

Streaming chunk validation is RFC-106 and stays Draft. This RFC does not implement it.

## Problem

`LLMGate.validate_only` already checks a finished JSON object with `POST /egress/{contract_id}` and resolves a contract name with `GET /contracts`. Callers using LangChain, LlamaIndex, or Instructor had to call that helper themselves, after the framework had already parsed or validated the text.

Instructor's Pydantic validation in particular reports field errors first. A payload that fails both the contract and the model surfaces the model error.

## Fix

Ship three optional extras on the `aischemagate` package. The root import does not load them.

- `aischemagate[langchain]` — `AISchemaGateParser`, a LangChain `BaseOutputParser`. `parse` returns the egress payload or raises `OutputParserException`.
- `aischemagate[llamaindex]` — the same parser for LlamaIndex `BaseOutputParser`. A rejection raises `AISchemaGateRejected`.
- `aischemagate[instructor]` — `AISchemaGateHook.guard` subclasses the response model with a Pydantic `mode='before'` validator. Instructor's OpenAI parser calls `model_validate_json` (other providers call `model_validate` or `model_validate_json`). Both run `mode='before'` validators before field validators, so this check runs first. A structural failure is the `ValidationError` Instructor wraps in `InstructorRetryException`. A passing check validates the egress payload.

All three call `LLMGate.validate_only`. No new gateway route. A token stream raises the same error `validate_only` already raises.

Instructor's `client.on("completion:response")` handlers cannot carry this check: that emitter swallows handler exceptions, so a failure there would not stop field validation.

## Testing

Unit tests in `sdks/aischemagate/python/tests/` use `httpx.MockTransport`. They cover the egress path, contract-name resolution, a rejected payload, invalid JSON, a stream (no HTTP call), and the Instructor ordering (field validator not called when the structural check fails; repaired payload is what Pydantic sees when it passes). `test_framework_interfaces.py` drives the same checks through LangChain's runnable chain, a LlamaIndex `PromptTemplate`, and `instructor.from_openai(...).chat.completions.create`.

## Rollout

Package version 0.5.0. `contractgate` is unchanged. No publish step in this change. Docs: the package README, `docs/integrations/python-framework-adapters.md`.
