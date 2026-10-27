# RFC-106 — Streaming chunk validation

**Status:** Draft
**Date:** 2026-10-27
**Branch:** `cursor/rfc-streaming-chunk-validation-4269`
**Depends on:** RFC-094 (LLM output guard), RFC-105 (AISchemaGate `wrapGenerate` guardrail), RFC-029 (egress validation), RFC-030 (egress PII and leakage), RFC-004 (PII transforms), RFC-044 (`date`), RFC-087 (`null_as_absent`)

This document is design only. It does not change the engine, the SDKs, or the dashboard. Implementation stays blocked until the UI mockup in [Rollout](#rollout) exists and the owner approves this RFC. Approval moves the status from Draft to Accepted. A later PR lands the code.

Owner review of the first draft asked for three additions, which this revision adopts: a stateful stream context on the gate and/or a local WASM/FFI parser; an `AISchemaGate.wrapStream` interceptor; and a progressive repair budget with a speculative JSON snapshot. Latency stays a list of goals, with no timing numbers.

---

## Problem

AISchemaGate checks a finished JSON object. Chat and agent UIs render an LLM reply as it arrives. Those two shapes do not meet.

A caller who asks for a stream is turned away. The practical workarounds are to buffer every token and call `POST /egress/{contract_id}` at the end, or to skip the gate on the streaming path. Buffering hides a violation that is already knowable: a closed number outside `min` / `max`, a closed string outside `enum`, a key the contract does not declare. The rest of the generation still runs, and the caller still waits for a round trip before seeing the failure.

The same gap sits on the Vercel AI SDK path. `aischemagateGuardrail` checks `generateText`. `streamText` has no middleware hook in that helper, so a streaming call never reaches egress.

Quarantine, repair prompts, and PII transforms are defined on a complete object (`src/egress.rs`, `src/transform.rs`). A chunk is not an object. This RFC says which checks may run on partial JSON, when the SDK stops the provider stream, and which side effects wait for a single terminal request.

## Current state

Verified in the tree this RFC was written against.

**Python `LLMGate` refuses `stream=True`.** In `sdks/python/src/contractgate/llm.py`, `LLMGate.completion` and `LLMGate.message` call `_reject_stream_kwarg` before the provider call. When the caller passes `stream=True`, that helper raises `LLMResponseError` with the message `streaming responses are not supported in v1 — pass stream=False`. `AsyncLLMGate.completion` and `AsyncLLMGate.message` call the same helper. `extract_json` raises the same error when the value is an iterator (`_is_streaming`). The tests that lock this refusal are `test_completion_stream_kwarg_is_rejected` and `test_extract_json_streaming_rejected` in `sdks/python/tests/test_llm_gate.py`.

**TypeScript `LLMGate` matches that refusal.** In `sdks/typescript/src/llm.ts`, `completion` and `message` throw `LLMResponseError` when `stream` is `true`. `extractJson` throws when the value is an async iterable. `sdks/typescript/tests/llm.test.ts` covers `stream: true`.

**The Vercel AI SDK guardrail implements `wrapGenerate` only.** In `sdks/aischemagate/typescript/src/ai.ts`, `AiSchemaGateMiddleware` is a structural `LanguageModelV4Middleware` with `specificationVersion: 'v4'` and a single method, `wrapGenerate`. `aischemagateGuardrail` returns that method. Its options object includes `doStream`, and the function never calls it. The comment on `aischemagateGuardrail` states that streaming is unchanged and that callers use a non-streaming generate call. `sdks/aischemagate/typescript/tests/guardrail.test.ts` asserts `wrapGenerate` is present. There is no `wrapStream`.

**Published package names.** The TypeScript package is `@nightmoose/aischemagate` (`sdks/aischemagate/typescript/package.json`). The Python package is `aischemagate` (`sdks/aischemagate/python/pyproject.toml`). `sdks/aischemagate/typescript/src/index.ts` re-exports `LLMGate` and `aischemagateGuardrail`. `sdks/aischemagate/python/src/aischemagate/__init__.py` re-exports `LLMGate`. Neither module exports `AISchemaGate.wrapStream` today.

**The engine validates a complete JSON value.** Egress is the shared DataContractGate and AISchemaGate route. `validate` in `src/validation.rs` walks a finished object: required fields, types, pattern, enum, range, length, format, `date` / `timestamp`, nested properties, array items, then metrics and per-event quality rules. `compliance_mode` reports `undeclared_field` for keys of the root object that are not in `declared_top_level_fields`. Nested extra keys are not an `undeclared_field` violation. Egress (`src/egress.rs`) runs that check, then `apply_egress_pii_pipeline`: RFC-004 transforms, then the leakage guard (`off` / `strip` / `fail`) on root keys. Failing records are quarantined when `dry_run` is false. Transforms in `src/transform.rs` run on complete top-level string fields (`mask`, `hash`, `drop`, `redact`). `hash` uses the per-contract `pii_salt`, which API responses do not return.

**Repair is a second full generation.** Python `LLMGate._run_loop` re-prompts on an egress failure or on non-JSON text. The default is two extra attempts (`_DEFAULT_MAX_RETRIES`) and the hard cap is five (`MAX_RETRIES_CAP`). The TypeScript guardrail uses the same numbers (`maxRetries` default 2, `AISG_MAX_REPAIR_ATTEMPTS`). `aischemagateGuardrail` re-prompts only for semantic kinds: `range_violation`, `pattern_mismatch`, `enum_violation`, `undeclared_field` (`SEMANTIC_VIOLATION_KINDS` in `sdks/aischemagate/typescript/src/ai.ts`). Malformed JSON does not start that loop. This RFC keeps those two policies.

RFC-094 listed streaming as a non-goal and pointed at a follow-up RFC. This is that follow-up.

## Goal

A caller passes an OpenAI `stream=True` iterator or an Anthropic `messages.stream()` handle to `AISchemaGate.wrapStream` and gets checks as each value closes, plus a JSON snapshot of the partial object a UI can parse before the model emits the final brackets.

**Done means:**

1. The Rust gate keeps a stream context per session. Each request carries the new chunk only. The gate appends that chunk to its parser. It does not receive the accumulated payload again.
2. The same parser is available as WASM and as an FFI library for edge and SDK execution. [Server session and local WASM/FFI](#4-server-session-and-local-wasmffi) records the tradeoff. Which of the two ships in the first implementation is an open question; the design covers both.
3. The parser tracks incomplete tokens (an unclosed string, an unclosed array or object, a number or literal still in progress) and builds a transient working AST. That AST is a side structure. The upstream provider stream is not rewritten.
4. Schema traversal runs when a value boundary closes, and it does not re-walk fields that have already closed. An enum or type violation fails the stream before the provider finishes the generation, subject to `onValidationError`.
5. `@nightmoose/aischemagate` exports `AISchemaGate.wrapStream(stream, { schema, onValidationError })`. PyPI `aischemagate` exports the Python equivalent `AISchemaGate.wrap_stream`. `onValidationError` is `'drop'`, `'throw'`, or `'repair'`. The wrapper is a drop-in around an OpenAI chat stream and around Anthropic `messages.stream()`.
6. After N unrepairable chunks the wrapper cancels the provider read and stops yielding, so later tokens are not consumed and downstream code does not keep running. N is `maxUnrepairableChunks`. This draft does not pick a default.
7. `partialJson()` (Python `partial_json()`) returns a string with trailing brackets and quotes closed on a copy of the working AST. `JSON.parse` / `json.loads` of that string succeeds. The copy is not the payload sent to egress, and closing it does not fire `missing_required_field`.
8. A complete object still ends at `POST /egress/{contract_id}`. That response is authoritative for pass/fail, transforms, audit, and quarantine. An incomplete abort writes one quarantine row. A chunk writes none.
9. Fields that declare a transform are withheld from yielded text. The caller sees the sanitized value from the egress payload.
10. Non-streaming `completion`, `message`, `wrapGenerate`, and `validateStructuredOutput` keep their current results.

## Non-goals

- A new contract field, rule type, or disposition. Streaming uses the deployed ontology as it is.
- Unifying the Python repair policy with the TypeScript semantic-kind policy.
- Validating free prose. The checker runs on the JSON object, including JSON inside a markdown fence once the fence has opened, which is the same extraction `extract_json` already performs on a finished string.
- Redacting text in the provider's own logs. The gate controls what it yields and what it stores.
- Batch uniqueness across generations. `uniqueness` stays a batch rule on the ingest path.
- Per-chunk metering. One generation attempt counts as one egress event, in line with RFC-083's event, not as one event per token.
- Changing `validate` on a complete object. The incremental rules are a prefix of that function. Where they disagree, the server result stands, and the disagreement is a bug in the port.

## Design

### 1. Working AST, incomplete tokens

Each stream has one context. The context holds:

- The parser stack and the current incomplete token (a string whose closing quote has not arrived, a number or literal still being spelled, or nothing).
- A transient working AST: objects and arrays that may still be open, strings that may still be open, and scalars that have closed.
- The set of paths whose value boundary has already been checked.
- The yield policy (transforms withheld, leakage fields withheld).

A chunk is appended to that context. The parser accepts a split inside a token, including inside a `\u` escape. Unclosed strings and unclosed arrays (and unclosed objects) stay open nodes. They are not errors.

The AST is built beside the provider stream. The wrapper does not write into the provider iterator, does not insert speculative brackets into the provider bytes, and does not replace a chunk the provider already emitted. Speculative completion, below, copies the AST and closes that copy.

The context lives in one of two places, [compared below](#4-server-session-and-local-wasmffi): in the gate process, keyed by a session id, or in the SDK/edge process inside the WASM/FFI parser. Either way the chunk that crosses the boundary is the new delta. The accumulated text is not sent again.

`stream` omitted or false keeps today's non-streaming methods. `AISchemaGate.wrapStream` is the opt-in.

### 2. What partial JSON can decide

A check runs only when its input can no longer change. The table matches `validate_value` and `validate_fields` in `src/validation.rs`.

| Check | When it runs on a stream | Kind |
|---|---|---|
| Type | As soon as the JSON type is fixed. `{` is an object, `[` is an array, `"` is a string (also `date` and `timestamp`), a completed `true` / `false` / `null` is that literal. A number waits until the token closes, because `1` may still become `1.5`. `FieldType::Any` accepts every type. | `type_mismatch` |
| Enum | When the scalar value closes, compare to `enum` (`allowed_values`). A prefix of an allowed string is not a failure. | `enum_violation` |
| Range | When the number token closes, compare to `min` / `max` inclusive. | `range_violation` |
| `max_length` | While a string is open, on the decoded byte length (`str::len` in the engine, not Unicode scalars). Length only grows, so crossing `max_length` is final. | `length_violation` |
| `min_length`, `pattern`, `format` (`email`, `uuid`, `uri` / `url`) | When the string closes. A prefix of `INV-000001` fails `^INV-[0-9]{6}$` if the pattern is applied early, so the pattern waits. Unknown `format` strings pass, as `string_format_ok` does. | `length_violation` or `pattern_mismatch` |
| `date`, `timestamp` | When the string closes. Calendar `YYYY-MM-DD` and RFC 3339, same as the engine. | `pattern_mismatch` |
| Required fields | When the object closes with `}`. Missing keys, and JSON `null` when `null_as_absent` is set, count as absent. Optional absent fields stay silent. Nested objects are judged at their own `}`, which may be before the root closes. | `missing_required_field` |
| Undeclared keys | When the key string closes, root object only, and only if `compliance_mode` is set. The kind is `undeclared_field`. Nested extra keys stay legal, because `validate` does not reject them today. | `undeclared_field` |
| Leakage | When a root key closes, if `egress_leakage_mode` is `strip` or `fail`. `strip` withholds the value and continues. `fail` is a violation (`leakage_violation`) and also strips, matching `apply_egress_pii_pipeline`. `off` ignores extra root keys unless `compliance_mode` already rejected them. | `leakage_violation` |

Quality rules wait for the terminal server check: `completeness`, `freshness` (it reads the gateway clock inside `validate`), and field-bound metrics (`metric_range_violation`). Formula metrics stay skipped. Running freshness in the client would use a different clock than `src/validation.rs` and could disagree with the row that gets stored.

Array items use the `items` schema when each item closes. The engine has no array length bounds; the streaming checker adds none.

The local checker is a port of those closed-value branches. It loads the deployed contract through the existing contract read API (the same document `aischemagateGuardrail` already resolves by UUID or name) and caches it on the gate instance for the pinned `version`, or for the current version when `version` is omitted. The cache stores constraints and transform kinds. It does not store `pii_salt`, and the client does not hash.

### 3. Fail fast versus buffer

Schema traversal is non-blocking in the sense that matters for a stream: when a value boundary closes, that field is checked before the next chunk is required, and fields that have already closed are not walked again. The traversal does not wait for the root `}` and does not call `validate()` on a reconstructed full document per chunk.

An enum violation or a type violation on that closed boundary fails the stream before generation finishes. The wrapper then follows `onValidationError` ([SDK interceptor](#5-sdk-interceptor)). Under `'throw'`, and under `'drop'` or `'repair'` once the unrepairable-chunk budget is spent, the wrapper cancels the provider iterator (`close`, `aclose`, or the AI SDK abort) so the model is not asked for the rest of the object and downstream code receives no further chunks.

Fail fast also applies when disposition is `block` or `fail` and any other row in the table has fired, or the parser has seen illegal JSON (a token that cannot be a prefix of a valid value, such as `tx`). An unclosed string, array, or object at a chunk boundary is a prefix, not illegal JSON, and not a reason to fail.

**Buffer** means: keep reading. Nothing is a violation yet.

Buffer applies to:

- Required fields of an object that is still open.
- Enum, range, `min_length`, pattern, format, `date`, and `timestamp` before the value closes.
- `max_length` until the decoded length exceeds the bound.
- Quality rules and metrics, until the terminal `POST /egress/{contract_id}`.
- The end of the provider stream. If the stream ends inside a value, the attempt is malformed JSON, which is the streaming form of today's `extract_json` failure. Required-field checks run only for objects the model actually closed. The speculative snapshot in [Progressive budget and speculative completion](#7-progressive-budget-and-speculative-completion) closes brackets on a copy for the UI. That copy is not the document those checks see.
- Disposition `tag`. Violations are collected and the stream runs to completion. The terminal egress result is the one the caller sees, including `tag`'s pass-through behavior from RFC-029.

A local failure on a **complete** object still performs the egress call. The server writes the same audit and quarantine row a non-streaming call would have written. The client does not skip that call because it already knows the answer.

A failure on an **incomplete** object uses session close (or the same close handler from a WASM/FFI context) instead of `POST /egress/{contract_id}`, because egress validates a JSON object and this buffer is not one.

### 4. Server session and local WASM/FFI

Both shapes run the same Rust stream context: append a delta, update the working AST, check paths whose value boundary just closed, return violations and a speculative snapshot. They differ in where that context lives.

**Server session.** One session per stream on the gate.

```
POST /egress/{contract_id}/sessions
{ "version": "optional pin", "disposition": "block" }
→ { "session_id": "..." }

POST /egress/{contract_id}/sessions/{session_id}/chunks
{ "delta": "<only the new bytes>" }
→ { "status": "continue" | "violation" | "closed", "violations": [], "partial_json": "{...}" }

POST /egress/{contract_id}/sessions/{session_id}/close
```

`delta` is appended to the session's parser. The body is not the concatenation of every chunk so far, and the handler does not re-parse from the start of the document. State held for the session is the stack, the incomplete token, the working AST, the paths already checked, and the unrepairable-chunk count. `close`, or a terminal `violation` under `'throw'`, writes at most one quarantine row and drops the context. `dry_run=true` skips the write. A complete object still goes through `validate` and `apply_egress_pii_pipeline` in `src/egress.rs` on close, which is the authoritative pass/fail, transform, audit, and quarantine result. `pii_salt` stays on the server.

**Local WASM/FFI.** The same context is a crate the SDK and the edge compile twice: `wasm32` for JavaScript hosts, and a C ABI for other languages. `session.push(delta)` returns the same status, violations, and `partial_json` the chunk route returns. The module does not contain `pii_salt` and does not write quarantine. When the object is complete, the SDK calls `POST /egress/{contract_id}` once. When the stream aborts incomplete, the SDK calls the same close handler the session uses, once, with the closed fields from the local context. Audit and transforms still happen once. Chunks are not uploaded.

A deployment may use either shape, or WASM/FFI for the per-chunk check and the gate only for the terminal write. The first implementation picks one or both; that choice is [open question 1](#open-questions).

| | Server session | Local WASM/FFI |
|---|---|---|
| Where the context lives | Gate process, one session per stream | SDK or edge process |
| What crosses the network per chunk | The new delta and the session id | Nothing for the check |
| Accumulated payload re-sent | No | No |
| Parser | The Rust stream context | The same Rust stream context, compiled |
| Enum or type failure before generation ends | The chunk response carries the violation. The wrapper cancels the provider. | `push` returns the violation. The wrapper cancels the provider. |
| `pii_salt`, hash, format-preserving mask | Applied on the terminal write. The salt never leaves the gate. | The module cannot hash. Transformed fields stay withheld until the terminal egress payload. |
| Quarantine | One row on session close or on early fail. Chunks write nothing. | One row on the terminal egress or partial write. Chunks write nothing. |
| Host with no WASM runtime | Works over HTTP. | Needs the WASM or FFI build. |
| Multi-instance gate | Session memory is local to the process that opened it. A later chunk has to reach that process, or the context has to move. | No gate session to pin. |
| Drift from `validate_value` | One binary. | Same crate, so the closed-value rules match the gate if the artifact is built from it. |
| Abandoned streams | The gate has to reap sessions that never `close`. | The SDK drops the context with the wrapper. |

### 5. SDK interceptor

The public interceptor is new on the packages that already ship. TypeScript: `@nightmoose/aischemagate` (`sdks/aischemagate/typescript/package.json`). Python: `aischemagate` (`sdks/aischemagate/python/pyproject.toml`).

```ts
import { AISchemaGate } from '@nightmoose/aischemagate';

const raw = await openai.chat.completions.create({
  model: 'gpt-4o',
  messages,
  stream: true,
});

const gated = AISchemaGate.wrapStream(raw, {
  schema: contractId, // deployed contract id or name
  onValidationError: 'throw', // 'drop' | 'throw' | 'repair'
});

for await (const chunk of gated) {
  // provider chunk shape, after transform holdback
}
JSON.parse(gated.partialJson());
```

```ts
const raw = anthropic.messages.stream({
  model: 'claude-sonnet-4-5',
  max_tokens: 1024,
  messages,
});
const gated = AISchemaGate.wrapStream(raw, {
  schema: contractId,
  onValidationError: 'repair',
});
```

`wrapStream` accepts the async iterable OpenAI returns for `stream: true` and the async iterable Anthropic returns from `messages.stream()`. It yields chunks in that same shape. It is a generator wrapper: the caller keeps the loop they already wrote.

Python equivalent, sync iterator or async iterator, on the PyPI package:

```python
from aischemagate import AISchemaGate

raw = client.chat.completions.create(model="gpt-4o", messages=messages, stream=True)
gated = AISchemaGate.wrap_stream(
    raw,
    schema=contract_id,
    on_validation_error="drop",  # "throw" | "repair"
)
for chunk in gated:
    ...
json.loads(gated.partial_json())
```

```python
raw = client.messages.stream(model="claude-sonnet-4-5", max_tokens=1024, messages=messages)
gated = AISchemaGate.wrap_stream(raw, schema=contract_id, on_validation_error="repair")
```

`wrap_stream` is the Python spelling of `wrapStream`. Both accept a sync generator and an async generator. `LLMGate.completion(..., stream=True)` and `LLMGate.message(..., stream=True)` become thin callers of `wrap_stream` after they open the provider stream. `stream` omitted still returns `LLMGateResult`. `extract_json` on a bare iterator still raises, so streaming goes through the wrapper.

`schema` is the deployed AISchemaGate contract (UUID or name), the same handle `aischemagateGuardrail` takes as `contractId`. An inline contract document is [open question 3](#open-questions).

`onValidationError` / `on_validation_error`:

| Mode | On a closed enum or type violation | After N unrepairable chunks |
|---|---|---|
| `throw` | Raise `AiSchemaGateRejectedError` (TypeScript) or `LLMResponseError` (Python), cancel the provider, stop the iterator. | Not used. The first such violation already stopped the stream. |
| `drop` | Do not yield the offending chunk. Keep reading. | Cancel the provider and end the iterator without raising. Downstream stops. `partialJson()` still reads. |
| `repair` | If the surface already repairs that kind, cancel, emit a reset, and start another generation within the existing generation cap (default two extra, hard cap five). If it does not repair that kind, treat the chunk as unrepairable. | Same as `drop`: cancel the provider, stop yielding. |

Yield rules for every mode:

- Deltas for a field with no transform are forwarded as they arrive.
- Deltas inside `mask`, `hash`, `drop`, or `redact` are not forwarded. Those transforms apply to top-level strings only (`src/transform.rs`). The sanitized value arrives with the terminal egress payload.
- The wrapper never writes speculative brackets into the yielded provider chunks.

**Vercel AI SDK.** `aischemagateGuardrail` in `sdks/aischemagate/typescript/src/ai.ts` gains a middleware `wrapStream` beside `wrapGenerate`. That method is an adapter: it reads text deltas and tool-call argument deltas from `doStream()` and feeds them to the same stream context as `AISchemaGate.wrapStream`. `wrapGenerate` stays the `generateText` path, including its repair loop. `streamText` callers pass the same `aischemagateGuardrail({...})` object. The helper stays free of a runtime `ai` dependency.

Repair on the middleware follows `SEMANTIC_VIOLATION_KINDS` and `maxRetries`, as `wrapGenerate` does today. A type mismatch or malformed JSON does not start a repair generation. Python `repair` follows `LLMGate._run_loop`: an egress failure or malformed JSON may re-prompt. The repair prompt is the existing `repair_prompt` / `repairHint`, sent as a user message on the next provider request. It is not inserted into the failed byte stream. The caller sees a reset, then deltas from the next attempt, and drops the partial text first.

### 6. Rust stream context

The context is a type in the gateway crate, used by the session handler and compiled into the WASM/FFI artifact. Sketch, not a frozen API:

```rust
struct StreamContext {
    stack: Vec<Frame>,
    pending: PendingToken, // none, string, number, or literal
    ast: Node,             // transient; not the provider buffer
    checked: HashSet<String>,
    unrepairable_chunks: u32,
}

enum Node {
    Object { fields: Vec<(String, Node)>, open: bool },
    Array { items: Vec<Node>, open: bool },
    String { value: String, open: bool },
    Number { raw: String, closed: bool },
    Bool(bool),
    Null,
}
```

`push(delta)` folds bytes. When a node transitions to closed, the walker checks that path against the compiled contract and records it in `checked`. Later chunks do not re-check it. Sibling fields that are still open are left alone. That is the non-blocking traversal: work is proportional to the new closure, and the read of the next chunk is not waiting on a full-document `validate()`.

The authoritative full `validate()` still runs once, when the root has actually closed: on session close, or on the terminal `POST /egress/{contract_id}` when the per-chunk checks ran in WASM/FFI. Disagreement between the incremental kinds and `validate()` for those kinds is a bug in the context.

The incomplete-abort write stores one quarantine row. The payload is `apply_transforms` of the closed fields only. An excerpt is stored only after spans that name a transformed field have been replaced with the transformed text. Required fields are not evaluated for objects the model left open. `direction` is `egress`. `dry_run=true` returns violations and writes nothing. A retried close uses `Idempotency-Key` and stores one row per key.

### 7. Progressive budget and speculative completion

**Unrepairable chunks.** A chunk is unrepairable when it closes a value into a violation this surface will not spend another generation on. On the TypeScript guardrail that includes `type_mismatch` and malformed JSON, which sit outside `SEMANTIC_VIOLATION_KINDS`. On Python `LLMGate` a type or enum failure can still start a repair generation until `MAX_RETRIES_CAP` (five) is exhausted; after that, further chunks are unrepairable. An incomplete token is not unrepairable.

`maxUnrepairableChunks` is N. This draft sets no default. The counter lives on the stream context.

- `'throw'` cancels the provider on the first enum or type violation and does not wait for N.
- `'drop'` withholds the offending chunk and keeps reading. At N it cancels the provider and ends the iterator without raising. Downstream execution stops. Tokens after that point are not read.
- `'repair'` spends a generation, within the existing cap (default two extra attempts, hard cap five), when the kind is repairable. The prompt is `repair_prompt` / `repairHint`, sent as a user message on the next provider call, not spliced into the failed stream. The caller sees a reset and drops the partial text. Unrepairable chunks count toward N. At N the wrapper cancels the provider and stops yielding, the same as `'drop'`.

Repair does not start until the failed provider stream is cancelled, so two provider streams are not open for one gate.

**Speculative completion.** `partialJson()` / `partial_json()` copies the working AST and serializes the copy:

1. An open string on the copy is closed with a quote.
2. Open arrays and objects on the copy are closed with `]` and `}`, from the inside out.
3. A trailing number or literal that is not yet valid JSON (`1.`, `-`, `tru`) is omitted from the copy. A number token that is already valid JSON (`12`) is included, and it stays unchecked until the real token closes.
4. The copy is not written back onto the parser AST, the provider stream, or the egress body.

`JSON.parse` / `json.loads` of that string succeeds, including while the model still has brackets open. Required-field checks do not run on the copy. They run when the model emits the real closer. The session chunk response carries the same string as `partial_json` so a server-session client and a WASM client expose one snapshot.

**Quarantine.** No chunk writes a row. A complete attempt writes through `POST /egress/{contract_id}`, including intermediate repair attempts that produced a complete object, because each such attempt already calls egress today. An incomplete abort writes one row on session close, as [the stream context](#6-rust-stream-context) specifies. `dry_run=true` writes neither.

**PII.** `apply_transforms` runs on a finished string. A prefix of an email is a different HMAC input than the full email, and a partial format-preserving mask is a different string than the mask of the finished value. The SDK therefore withholds a transformed field's deltas for the whole attempt. The yielded value is the one the terminal egress payload returns. `drop` yields nothing for that field. The client, including the WASM module, never computes `hash`.

Undeclared root fields under leakage `strip` or `fail` are withheld from the moment the key closes. `fail` also fail-fasts when disposition is `block` or `fail`.

The stored quarantine payload follows RFC-004: transformed fields, and under `compliance_mode` the undeclared keys removed, as `apply_transforms` already does before `audit_log` and `quarantine_events`.

Provider retention of the raw generation is outside the gate. The gate's guarantee is the one RFC-004 and RFC-030 already state for stored and forwarded payloads.

### 8. Latency goals

These are goals for the implementation once a streaming path exists. They are goals only. This draft states no timing number.

- Goal: a session chunk carries the new delta, not the bytes the context has already accepted.
- Goal: an enum or type violation is signaled on the chunk that closed the value, before the provider finishes the generation.
- Goal: the WASM/FFI path folds that same delta in-process, with no per-chunk gateway hop.
- Goal: when the unrepairable-chunk budget is spent, the wrapper stops reading the provider, so later tokens are not consumed and downstream code stops.
- Goal: the speculative snapshot is produced from the AST copy on that same chunk, so a UI can parse it before the model emits the closing brackets.
- Goal: salt-backed transforms and the quarantine write still happen on one terminal request per attempt, not on every chunk.

## Rollout

Status stays **Draft** until both of the following are true:

1. A UI mockup, reviewed by the owner, shows all seven moments below.
2. The owner approves this RFC, including whether the first implementation is the server session, the WASM/FFI parser, or both. The status line then changes to Accepted, in a docs follow-up or at the top of the implementation PR.

The mockup frames:

1. Deltas for a field with no transform appearing while the object is still open.
2. The stream stopping on a closed enum or type violation before the provider finishes, with the field, kind, and message visible.
3. Repair: the partial text is discarded, then a later attempt's deltas appear.
4. A field declared `redact` or `hash` stays hidden, then shows the sanitized egress value.
5. One quarantine row for the failed attempt, including an incomplete abort, and no row per chunk.
6. `partialJson()` parsing while the model has not yet emitted the closing brackets.
7. After N unrepairable chunks, the provider read stops and the downstream loop receives nothing further.

Until that approval, no engine, SDK, or dashboard change ships from this design.

After approval, the implementation order is:

1. The Rust `StreamContext` and shared fixtures of partial inputs. The gate tests and the SDK tests consume the same files.
2. The server session, the WASM/FFI artifact, or both, per the approval.
3. `AISchemaGate.wrapStream` in `@nightmoose/aischemagate` and `AISchemaGate.wrap_stream` in `aischemagate`, then the Vercel middleware adapter.
4. A reference page, `docs/streaming-validation-reference.md`, linked from `docs/llm-output-validation.md` and `docs/integrations/vercel-ai-sdk.md`, in the same PR as the user-facing API. That page is user-facing surface (new method and new routes) and waits for the implementation PR, per the contributor guide.

No migration. Existing callers that omit `stream` see no change. Callers that pass `stream=True` today already receive `LLMResponseError`; the implementation replaces that error with an iterator. That is a behavior change on a path that has no successful caller. The SDK minor version notes it. Tests that assert the v1 error are updated in the implementation PR: `sdks/python/tests/test_llm_gate.py`, `sdks/typescript/tests/llm.test.ts`, and `sdks/aischemagate/typescript/tests/guardrail.test.ts`.

`dry_run` and disposition defaults stay as they are. Contract documents are not rewritten.

## Testing

The implementation PR is expected to cover the following. This RFC adds none of these tests.

**Parser and closed-value rules** (fixtures shared with `src/validation.rs`):

- A string prefix of a pattern-matching value produces no `pattern_mismatch`. The closed string does, when it mismatches.
- `1` then `2` with `max: 10` produces no `range_violation` until the number token closes. A closed `12` does.
- `1` then `.5` on an integer field is `type_mismatch` only after the token closes.
- `max_length` fires while the string is still open, using decoded byte length, including when a chunk boundary falls inside an escape.
- A required key added before `}` is clean. The same object closed without that key is `missing_required_field`. A nested object's missing required field fires at the nested `}`, with the parent still open.
- `null_as_absent`: a closed `null` for a required field is reported at the object's `}`.
- `compliance_mode`: an unknown root key fires `undeclared_field` when the key closes. An unknown nested key does not.
- `egress_leakage_mode: strip` withholds the extra root field and does not fail the stream. `fail` records `leakage_violation`.
- Illegal JSON (`tx`) fails immediately. A chunk that ends mid-token does not.

**Parity.** For every fixture that is a complete object, the local checker and `validate()` return the same kinds for the checks in the table. A deliberate mismatch fails the test. Freshness and metrics are asserted only on the server result.

**Stream context.** Two deltas that together form one string are one token. The second request's body is only the new bytes. Pushing the concatenation a second time is a test that must fail the fixture, so a client cannot "work" by resending the accumulated payload. An unclosed string and an unclosed array survive a chunk boundary as open nodes. The provider buffer in the test double is unchanged after `push`.

**Speculative snapshot.** An AST with an open object and an open array serializes to a string `JSON.parse` accepts. The parser AST's open flags stay open after `partialJson()`. A missing required field is not reported from the snapshot. It is reported when the real `}` arrives.

**Progressive budget.** With `onValidationError: 'drop'` and N set by the test, the Nth unrepairable chunk cancels the provider double and the wrapper yields nothing further. With `'throw'`, the first enum or type violation cancels and raises. With `'repair'`, a semantic kind starts one new generation and a type mismatch on the TypeScript surface counts toward N.

**Interceptor.** `AISchemaGate.wrapStream` around an async iterable of OpenAI chat chunks, and around an async iterable shaped like Anthropic `messages.stream()`, yields the provider chunk type. `AISchemaGate.wrap_stream` does the same for a sync generator and an async generator. `stream` omitted on `LLMGate.completion` still returns `LLMGateResult`. `extract_json` on a bare iterator still raises.

**Vercel adapter.** Forwards a non-transformed text delta. Withholds a `redact` field until the egress payload is spliced in. Semantic failure with repair resets and re-enters the stream within the generation cap. Type mismatch does not re-prompt. `wrapGenerate` tests in `guardrail.test.ts` stay green.

**Quarantine and PII.** A complete failure produces one egress quarantine write. An incomplete enum failure produces one close-time write. Two chunks produce that single write, and neither chunk body is the full text. `dry_run=true` produces none. The stored payload for a `hash` field is the transformed form. The yielded text and the WASM module's output do not contain the raw field. An idempotent retry of close stores one row.

**Disposition `tag`.** A closed enum failure does not cancel the provider. The terminal result carries the violations.

**Latency.** A later benchmark may be read against the goals in [Latency goals](#8-latency-goals). The reference page is where that run is recorded, if the implementation adds one. This RFC records no timing result.

## Alternatives considered

**Resend the accumulated payload on every chunk.** The gate would parse the whole text so far on each call and keep no session. That avoids pinned session memory. It repeats work the context already did, and it makes "the new chunk" ambiguous. Set aside. Both the session and the WASM/FFI parser append a delta.

**A hand-ported parser in TypeScript and Python.** The SDK would reimplement `validate_value` for closed nodes. That ships with no WASM toolchain. It will drift from the Rust walker. Set aside in favor of compiling the same `StreamContext`. The comparison of running that crate on the gate versus in-process is [section 4](#4-server-session-and-local-wasmffi), not a rejection of either.

**Silent full buffer, then today's egress.** `stream=True` would collect every chunk and return `LLMGateResult`. RFC-094 rejected that as the v1 default because callers pass `stream=True` in order to render partial text. This RFC keeps incremental yield. The working AST is retained so the terminal request and the snapshot have a structure to read. The caller still receives deltas.

**Client checks only, no terminal egress.** The SDK would be the authority. That skips quarantine, audit, and transforms that need `pii_salt`. Set aside. A finished object still calls egress. An incomplete abort still writes one server row.

**Server check only, no incremental parser.** `stream=True` would forward every raw delta and call egress at the end. A bad enum would stay on screen until the model stops, and transformed fields would be painted raw. Set aside as the default. A caller who assembles the object and calls egress directly is unchanged.

**Regex-prefix rejection.** A pattern that cannot match any continuation of the open string could fail before the closing quote. It is easy to reject a prefix that a later character would have saved, and the engine does not do this on complete values. Out of scope. Patterns run when the string closes.

**Per-field sanitize RPC.** A closed transformed field would be sent to the gateway immediately so the UI can paint the mask before the object finishes. That spends a round trip per sensitive field. The yield rule withholds the field until the single terminal payload instead.

**Treat the speculative snapshot as the egress body.** Auto-closed brackets would make every partial object look complete, and required fields would fail for keys the model has not emitted yet. The snapshot is a UI view. Egress sees the model's bytes.

## Open questions

1. **Session, WASM/FFI, or both first.** The design covers both, and [section 4](#4-server-session-and-local-wasmffi) is the tradeoff. Approval of the mockup includes this choice. A host with no WASM runtime needs the session. A hot path that cannot spend a hop per delta needs the local parser. Using both (local check, gate only on close) is the third option.
2. **Default of `maxUnrepairableChunks`.** N is part of the API. This draft does not choose a number. `'throw'` does not consult N.
3. **`schema` argument.** The sketches pass a deployed contract id or name. Whether an inline contract document is also accepted is unset.
4. **Session placement.** In-memory state is local to the gate process that opened the session. What happens to a chunk that arrives on another instance, and how an abandoned session is reaped, is unset. No interval is chosen here.
5. **Trailing number in the snapshot.** A token that is already valid JSON (`12`) is included and not range-checked. Confirm that a UI prefers seeing `12` over omitting it until the token closes.
6. **Text and tool-call buffers together.** `extract_json` prefers an OpenAI content string, then tool-call arguments, then an Anthropic tool input. If a stream emits both, which buffer is the contract object while both are still growing? Proposal: follow that precedence, and parse only the preferred buffer once it has started receiving deltas.
7. **`'drop'` and quarantine.** `'drop'` ends the iterator at N without raising. The session still writes one quarantine row on close. Confirm the mockup's quarantine frame shows that row for a dropped stream.
8. **Idempotency window for session close.** The design requires `Idempotency-Key` and stores one row per key. How long the server remembers the key, and what a reused key with a different body returns, is unset.
9. **Repair transport for the Vercel adapter.** The adapter re-enters `doStream` so the retry stays streaming. Falling back to `doGenerate` for the retry is simpler and drops incremental checks on the attempt the user is most likely to see. Owner confirmation belongs on mockup frame 3.
