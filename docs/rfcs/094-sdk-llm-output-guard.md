# RFC-094 — `LLMGate` SDK Class + `aischemagate` Alias Package

**Status:** Draft
**Date:** 2026-10-11
**Depends on:** RFC-029 (egress validation), RFC-030 (egress PII and leakage), RFC-093 (egress as first-class door), RFC-004 (PII masking at ingest), RFC-005 (Python SDK), RFC-018 (TypeScript SDK), RFC-089 (LLM integration playbook), RFC-090 (MCP server)
**Companion RFCs:** RFC-095 (AISchemaGate docs portal), RFC-096 (AISchemaGate frontend repo + CORS)

---

## Problem

The engine already enforces semantic contracts on outbound JSON payloads via
`POST /egress/{contract_id}` (RFC-029 / RFC-030 / RFC-093). The Python and TS
SDKs already expose `client.egress()`. The MCP server already exposes
`egress_validate`. RFC-004 transforms already run on whatever `egress` sees.

A dev brief dated 2026-10-11 asks us to launch a sister product **AISchemaGate**
with its own frontend (`aischemagate.com`), docs portal (`aischemagate.dev`),
and an SDK import of `from aischemagate import LLMGate`. The backend, tenant
service, API keys, and Rust engine are shared with DataContractGate. The brief
proposes this developer experience:

```python
from aischemagate import LLMGate
from openai import OpenAI

client = OpenAI()
gate = LLMGate(api_key="cg_live_...", contract_id="extract_invoice_v1")
response = gate.completion(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Extract invoice data..."}],
    auto_repair=True,
)
```

What is missing:

1. An ergonomic class-level wrapper (`LLMGate`) that bundles response-shape
   extraction, egress validation, and the auto-repair loop into a single
   object a developer can import and use in one line.
2. A new PyPI / npm package named `aischemagate` that re-exports the shared
   `contractgate` client, contract types, violation types, and the new
   `LLMGate` class, so the brief's `from aischemagate import LLMGate` works
   without copy-pasting code between packages.
3. A clear rule for whether `gate.completion(...)` **calls the LLM provider
   itself** or **wraps a response the caller already produced**. The brief's
   example is ambiguous: `client = OpenAI()` is constructed but not passed
   into `gate`. Both interpretations have very different implementation
   footprints. See the top open question (Q1).

RFC-094 defines the `LLMGate` surface, the alias package, and the shared-core
semantics. RFC-095 covers the docs portal. RFC-096 covers the frontend repo
and CORS/DNS. No engine code changes in any of them.

## Goal

Ship a thin, class-based `LLMGate` wrapper plus an `aischemagate` alias
package so `from aischemagate import LLMGate` works in Python and
`import { LLMGate } from 'aischemagate'` works in TypeScript, both of them
routing all enforcement through the shipped `/egress/{contract_id}` endpoint
with no engine change.

**Done means:**

1. `pip install aischemagate` installs a package that re-exports `Client`,
   `AsyncClient`, `Violation`, `ValidationResult`, `EgressResponse`, and the
   new `LLMGate` class from the shared `contractgate` core. `npm install
   aischemagate` does the same for TypeScript.
2. `LLMGate(api_key=..., contract_id=...)` is the one-line constructor. The
   class is additive and introduces no new state on the existing `Client`.
3. `gate.completion(model=..., messages=..., auto_repair=True)` returns a
   validated response with `payload`, `violations`, `ok`, `attempts`, and
   `raw_response`. See §Design for the shape.
4. Response extraction supports OpenAI chat (JSON mode), OpenAI tool_calls,
   Anthropic tool_use blocks, raw strings with markdown-fence tolerance, and
   dict passthrough. vLLM and OpenAI-compatible Ollama endpoints work because
   their response shape **is** the OpenAI shape; no extra adapter is needed.
5. Auto-repair is a bounded re-prompt loop, default `max_retries=2`, hard cap
   5. Deterministic. Caller can supply a custom `repair_prompt` formatter.
