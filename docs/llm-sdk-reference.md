# LLM SDK Reference — `LLMGate`

`LLMGate` is a thin wrapper over the shipped `POST /egress/{contract_id}`
endpoint for validating LLM structured output. It extracts the JSON object
from the common provider response shapes, calls egress against your
deployed contract, and optionally runs a bounded re-prompt loop until the
output passes or the retry budget exhausts.

`contract_id` (and `prompt_contract_id`) take either the contract's UUID or
its YAML `name`. A name is looked up once on the API key's contract list
(`GET /contracts`) and cached for the life of the gate; an unknown name
raises an error listing the names on the key.

**Design**: see [RFC-094](rfcs/094-sdk-llm-output-guard.md).
**Egress semantics** (dispositions, PII transforms, audit writes): see
[egress-validation-reference.md](egress-validation-reference.md).

---

## Install

| Language | Core SDK | AISchemaGate alias |
|---|---|---|
| Python | `pip install "contractgate>=0.3.0"` | `pip install aischemagate` |
| TypeScript / Node ≥ 20 | `npm install @nightmoose/contractgate-sdk@^0.3.0` | `npm install @nightmoose/aischemagate` |

The alias package re-exports the same classes under the `aischemagate` /
`@nightmoose/aischemagate` import name. Pick whichever fits your mental
model; the behavior is identical.

```python
# Both of these import the same object.
from contractgate.llm import LLMGate
from aischemagate       import LLMGate
```

```ts
// Both of these import the same object.
import { LLMGate } from '@nightmoose/contractgate-sdk/llm';
import { LLMGate } from '@nightmoose/aischemagate';
```

## Constructor

The caller **brings their own LLM client**. The SDK takes no runtime
dependency on `openai` / `anthropic` / any provider. vLLM and
OpenAI-compatible Ollama endpoints work with the OpenAI-shaped
`.completion()` call unchanged.

### Python

```python
from contractgate.llm import LLMGate
from openai import OpenAI

gate = LLMGate(
    api_key="cg_live_...",            # required — same key as the dashboard
    contract_id="extract_invoice_v1", # required — contract name or UUID
    llm=OpenAI(),                     # required — caller-bound client
    base_url="https://api.datacontractgate.com",  # default
    version=None,                     # pin to a specific contract version
    disposition=None,                 # None → gateway default (block)
    max_retries=2,                    # default 2; hard cap 5 (warns + clamps)
    repair_prompt=None,               # callable(violations) -> str; default below
    client=None,                      # inject an existing contractgate.Client
)
```

`AsyncLLMGate` has the identical constructor, except `llm` is an
async-aware client (`openai.AsyncOpenAI`, `anthropic.AsyncAnthropic`, …)
and `client` takes an `AsyncClient`.

### TypeScript

```ts
import { LLMGate } from '@nightmoose/contractgate-sdk/llm';
import OpenAI from 'openai';

const gate = new LLMGate({
  apiKey: 'cg_live_...',           // required
  contractId: 'extract_invoice_v1',// required — contract name or UUID
  llm: new OpenAI(),               // required
  baseUrl: 'https://api.datacontractgate.com',  // default
  version: undefined,
  disposition: undefined,          // undefined → gateway default (block)
  maxRetries: 2,                   // hard cap 5
  repairPrompt: undefined,         // (violations) => string
  client: undefined,               // inject an existing Client
});
```

All TS methods are `async`. There is only one TS class (no sync/async
split) because the TS `Client` is itself async-only.

## Methods

### `completion` — OpenAI protocol

Calls `llm.chat.completions.create({...})` under the hood. Works for
OpenAI, vLLM, and Ollama OpenAI-compat endpoints.

```python
result = gate.completion(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Extract the invoice JSON..."}],
    auto_repair=True,
    response_format={"type": "json_object"},   # forwarded as-is
    temperature=0.1,                           # forwarded as-is
)
```

```ts
const result = await gate.completion({
  model: 'gpt-4o-mini',
  messages: [{ role: 'user', content: 'Extract the invoice JSON...' }],
  autoRepair: true,
  response_format: { type: 'json_object' },
  temperature: 0.1,
});
```

### `message` — Anthropic protocol

