# RFC-106 — Streaming chunk validation

**Status:** Draft
**Date:** 2026-10-27
**Branch:** `cursor/rfc-streaming-chunk-validation-4269`
**Depends on:** RFC-094 (LLM output guard), RFC-105 (AISchemaGate `wrapGenerate` guardrail), RFC-029 (egress validation), RFC-030 (egress PII and leakage), RFC-004 (PII transforms), RFC-044 (`date`), RFC-087 (`null_as_absent`)

This document is design only. It does not change the engine, the SDKs, or the dashboard. Implementation stays blocked until the UI mockup in [Rollout](#rollout) exists and the owner approves this RFC. Approval moves the status from Draft to Accepted. A later PR lands the code.

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

**The engine validates a complete JSON value.** Egress is the shared DataContractGate and AISchemaGate route. `validate` in `src/validation.rs` walks a finished object: required fields, types, pattern, enum, range, length, format, `date` / `timestamp`, nested properties, array items, then metrics and per-event quality rules. `compliance_mode` reports `undeclared_field` for keys of the root object that are not in `declared_top_level_fields`. Nested extra keys are not an `undeclared_field` violation. Egress (`src/egress.rs`) runs that check, then `apply_egress_pii_pipeline`: RFC-004 transforms, then the leakage guard (`off` / `strip` / `fail`) on root keys. Failing records are quarantined when `dry_run` is false. Transforms in `src/transform.rs` run on complete top-level string fields (`mask`, `hash`, `drop`, `redact`). `hash` uses the per-contract `pii_salt`, which API responses do not return.

**Repair is a second full generation.** Python `LLMGate._run_loop` re-prompts on an egress failure or on non-JSON text. The default is two extra attempts (`_DEFAULT_MAX_RETRIES`) and the hard cap is five (`MAX_RETRIES_CAP`). The TypeScript guardrail uses the same numbers (`maxRetries` default 2, `AISG_MAX_REPAIR_ATTEMPTS`). `aischemagateGuardrail` re-prompts only for semantic kinds: `range_violation`, `pattern_mismatch`, `enum_violation`, `undeclared_field` (`SEMANTIC_VIOLATION_KINDS` in `sdks/aischemagate/typescript/src/ai.ts`). Malformed JSON does not start that loop. This RFC keeps those two policies.

RFC-094 listed streaming as a non-goal and pointed at a follow-up RFC. This is that follow-up.

## Goal

A caller who sets `stream=True`, or who wraps a Vercel AI SDK model and calls `streamText`, gets incremental checks while tokens arrive, and one authoritative server check when the generation attempt ends.

**Done means:**

1. Closed values are checked locally for type, enum, range, length, pattern, format, `date`, and `timestamp`. Required fields are checked when their object closes. Undeclared root keys are checked when the key closes, under the same flags the engine uses today.
2. A definitive local violation stops the provider stream when disposition is `block` or `fail` (the gateway default is `block`). Disposition `tag` keeps reading and reports violations on the terminal result.
3. `aischemagateGuardrail` gains `wrapStream` beside the existing `wrapGenerate`.
4. Python `LLMGate` and `AsyncLLMGate` accept `stream=True` on `completion` and `message` and return a sync iterator or an async iterator.
5. Every generation attempt that produces a complete JSON object still calls `POST /egress/{contract_id}`. That response is authoritative for pass/fail, transforms, audit, and quarantine.
6. An attempt aborted before the object closes writes one partial quarantine row, through the route in [Quarantine, repair, and PII](#7-quarantine-repair-and-pii). A chunk writes no row of its own.
7. Fields that declare a transform are withheld from yielded text. The caller sees the sanitized value from the egress payload.
8. Repair, when the surface already repairs that violation kind, cancels the current stream, tells the caller to discard the partial text, and starts a new generation within the existing attempt cap.
9. Non-streaming `completion`, `message`, `wrapGenerate`, and `validateStructuredOutput` keep their current results.

## Non-goals

- A new contract field, rule type, or disposition. Streaming uses the deployed ontology as it is.
- Unifying the Python repair policy with the TypeScript semantic-kind policy.
- Validating free prose. The checker runs on the JSON object, including JSON inside a markdown fence once the fence has opened, which is the same extraction `extract_json` already performs on a finished string.
- Redacting text in the provider's own logs. The gate controls what it yields and what it stores.
- Batch uniqueness across generations. `uniqueness` stays a batch rule on the ingest path.
- Per-chunk metering. One generation attempt counts as one egress event, in line with RFC-083's event, not as one event per token.
- Changing `validate` on a complete object. The incremental rules are a prefix of that function. Where they disagree, the server result stands, and the disagreement is a bug in the port.

## Design

### 1. Two buffers, one decision

The SDK keeps a parser buffer and a yield buffer.

The parser buffer is the decoded JSON text of the current attempt (response text, or tool-call arguments — see [Open questions](#open-questions) when both grow). It feeds an incremental JSON parser. The parser's state (stack, current key, in-string, escape, number token) survives arbitrary chunk splits, including a split inside a `\u` escape.

The yield buffer is what the caller is allowed to see. It is a view of the parser buffer with transformed fields and not-yet-closed leakage fields removed. The SDK retains the full parser buffer until the attempt ends so the terminal egress call can send the object. That retention is internal. The caller receives deltas as the yield rules below allow. RFC-094's rule stands: `stream=True` does not mean "buffer the whole reply and return it as a normal `LLMGateResult`."

`stream` omitted or false keeps today's non-streaming methods. Passing `stream=True` is the opt-in.

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

**Fail fast** means: the attempt's outcome is already a violation, so the SDK cancels the provider iterator (`close` on a sync iterator, `aclose` on an async iterator, or the equivalent abort on the AI SDK stream) and stops yielding data deltas.

Fail fast applies when disposition is `block` or `fail` and any row in the table above has fired, or the parser has seen illegal JSON (a token that cannot be a prefix of a valid value, such as `tx` where a literal was starting). An unclosed string at a chunk boundary is a prefix, not illegal JSON.

**Buffer** means: keep reading. Nothing is a violation yet.

Buffer applies to:

- Required fields of an object that is still open.
- Enum, range, `min_length`, pattern, format, `date`, and `timestamp` before the value closes.
- `max_length` until the decoded length exceeds the bound.
- Quality rules and metrics, until the terminal `POST /egress/{contract_id}`.
- The end of the provider stream. If the stream ends inside a value, the attempt is malformed JSON, which is the streaming form of today's `extract_json` failure. Required-field checks run only for objects that actually closed. The SDK does not invent a closing `}` to force those checks.
- Disposition `tag`. Violations are collected and the stream runs to completion. The terminal egress result is the one the caller sees, including `tag`'s pass-through behavior from RFC-029.

A local failure on a **complete** object still performs the egress call. The server writes the same audit and quarantine row a non-streaming call would have written. The client does not skip that call because it already knows the answer.

A local failure on an **incomplete** object uses the partial route below instead of `POST /egress/{contract_id}`, because egress validates a JSON object and this buffer is not one.

### 4. Vercel AI SDK `wrapStream`

`AiSchemaGateMiddleware` in `sdks/aischemagate/typescript/src/ai.ts` gains `wrapStream` next to `wrapGenerate`. The helper stays free of a runtime `ai` dependency: the method is structural, as `wrapGenerate` is today.

`wrapStream` calls `doStream()`, reads text deltas and tool-call argument deltas, and returns a stream of the same part shape the AI SDK gave it. `wrapGenerate` stays the path for `generateText` and is unchanged, including its repair loop.

Yield rules inside `wrapStream`:

- Deltas for a field with no transform are forwarded as they arrive.
- Deltas that fall inside a field with `mask`, `hash`, `drop`, or `redact` are not forwarded. Those transforms apply to top-level strings only (`src/transform.rs`). After the terminal egress call, the middleware emits the sanitized field from the egress payload (or omits it, for `drop`) as a single replacement delta before the finish part.
- On fail fast, the middleware errors the stream with `AiSchemaGateRejectedError` (the same error `wrapGenerate` throws) carrying the violations, unless a repair is starting.
- Repair follows `SEMANTIC_VIOLATION_KINDS` and `maxRetries` exactly as `wrapGenerate`. The middleware cancels `doStream`, appends the repair hint with the existing `appendRepairMessage` path, and opens another stream. The caller sees a reset part before the next attempt's deltas so the UI drops the partial text. The attempt counter counts generations, not chunks. The hard cap remains five extra attempts.
- A type mismatch or malformed JSON does not repair. That matches `wrapGenerate`, which does not treat those as semantic violations.
- When the attempt's text parses as a complete object and local checks pass, `wrapStream` calls `POST /egress/{contract_id}` before it emits the finish part. A server-side failure (including a quality rule the client did not run) is a terminal error, or a repair, under the same rules. The egress payload replaces the assembled text the downstream `Output.object` parser will see, which is the same substitution `wrapGenerate` performs with `write`.

`streamText` users pass the same `aischemagateGuardrail({...})` object they pass for `generateText`. No second helper.

### 5. Python sync and async iterators

`LLMGate.completion` and `LLMGate.message` in `sdks/python/src/contractgate/llm.py` grow an explicit branch. `stream=True` returns an `LLMStream` instead of raising. `stream` false or absent returns `LLMGateResult` as today.

`AsyncLLMGate.completion` and `AsyncLLMGate.message` return an `AsyncLLMStream` when `stream=True`.

```python
class LLMStream:
    def __iter__(self) -> Iterator[StreamEvent]: ...
    # Set when the iterator finishes or raises.
    result: LLMGateResult | None

class AsyncLLMStream:
    def __aiter__(self) -> AsyncIterator[StreamEvent]: ...
    result: LLMGateResult | None

@dataclass
class StreamDelta:
    text: str

@dataclass
class StreamReset:
    violations: list[Violation]
    attempt: int  # 1-based count of the attempt that just failed
```

The sync iterator wraps the provider's sync iterator. The async iterator wraps the provider's async iterator. Both run the same parser and the same yield rules as `wrapStream`. OpenAI-protocol streams (`chat.completions.create(..., stream=True)`) and Anthropic-protocol streams (`messages.create(..., stream=True)`) both go through these wrappers. Chunk boundaries are the provider's chunks; the parser reassembles tokens.

`auto_repair=True` uses the Python policy: an egress failure or malformed JSON may re-prompt, within `max_retries` (default 2, hard cap 5). The iterator yields `StreamReset`, cancels the provider iterator, appends `repair_prompt(violations)` as a user message (the existing `_default_repair_prompt` when the caller did not pass one), and continues with the next attempt's `StreamDelta`s. The repair prompt is not inserted into the byte stream.

On a terminal failure with repair exhausted or `auto_repair` false, the iterator raises `LLMResponseError` with the violations attached, after cancelling the provider. `result` holds the last `LLMGateResult` when the attempt produced one (complete object, egress already called). An incomplete abort sets `result` from the partial-route response.

`validate_only` stays non-streaming. Passing an iterator to `extract_json` continues to raise, so a caller who wants streaming goes through `completion` / `message` with `stream=True`.

The TypeScript `LLMGate` in `sdks/typescript/src/llm.ts` follows the same contract with an async iterator only. This RFC does not add a second design for it.

### 6. Rust gate API

**Adopted shape: client-side closed-value checks, one server check per attempt.**

The tokens are already in the SDK process. Re-uploading each chunk to the gateway would put a network round trip on the path this RFC is trying to shorten. `pii_salt` stays on the server, so the client cannot apply `hash` or format-preserving `mask` and get the digest RFC-004 stores. Quarantine and audit are writes; they belong on one terminal request per attempt.

The authoritative check is the existing handler: `POST /egress/{contract_id}` in `src/egress.rs`, which runs `validate`, `apply_egress_pii_pipeline`, audit, and quarantine. No flag on that route changes meaning. `dry_run=true` still skips writes. `disposition` still means `block`, `fail`, or `tag`.

The one new route covers an attempt that fail-fasts before a complete object exists:

```
POST /egress/{contract_id}/partial
```

```json
{
  "closed": { "currency": "XYZ" },
  "violations": [
    { "field": "currency", "kind": "enum_violation", "message": "..." }
  ],
  "excerpt": "{ \"currency\": \"XYZ\", \"vendor\": \"A",
  "excerpt_spans": [{ "field": "vendor", "start": 28, "end": 31 }]
}
```

Server behavior:

1. Re-check each entry of `closed` with the same rules as `validate_value`. Drop any client violation the server does not reproduce. The server is authoritative.
2. Run `apply_transforms` on those closed fields. Transforms stay limited to declared top-level strings.
3. Store one quarantine row whose payload is the transformed closed object. Copy `excerpt` only after every `excerpt_spans` range that names a transformed field has been replaced with the transformed text. If a span does not line up with the excerpt, omit the excerpt.
4. Do not evaluate required fields for objects that were not closed, and do not run freshness.
5. `dry_run=true` returns the server violations and writes nothing.
6. The row uses `direction = 'egress'`, the same audit direction as a normal egress failure.

A complete object does not use this route. It uses `POST /egress/{contract_id}`.

**Session endpoint, not adopted.** A streaming session (`POST /egress/{contract_id}/stream`, chunked request, SSE of per-field verdicts) would move the parser into the gateway. That helps a host that cannot run the SDK. It also holds gateway memory for the life of a generation, and it either skips PII until the end (the client design, with extra hops) or sanitizes per field (a round trip per closed field). This RFC does not add that route. Hosts without the SDK keep today's option: assemble the object, then call egress. The session route is recorded under [Alternatives](#alternatives-considered).

### 7. Quarantine, repair, and PII

**Quarantine.** No chunk writes a row. A complete attempt writes through the existing egress path, including intermediate repair attempts that produced a complete object — that is what a non-streaming repair loop already does, because each attempt calls `client.egress`. An incomplete fail-fast writes one row through `POST /egress/{contract_id}/partial`. A retried client uses an `Idempotency-Key` on that partial call; the server stores one row per key. `dry_run=true` writes neither kind of row.

**Repair prompts.** The prompt is built with the surface's existing function (`repair_prompt` in Python, `repairHint` in `sdks/aischemagate/typescript/src/ai.ts`) and sent as a user message on the next provider request. It is not streamed into the failed attempt. The caller observes `StreamReset` (Python) or a reset part (`wrapStream`) and drops text from the failed attempt before painting the next one. Repair does not start until the failed provider stream is cancelled, so two provider streams are not open for one gate. The cap is unchanged: default two extra generations, hard cap five, counting attempts rather than chunks.

**PII.** `apply_transforms` runs on a finished string. A prefix of an email is a different HMAC input than the full email, and a partial format-preserving mask is a different string than the mask of the finished value. The SDK therefore withholds a transformed field's deltas for the whole attempt. The yielded value is the one egress (or the partial route) returns after `apply_transforms`. `drop` yields nothing for that field. The client never computes `hash`.

Undeclared root fields under leakage `strip` or `fail` are withheld from the moment the key closes. `fail` also fail-fasts when disposition is `block` or `fail`.

The stored quarantine payload follows RFC-004: transformed fields, and under `compliance_mode` the undeclared keys removed, as `apply_transforms` already does before `audit_log` and `quarantine_events`. The partial route uses that same function on `closed`.

Provider retention of the raw generation is outside the gate. The gate's guarantee is the one RFC-004 and RFC-030 already state for stored and forwarded payloads.

### 8. Latency goals

These are goals for the implementation to hit once a streaming path exists. They are goals only.

- Goal: closed-value checks run in the SDK process, with no network round trip on the per-chunk path, so a definitive violation is visible to the caller before the provider finishes the rest of the object.
- Goal: a generation attempt sends one gateway request (the existing egress call, or one partial call), plus one further request per repair attempt, and not one request per chunk.
- Goal: the authoritative `validate()` on a complete object stays inside the gateway budget the contributor guide already sets for the engine, p99 well under 15 ms. That budget is a goal for this work. Chunk handling has its own goals above, and no figure is attached to it here.
- Goal: withholding a transformed field does not wait on a per-field sanitize RPC. The sanitized bytes arrive with the single terminal response.

## Rollout

Status stays **Draft** until both of the following are true:

1. A UI mockup, reviewed by the owner, shows all five moments below.
2. The owner approves this RFC. The status line then changes to Accepted, in a docs follow-up or at the top of the implementation PR.

The mockup frames:

1. Deltas for a field with no transform appearing while the object is still open.
2. The stream stopping on a closed-value failure (enum or range), with the field, kind, and message visible.
3. Repair: the partial text is discarded, then a later attempt's deltas appear.
4. A field declared `redact` or `hash` stays hidden, then shows the sanitized egress value.
5. One quarantine row for the failed attempt, including an incomplete abort, and no row per chunk.

Until that approval, no engine, SDK, or dashboard change ships from this design.

After approval, the implementation order is:

1. Shared JSON fixtures of partial inputs and the violation kind expected at each prefix. The Rust tests and the SDK tests consume the same files.
2. Python iterators, then the TypeScript `LLMGate` async iterator, then `wrapStream`.
3. `POST /egress/{contract_id}/partial` and its quarantine write.
4. A reference page, `docs/streaming-validation-reference.md`, linked from `docs/llm-output-validation.md` and `docs/integrations/vercel-ai-sdk.md`, in the same PR as the user-facing API. That page is user-facing surface (new method behavior and a new route) and waits for the implementation PR, per the contributor guide.

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

**Python.** Sync iterator and async iterator. `stream=True` yields `StreamDelta` before the provider finishes. Fail-fast cancels the underlying iterator (a test double that records `close` / `aclose`). `auto_repair` yields `StreamReset`, starts a second provider call, and stops at the hard cap. `stream` omitted still returns `LLMGateResult`. `extract_json` on an iterator still raises.

**`wrapStream`.** Forwards a non-transformed text delta. Withholds a `redact` field until the egress payload is spliced in. Semantic failure with `autoRepair` resets and re-calls `doStream` within the cap. Type mismatch does not re-prompt. `wrapGenerate` tests in `guardrail.test.ts` stay green without a stream.

**Quarantine and PII.** A complete failure produces one egress quarantine write and zero partial writes. An incomplete enum failure produces one partial write and zero egress event writes. Two chunks produce the same single write. `dry_run=true` produces none. The stored payload for a `hash` field is the transformed form; the yielded text never contains the raw field. An idempotent retry of the partial call stores one row.

**Disposition `tag`.** A closed enum failure does not cancel the provider. The terminal result carries the violations.

**Latency.** A later benchmark may be read against the goals in [Latency goals](#8-latency-goals). The reference page is where that run is recorded, if the implementation adds one.

## Alternatives considered

**Streaming session on the gateway.** `POST /egress/{contract_id}/stream` would accept text chunks and emit per-field verdicts. The parser would live in one place, which removes the risk of the SDK port drifting from `validate_value`. The cost is a network hop per chunk on the path whose goal is to decide before the provider finishes, plus session state held for the whole generation. PII would still wait for a finished string, so the session would not sanitize any earlier than the client design. Deferred to a later RFC if a host that cannot run the SDK needs fail-fast. The partial route in this RFC is the only new HTTP surface, and it runs once per incomplete attempt.

**Silent full buffer, then today's egress.** `stream=True` would collect every chunk and return `LLMGateResult`. RFC-094 rejected that as the v1 default because callers pass `stream=True` in order to render partial text. This RFC keeps incremental yield, with internal retention only so the terminal request has an object to send.

**Client checks only, no terminal egress.** The SDK would be the authority. That skips quarantine, audit, and transforms that need `pii_salt`, and it lets the port drift from `validate` with no server result to compare. Rejected. A local pass still calls egress. A local failure on a complete object still calls egress.

**Server check only, no incremental parser.** `stream=True` would be allowed, the SDK would forward every raw delta, and egress would run at the end. That is the buffer workaround this RFC exists to replace: a bad enum stays on screen until the model stops, and transformed fields are painted raw. Rejected as the default. It remains the behavior of a caller who assembles the object themselves and calls egress, which is unchanged.

**Regex-prefix rejection.** A pattern that cannot match any continuation of the open string could fail before the closing quote. It is easy to reject a prefix that a later character would have saved, and the engine does not do this on complete values. Out of scope. Patterns run when the string closes.

**Per-field sanitize RPC.** A closed transformed field would be sent to the gateway immediately so the UI can paint the mask before the object finishes. That spends a round trip per sensitive field and complicates attempt accounting. The yield rule withholds the field until the single terminal payload instead.

## Open questions

1. **Text and tool-call buffers together.** `extract_json` prefers an OpenAI content string, then tool-call arguments, then an Anthropic tool input. If a stream emits both text deltas and argument deltas, which buffer is the contract object while both are still growing? Proposal to confirm at approval: follow the finished-response precedence, and parse only the preferred buffer once it has started receiving deltas. The other buffer is ignored for validation.
2. **Repair transport for `wrapStream`.** The design re-enters `doStream` so the retry stays streaming. Falling back to `doGenerate` for the retry is simpler and drops incremental checks on the attempt most likely to be the one the user sees. Owner confirmation belongs on the mockup (frame 3).
3. **Contract cache lifetime.** The local checker caches the contract document for the gate instance. A deploy of a new version during a long-lived process would leave the client on the old ontology until restart, while egress would use the new one and could fail a stream the client passed. Proposal: cache key `(contract_id, version)` with a short TTL when `version` is omitted, and no TTL when `version` is pinned. The number for that TTL is unset.
4. **Idempotency window for `POST /egress/{contract_id}/partial`.** The design requires `Idempotency-Key` and stores one row per key. How long the server remembers the key, and what a reused key with a different body returns, is unset. It should match whatever idempotency egress already has, or be explicit if egress has none.
5. **Python event type.** `StreamEvent` (`StreamDelta` | `StreamReset`) is the shape in this RFC. An alternative is to yield provider text and expose `reset` as a callback. The event type is what the mockup's repair frame binds to; if the mockup needs a different hook, this question is the one to change.