6. The alias package has no behavioral surface beyond the re-exports and
   `LLMGate`. Breaking changes are made in `contractgate`, never in
   `aischemagate` directly — the alias package pins to a floor version of
   `contractgate` and lets semver carry upgrades.
7. Unit tests cover each extractor shape, happy-path validation, failure-path
   validation, repair pass-on-first-try, repair pass-on-second-try, repair
   exhaust-budget, and the `aischemagate` import surface.
8. RFC-094 is cited by RFC-095's docs portal and by RFC-096's frontend repo.

## Non-goals

- **A new validation engine or endpoint.** All validation goes through
  `/egress/{contract_id}`. RFC-004 transforms run server-side as today. No
  new rule type.
- **A new tenant service.** The brief constrains both domains to share the
  same tenant / API-key issuance. The `api_key` passed to `LLMGate` is the
  same `cg_live_...` key the dashboard already issues (RFC-056). No new
  issuance path.
- **Streaming output validation.** The engine validates complete JSON
  objects. SSE / token-stream validation needs an incremental parser and
  partial-violation semantics. v1 `LLMGate` rejects streaming with a clear
  error — do not silently buffer. Follow-up RFC.
- **"Eliminate hallucinated values" in the marketing sense.** The contract
  catches violations the contract can express (type, range, enum, pattern,
  undeclared field under compliance mode, RFC-004 transforms). Semantic
  truth outside the schema is still a boundary problem (RFC-093 §Non-goals).
  RFC-095 Appendix A forbids the broader marketing claim.
- **Prompt-firewall engine.** Prompt-side PII masking stays a one-line
  convenience over `client.ingest(..., dry_run=True)` + RFC-004 transforms.
  See §Design 5.
- **Vendoring provider SDKs.** We do not fork or copy OpenAI / Anthropic
  SDK code. Whether the caller or `LLMGate` imports them is Q1.
- **Breaking changes to `contractgate`.** `aischemagate` is purely additive.
  Existing `contractgate` imports keep working byte-for-byte.
- **A full rename to AISchemaGate.** PyPI `contractgate` and npm
  `@nightmoose/contractgate-sdk` keep their existing names and continue to
  publish. `aischemagate` is a sister alias package, not a rename.

## Design

### 1. Shape of `LLMGate` (Python)

```python
# sdks/python/src/contractgate/llm.py  (the actual implementation lives in
# contractgate; aischemagate re-exports it)

from contractgate import Client, AsyncClient, Violation

class LLMGate:
    def __init__(
        self,
        *,
        api_key: str,
        contract_id: str,
        base_url: str = "https://app.datacontractgate.com",
        llm: Any,                               # required — caller binds OpenAI/Anthropic/Ollama/vLLM client
        version: Optional[str] = None,
        disposition: Optional[str] = None,      # None → gateway default (block)
        max_retries: int = 2,                   # hard cap 5
        repair_prompt: Optional[Callable[[list[Violation]], str]] = None,
        client: Optional[Client] = None,        # inject for tests
    ) -> None: ...

    def completion(
        self,
        *,
        model: str,
        messages: list[dict[str, str]],
        auto_repair: bool = False,
        response_format: Optional[dict] = None, # defaults to {"type": "json_object"}
        tools: Optional[list[dict]] = None,
        tool_choice: Optional[Any] = None,
        **provider_kwargs: Any,                 # forwarded unchanged
    ) -> "LLMGateResult": ...

    def validate_only(
        self,
        response: Any,
        *,
        dry_run: bool = False,
    ) -> "LLMGateResult": ...                   # for callers that already have a response

@dataclass
class LLMGateResult:
    ok: bool
    payload: dict                               # egress-sanitized; forward this
    violations: list[Violation]
    attempts: int                               # 1 for happy path, up to max_retries + 1
    raw_response: Any                           # the final provider response object
    repair_hints: list[str]                     # the hints we fed back, empty on first-try pass
```

Design notes:

- **No new state on `Client`.** `LLMGate` composes a `Client` (either
  provided or constructed from `api_key` + `base_url`). The existing
  `contractgate.Client` surface is unchanged.
- **`validate_only`** is the escape hatch for callers who do not want
  `LLMGate` to call the provider at all (RFC-089 ingest users; MCP agents).
  It takes a response the caller already has, extracts the JSON, calls
  egress, returns the same `LLMGateResult`.
- **Async mirror**: `AsyncLLMGate` with `async def completion` /
  `async def validate_only`. Same arguments.
- **`provider_kwargs` passthrough** keeps us out of the way of
  provider-specific knobs we do not care about (seed, temperature, top_p,
  user_id, metadata, …).
- **`disposition` default is None → gateway default.** RFC-093 shipped
  `block` as the gateway default. `LLMGate` does not duplicate that
  default; it defers to the server so a server-side change propagates.

### 2. Response extraction (shared with RFC-089 playbook)

`LLMGate._extract(response)` handles:

- `str` → `json.loads` with markdown-fence tolerance.
- OpenAI chat `response.choices[0].message.content` → `json.loads`.
- OpenAI chat `response.choices[0].message.tool_calls[0].function.arguments`
  → `json.loads`.
- OpenAI Responses API `response.output_parsed` → passthrough if dict.
- Anthropic Messages `response.content[*].type == 'tool_use'` → `.input`.
- `dict` → passthrough.

Multiple tool_calls / tool_use blocks → take the first and document it.
Streaming iterables → raise `LLMResponseError("streaming responses are not
supported in v1")`. Unrecognized shape → raise with the detected top-level
type.

vLLM speaks the OpenAI protocol; its responses deserialize into the OpenAI
chat shape. Ollama has an OpenAI-compat endpoint that does the same.
Neither needs a dedicated extractor path.

### 3. Auto-repair loop

Caller-side, pure. Pseudocode:

```python
def completion(self, *, model, messages, auto_repair=False, **kw):
    for attempt in range(1, self.max_retries + 2):  # 1..=max_retries+1
        resp = self._call_provider(model=model, messages=messages, **kw)
        try:
            payload = self._extract(resp)
        except LLMResponseError:
            if not auto_repair or attempt > self.max_retries:
                raise
            hint = "Return a JSON object only. Previous response was not parseable."
            messages = self._append_repair(messages, hint)
            continue

        egress = self._client.egress(contract_id=self.contract_id, events=[payload])
        if egress.failed == 0 or not auto_repair or attempt > self.max_retries:
            return LLMGateResult(
                ok=(egress.failed == 0),
                payload=egress.payload,          # sanitized
                violations=egress.outcomes[0].violations,
                attempts=attempt,
                raw_response=resp,
                repair_hints=self._hints_so_far,
            )

        hint = (self.repair_prompt or _default_repair_prompt)(egress.outcomes[0].violations)
        messages = self._append_repair(messages, hint)
        self._hints_so_far.append(hint)
    # unreachable; the loop above returns on the last iteration
```

Key rules (same as RFC-089 playbook will state):

- `max_retries` is clamped to `[0, 5]`. Values above 5 warn and clamp to 5.
  Default 2 → up to 3 provider calls.
- The repair hint is **appended as a new user message**, not inserted into
  the system message. The system message is the schema instruction.
- On exhaust, the result has `ok=False` and the final sanitized payload.
  Caller must not forward `raw_response`'s content after a `block` or
  `fail` disposition — RFC-093's rule still applies.
- No backoff; the provider SDK handles rate limits.

### 4. The `aischemagate` alias package

Two artifacts, both newly published, both thin:

**PyPI `aischemagate`**:

```
# sdks/aischemagate/python/
# pyproject.toml:
#   name = "aischemagate"
#   dependencies = ["contractgate >= X.Y.Z"]
# aischemagate/__init__.py:

from contractgate import (
    Client, AsyncClient, Contract, CompiledContract,
    Violation, ValidationResult, ViolationKind,
    EgressResponse, BatchIngestResponse,
    # … all current contractgate exports
)
from contractgate.llm import LLMGate, AsyncLLMGate, LLMGateResult, LLMResponseError

__all__ = [
    "Client", "AsyncClient", "Contract", "CompiledContract",
    "Violation", "ValidationResult", "ViolationKind",
    "EgressResponse", "BatchIngestResponse",
    "LLMGate", "AsyncLLMGate", "LLMGateResult", "LLMResponseError",
]
```

**npm `aischemagate`** (or `@nightmoose/aischemagate` if we scope it;
confirm in Q4):

```ts
// sdks/aischemagate/typescript/src/index.ts
export * from '@nightmoose/contractgate-sdk';
export { LLMGate, AsyncLLMGate, LLMResponseError } from '@nightmoose/contractgate-sdk/llm';
export type { LLMGateResult } from '@nightmoose/contractgate-sdk/llm';
```

Rules:

- `aischemagate` **has no behavior of its own**. It pins a minimum
  `contractgate` version and re-exports. Breaking changes are made in
  `contractgate`, and `aischemagate` releases follow.
- Version numbers are independent of `contractgate` so marketing can bump
  `aischemagate` to 1.0.0 at launch while `contractgate` stays on its own
  track.
- Install docs say `pip install aischemagate` / `npm install aischemagate`.
  Install docs do **not** say to install both; `aischemagate` pulls
  `contractgate` as a transitive dep.