Calls `llm.messages.create({...})` under the hood. Caller supplies
`max_tokens` (default 1024).

```python
result = gate.message(
    model="claude-sonnet-4-5",
    messages=[{"role": "user", "content": "Extract the invoice JSON..."}],
    max_tokens=1024,
    auto_repair=True,
    tools=[{...}],                 # forwarded as-is
)
```

```ts
const result = await gate.message({
  model: 'claude-sonnet-4-5',
  messages: [{ role: 'user', content: 'Extract the invoice JSON...' }],
  maxTokens: 1024,
  autoRepair: true,
  tools: [{...}],
});
```

### `validate_only` / `validateOnly` — bring your own response

Escape hatch for callers who already have the provider response (e.g. a
LangChain / CrewAI chain output, or an MCP agent tool-call result).
Extracts the JSON and runs egress; no provider call.

```python
chain_output = await chain.ainvoke({"input": "..."})
result = gate.validate_only(chain_output, dry_run=False)
```

```ts
const chainOutput = await chain.invoke({ input: '...' });
const result = await gate.validateOnly(chainOutput, { dryRun: false });
```

### `mask_prompt` / `maskPrompt` — RFC-004 prompt-side masking

Returns the RFC-004-transformed form of a prompt dict. One-line
convenience over `client.ingest(dry_run=True)`. Format your prompt
template from the returned dict; never send the raw dict to the LLM.

```python
clean = gate.mask_prompt(
    {"email": "alice@example.com", "ssn": "123-45-6789", "message": "..."},
    prompt_contract_id="my_prompt_shape_v1",
)
# clean == {"email": "hmac-sha256:...", "ssn": "<REDACTED>", "message": "..."}
# depending on the prompt contract's declared transforms (RFC-004).
```

```ts
const clean = await gate.maskPrompt(
  { email: 'alice@example.com', ssn: '123-45-6789', message: '...' },
  { promptContractId: 'my_prompt_shape_v1' },
);
```

## Return shape — `LLMGateResult`

| Field | Python | TypeScript | Meaning |
|---|---|---|---|
| success flag | `ok: bool` | `ok: boolean` | `True` iff egress returned `failed == 0`. |
| sanitized payload | `payload: dict` | `payload: Record<string, unknown>` | The egress-sanitized object. **Forward this** downstream — never `raw_response`. |
| violations | `violations: list[Violation]` | `violations: Violation[]` | Empty on success; otherwise the gateway's per-field violations. |
| attempts | `attempts: int` | `attempts: number` | 1 on happy path; up to `max_retries + 1` under `auto_repair`. |
| raw response | `raw_response: Any` | `rawResponse: unknown` | The final provider response object. For debugging only. |
| repair hints | `repair_hints: list[str]` | `repairHints: string[]` | Hints the SDK appended during auto-repair. Empty on first-try pass. |

**Never** forward `raw_response.content` to a downstream system after a
`block` or `fail` disposition — the sanitized `payload` is the only safe
object. See `docs/egress-validation-reference.md`.

## Auto-repair

When `auto_repair=True` and egress returns `failed > 0`, the SDK:

1. Formats a repair hint from the violation list (default formatter
   below; override with `repair_prompt`).
2. Appends the hint as a new **user** message. The system message is
   preserved; the hint is never injected into the system role.
3. Re-invokes the provider.
4. Repeats until egress passes or `max_retries + 1` provider calls have
   been made.

**Budget**: default `max_retries = 2` → up to 3 provider calls total.
Hard cap `MAX_RETRIES_CAP = 5`. Setting `max_retries > 5` emits a warning
and clamps.

**Default hint**:

```
The previous JSON was rejected by schema validation. Fix the violations
below and return ONLY the corrected JSON object. No prose, no markdown
fence.
- field 'amount': Field 'amount' value -3 is below minimum 0
- field 'event_type': Field 'event_type' value "purchse" not in allowed set: ["click","view","purchase","login"]
```

Override:

```python
def my_hint(violations):
    return "Please fix: " + "; ".join(v.field for v in violations)

gate = LLMGate(..., repair_prompt=my_hint)
```