- `LLMGate` lives in `contractgate.llm`. The brief's example (`from
  aischemagate import LLMGate`) works because of the re-export. Advanced
  users who prefer the DataContractGate brand can still import from
  `contractgate.llm` directly; both paths resolve to the same class.

### 5. Prompt-side PII (documented pattern, no new engine)

`LLMGate.mask_prompt(payload, *, prompt_contract_id)` returns the
transformed payload (`client.ingest(dry_run=True)` under the hood). The
caller formats their prompt string from the transformed dict and sends
that to the LLM.

Design intent: zero new engine code. Prompts are inbound to the LLM, which
means the natural direction is `ingress` with RFC-004 transforms. We just
expose it as a named method on `LLMGate` for ergonomics.

### 6. Streaming (v1 non-goal, explicit error)

`LLMGate.completion(..., stream=True)` raises
`LLMResponseError("streaming is not supported in v1 — pass stream=False or
use validate_only(response) with a non-streaming response")`. This is a
deliberate hard fail. Silent buffering of streamed tokens is the wrong
default (users want to render partial tokens for UX; buffering breaks
that invariant).

### 7. MCP interaction

Unchanged. The existing `egress_validate` MCP tool (RFC-093) is the
agent-side entry point. The RFC-095 playbook tells agents to use the MCP
tool rather than `LLMGate` because agents in MCP hosts already have the
JSON object on hand after their LLM step; no extraction needed.

### 8. Changes to shipped `contractgate` surface

Additive only:

- `contractgate.llm` submodule: `LLMGate`, `AsyncLLMGate`, `LLMGateResult`,
  `LLMResponseError`, `mask_prompt`, `extract_json` (public for callers who
  want the raw extractor).
- `contractgate.__init__` **does not** re-export `LLMGate` at the top level
  — callers import from `contractgate.llm` or from `aischemagate`.
- `@nightmoose/contractgate-sdk` TypeScript gets a `./llm` subpath export.

No change to `Client`, `AsyncClient`, `Contract`, or any shipped method.

## Implementation checklist

1. `sdks/python/src/contractgate/llm.py`: `LLMGate`, `AsyncLLMGate`,
   `LLMGateResult`, `LLMResponseError`, `mask_prompt`, `extract_json`.
2. `sdks/python/tests/test_llm_gate.py`: extractor matrix (OpenAI JSON mode,
   OpenAI tool_calls, Anthropic tool_use, str w/ and w/o fence, dict
   passthrough, unknown shape → raise, streaming → raise), `validate_only`
   happy path and failure, `completion` happy path, `completion` auto-repair
   three-arm test (first-try pass, second-try pass, exhaust budget),
   `max_retries > 5 → warn + clamp`, `mask_prompt` round-trip.
3. `sdks/typescript/src/llm.ts`: mirror the Python surface
   (`LLMGate`, `AsyncLLMGate`, `LLMGateResult`, `LLMResponseError`,
   `maskPrompt`, `extractJson`).
4. `sdks/typescript/tests/llm.test.ts`: mirror the Python tests.
5. `sdks/typescript/package.json`: add `"./llm": "./dist/llm.js"` under
   `exports`.
6. `sdks/aischemagate/python/`: new PyPI package. `pyproject.toml`,
   `aischemagate/__init__.py` with re-exports, `tests/test_imports.py`
   asserting every documented symbol is importable from `aischemagate`.
7. `sdks/aischemagate/typescript/`: new npm package. `package.json`,
   `src/index.ts` with re-exports, `tests/imports.test.ts`.
8. Decide the npm scope — unscoped `aischemagate` vs. `@nightmoose/aischemagate`.
   See Q4.
9. CHANGELOG entries in `contractgate` for the new submodule and in
   `aischemagate` for v0.1.0 initial release.
10. `docs/llm-sdk-reference.md`: public API reference for `LLMGate`
    (Python + TS side-by-side).
11. No Rust / engine / dashboard / migration changes.

## Test plan

### Unit (`contractgate` SDKs)
1. `LLMGate._extract` on each supported shape — six cases.
2. `LLMGate.validate_only` with a passing payload → `ok=True`, `payload`
   equals egress-sanitized form.
3. `LLMGate.validate_only` with a failing payload → `ok=False`, violations
   populated, `payload` is the sanitized form.
4. `LLMGate.completion(auto_repair=False)` happy path → one provider call,
   one egress call.
5. `LLMGate.completion(auto_repair=True)` with ok-on-first → one provider
   call, one egress call, `attempts=1`.
6. `LLMGate.completion(auto_repair=True)` with ok-on-second → two provider
   calls (second message list includes the repair hint), two egress calls,
   `attempts=2`.
7. `LLMGate.completion(auto_repair=True)` exhaust budget → `max_retries+1`
   provider calls, final `ok=False`, violations populated.
8. `LLMGate(max_retries=99)` → warning emitted, clamped to 5, at most 6
   provider calls on exhaust.
9. `LLMGate.completion(stream=True)` → `LLMResponseError`.
10. `LLMGate.mask_prompt` → asserts the ingest body shape and `dry_run=true`
    query param; returns the transformed payload the server returned.

### Unit (`aischemagate` alias)
11. `from aischemagate import LLMGate` works; the imported class is the
    same object as `contractgate.llm.LLMGate` (`is` check).
12. Every symbol in `aischemagate.__all__` resolves and matches the
    `contractgate` export of the same name.
13. TypeScript equivalent of (11) and (12) via `imports.test.ts`.

### Integration (gateway + SDK)
14. Live `LLMGate.validate_only` against a running gateway with an RFC-004
    `hash` transform → returned `payload.email` is a `hmac-sha256:...` hex
    string.
15. Live `LLMGate.completion(auto_repair=True)` with a mock LLM that fails
    once then passes → asserts one audit row with `direction='egress'`.

### Conformance
16. Both SDKs run the same end-to-end scenario against the same gateway;
    outputs match.

## Rollout

1. Accept RFC-094, RFC-095, RFC-096 as a set (they ship as a product
   launch, not as isolated features).
2. Implement `contractgate.llm` module + tests (Python).
3. Mirror in TypeScript.
4. Build `aischemagate` Python package + import test. Publish to TestPyPI
   first, verify `pip install -i testpypi aischemagate` resolves.
5. Build `aischemagate` npm package + import test. Publish `--dry-run`
   first.
6. Doc reference `docs/llm-sdk-reference.md`.
7. RFC-095 docs portal consumes this reference.
8. Release notes: `contractgate` minor bump (new submodule),
   `aischemagate` 0.1.0 initial release.
9. MAINTENANCE_LOG + STATUS.md updates.

## Decisions (signed off 2026-10-11)

- **Q1 → B (caller binds client).** `LLMGate.__init__` takes `llm` as a
  required parameter. Zero provider SDK deps on `aischemagate` /
  `contractgate`. vLLM and Ollama work for free because they expose the
  OpenAI protocol. `gate.completion(model, messages, **kw)` forwards to
  `llm.chat.completions.create`; `gate.message(model, messages, max_tokens, **kw)`
  forwards to `llm.messages.create` for Anthropic. The brief's example
  gets one extra kwarg (`llm=client`).
- **Q2 → `max_retries = 2`** (3 total provider calls default). Hard cap at 5.
- **Q3 → Ship sync + async in v1.** Both `LLMGate` and `AsyncLLMGate` land
  in v0.1.0.
- **Q4 → Scoped npm, unscoped PyPI.** `@nightmoose/aischemagate` on npm,
  `aischemagate` on PyPI.
- **Q5 → Streaming deferred to v2.** `stream=True` raises `LLMResponseError`.
- **Q6 → `mask_prompt` on the class.** Confirmed; no module-level helper
  in v1.

Original open-question text retained below as a record.

## Open questions (resolved — see Decisions above)

- **Q1 — Does `LLMGate.completion()` call the LLM provider itself, or does
  the caller bind a provider client?** The brief's example constructs
  `client = OpenAI()` and does not pass it to `LLMGate`, which implies
  `LLMGate` calls OpenAI itself. Two interpretations:

  **A. `LLMGate` calls providers directly** (brief as written). Pros:
  truest one-line adoption; developer never writes a provider call.
  Cons: we add provider SDKs as optional extras (`aischemagate[openai]`,
  `aischemagate[anthropic]`); we own model-name → provider dispatch
  (`gpt-*` → OpenAI, `claude-*` → Anthropic, custom endpoint for Ollama /
  vLLM); we pin a floor version of each SDK and track their breaking
  changes forever; streaming / tool-use / response_format shape differences
  across providers become our problem.

  **B. Caller binds a provider client** (draft recommendation). The
  constructor takes `llm=openai_client` (or `llm=anthropic_client`), and
  `gate.completion(...)` forwards `model` / `messages` / `**kwargs` to
  `llm.chat.completions.create` or `llm.messages.create`. Pros: no provider
  SDK deps; no dispatch; no streaming / tool-use shape maintenance; works
  identically for vLLM and Ollama because they expose the OpenAI protocol.
  Cons: brief's example as written is slightly different — the user must
  pass the client to the constructor.

  **Draft default: B** with a note that the brief's example gets a minor
  edit (one extra kwarg). Confirm or switch to A.

- **Q2 — `max_retries` default** = 2 (3 total provider calls). Confirm.

- **Q3 — Async mirror** `AsyncLLMGate`: ship in v1 or sync-only in v1?
  Draft recommends ship in v1 since the shipped `AsyncClient` is on the
  same bar.

- **Q4 — npm package name**: `aischemagate` (unscoped) or
  `@nightmoose/aischemagate` (scoped)? PyPI `aischemagate` is likely
  available; npm unscoped names are harder to acquire. The brief's example
  (`from aischemagate import LLMGate`) is Python-only; the TS import can
  be `from '@nightmoose/aischemagate'` without breaking the mental model.
  Draft recommends scoped npm; unscoped PyPI.

- **Q5 — Streaming**: v1 rejects (non-goal). Confirm defer.

- **Q6 — `mask_prompt` on `LLMGate` vs. module-level helper**: draft puts
  it on the class so callers who already have `gate` do not need a second
  import. Confirm.

Close Q1–Q6 before implementation, per RFC-004 / RFC-093 convention.