```ts
const gate = new LLMGate({
  ...,
  repairPrompt: (violations) =>
    'Please fix: ' + violations.map((v) => v.field).join('; '),
});
```

**Exhausted budget**: `result.ok == False` and `result.attempts ==
max_retries + 1`. Do not forward `result.payload` to production
downstream systems in this case — surface to the human / log the
failure.

## Response-shape extraction (`extract_json`)

`LLMGate` calls this internally. It is public for callers who want to
plug extraction into their own pipeline.

Supported shapes, in priority order:

| # | Shape | Example key path |
|---|---|---|
| 1 | OpenAI chat tool-call | `response.choices[0].message.tool_calls[0].function.arguments` |
| 2 | OpenAI chat JSON mode | `response.choices[0].message.content` |
| 3 | Anthropic Messages tool_use | first `response.content[*]` with `type == "tool_use"` → `.input` |
| 4 | Anthropic Messages text (fallback) | first `response.content[*]` with `type == "text"` → `JSON.parse(.text)` |
| 5 | OpenAI Responses API | `response.output_parsed` (dict passthrough) |
| 6 | Raw string | `JSON.parse(s)`, with markdown-fence tolerance |
| 7 | Plain dict / object | passthrough |

```python
from contractgate.llm import extract_json
payload = extract_json(resp)      # resp from any shape above
```

```ts
import { extractJson } from '@nightmoose/contractgate-sdk/llm';
const payload = extractJson(resp);
```

**Markdown-fence tolerance**: strings wrapped in ``` ```json\n...\n``` ```
or ``` ```\n...\n``` ``` are unwrapped before parsing.

**Unknown shape** → `LLMResponseError` with the detected top-level type.

**Streaming responses** (iterators with no `.choices` / `.content`) →
`LLMResponseError`. Streaming is not supported in v1 (RFC-094 §Non-goals).
Use a non-streaming call, or buffer the stream yourself and pass the
accumulated text to `validate_only`.

## Error handling

### `LLMResponseError`

Subclass of `ContractGateError`. Raised when:

- The response is a streaming iterator (not supported in v1).
- The response shape is unrecognized.
- The response string is not valid JSON.
- The response JSON is not an object (array, number, string).

### `ValueError` / `TypeError`

- `LLMGate(..., llm=None)` → `ValueError` (Python) / `TypeError` (TS).
- `gate.completion(...)` on an LLM client missing `chat.completions.create`
  → `TypeError`.
- `gate.message(...)` on an LLM client missing `messages.create` →
  `TypeError`.

### Egress HTTP errors

The underlying `Client.egress` call can raise any
[`HTTPError`](../sdks/python/src/contractgate/exceptions.py) subclass
(`AuthError`, `BadRequestError`, `NotFoundError`,
`ValidationFailedError`, `ServerError`). Catch them where you catch
your other `contractgate` HTTP errors.

## Non-goals (v1)

- **Streaming output validation.** v1 rejects streaming responses. See
  RFC-094 §Non-goals.
- **"Eliminate hallucinations."** The contract catches only violations
  the contract can express (type, range, enum, pattern, undeclared field
  under compliance mode, RFC-004 transforms). Semantic truth outside the
  schema is a boundary problem.
- **Calling the LLM provider for you.** The caller owns the provider
  call; `LLMGate` only inspects the response and runs egress. See
  RFC-094 §Decisions Q1 → B.
- **First-class LangChain / CrewAI adapters.** v1 ships cookbook
  snippets in `docs/llm-output-validation.md` (shipped under RFC-095).
  Dedicated adapter packages would be a follow-up RFC.

## See also

- [RFC-094](rfcs/094-sdk-llm-output-guard.md) — `LLMGate` design.
- [RFC-095](rfcs/095-llm-agent-integration-docs-and-positioning.md) —
  the LLM output validation playbook that uses this SDK.
- [RFC-093](rfcs/093-egress-first-class.md) — egress as a first-class
  door; the engine behind `LLMGate`.
- [RFC-004](rfcs/004-pii-masking.md) — PII transforms applied on both
  ingest (for `mask_prompt`) and egress.
- [egress-validation-reference.md](egress-validation-reference.md) —
  dispositions, audit writes, PII masking semantics.
