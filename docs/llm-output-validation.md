# LLM Output Validation Playbook

Validate structured JSON output from an LLM against a semantic contract
before it reaches your database, application logic, or downstream API.
Same Rust engine as DataContractGate; same `cg_live_...` API key; same
backend. The difference is the ergonomic SDK wrapper and the
documentation entry point.

If your job is to **wire up data ingestion** (Kafka, Kinesis, warehouse),
read [`llm-integration.md`](llm-integration.md) instead. This playbook is
for the **AI output** use case.

- **Product:** AISchemaGate — <https://aischemagate.com>
- **Docs:** <https://aischemagate.dev>
- **API base URL:** `https://api.datacontractgate.com`
- **SDK design:** [RFC-094](rfcs/094-sdk-llm-output-guard.md)
- **Integrations:** [Vercel AI SDK](integrations/vercel-ai-sdk.md), [OpenAI / Anthropic / Pydantic / Instructor](integrations/openai-anthropic-structured-output.md), [LangGraph tool gate](integrations/langgraph-tool-gate.md)

---

## §0 Install

| Language | AISchemaGate | DataContractGate (same objects, different name) |
|---|---|---|
| Python ≥ 3.9 | `pip install aischemagate` | `pip install "contractgate>=0.3.0"` |
| Node ≥ 20 | `npm install @nightmoose/aischemagate` | `npm install @nightmoose/contractgate-sdk@^0.3.0` |

Set your API key:

```bash
export CONTRACTGATE_API_KEY="cg_live_..."
```

Both brands share the same key issuance (same dashboard, same tenant).
See [`auth-reference.md`](auth-reference.md) if you don't have a key yet.

## §1 Author an output contract

Collect 5–20 **known-good** JSON outputs from your LLM — the ones your
pipeline would be happy to accept. Hand them to `POST /contracts/infer`
and review the generated YAML before you deploy. The brief case for 5–20
samples is in [RFC-093](rfcs/093-egress-first-class.md) — fewer samples
under-constrain the schema and let hallucinations through.

```bash
curl -X POST https://api.datacontractgate.com/contracts/infer \
     -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
     -H "Content-Type: application/json" \
     -d @samples.json
```

Review, tighten (add `enum`, `min`, `max`, `pattern` where you know the
truth), then promote via `POST /contracts/deploy`. See
[`deploy-contract-reference.md`](deploy-contract-reference.md).

A worked example contract this playbook validates against:

```yaml
version: "1.0"
name: "extract_invoice_v1"
description: "LLM-extracted invoice totals"

ontology:
  entities:
    - name: invoice_id
      type: string
      required: true
      pattern: "^INV-[0-9]{6}$"
    - name: vendor
      type: string
      required: true
      min_length: 1
      max_length: 200
    - name: amount
      type: number
      required: true
      min: 0
      max: 1000000
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
    - name: due_date
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
```

## §2 Validate one output

```python
import os
from aischemagate import LLMGate
from openai import OpenAI

openai_client = OpenAI()
gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
    llm=openai_client,
)

resp = openai_client.chat.completions.create(
    model="gpt-4o-mini",
    response_format={"type": "json_object"},
    messages=[{"role": "user", "content": "Extract invoice JSON from: ..."}],
)
result = gate.validate_only(resp)
if result.ok:
    store_invoice(result.payload)           # egress-sanitized; safe to forward
```

```ts
import { LLMGate } from '@nightmoose/aischemagate';
import OpenAI from 'openai';

const openaiClient = new OpenAI();
const gate = new LLMGate({
  apiKey: process.env.CONTRACTGATE_API_KEY!,
  contractId: 'extract_invoice_v1',
  llm: openaiClient,
});

const resp = await openaiClient.chat.completions.create({
  model: 'gpt-4o-mini',
  response_format: { type: 'json_object' },
  messages: [{ role: 'user', content: 'Extract invoice JSON from: ...' }],
});
const result = await gate.validateOnly(resp);
if (result.ok) storeInvoice(result.payload);
```

The returned `payload` is the **egress-sanitized** form the gateway
produced via `POST /egress/{contract_id}` — forward this downstream. Do
not forward `resp.choices[0].message.content` after a `block` or `fail`
disposition. See [`egress-validation-reference.md`](egress-validation-reference.md).

## §3 Validate with auto-repair

Let the SDK re-prompt the model with the violation list when validation
fails. Hard-capped at 5 total retries; default is 2.

```python
result = gate.completion(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Extract invoice JSON..."}],
    response_format={"type": "json_object"},
    auto_repair=True,
)
# result.attempts == 1 on a clean first-try pass; up to max_retries + 1 on exhaust.
# result.repair_hints lists the hints the SDK fed back on retries.
```

```ts
const result = await gate.completion({
  model: 'gpt-4o-mini',
  messages: [{ role: 'user', content: 'Extract invoice JSON...' }],
  response_format: { type: 'json_object' },
  autoRepair: true,
});
```

The repair hint is appended as a new **user** message, never injected
into the system role. Default formatter lists each `field: message`
pair; override via the `repair_prompt` / `repairPrompt` constructor
option.

**Exhausted budget**: `result.ok == false`. Do not forward
`result.payload` — surface to a human / log and move on. The sanitized
payload is still well-formed, which is misleading if you treat it as
authoritative.

## §4 Mask the prompt (optional)

Prompts can carry user PII (emails, SSNs, account IDs). Author a prompt
contract with [RFC-004](rfcs/004-pii-masking.md) `transform:` rules on
those fields and let the SDK strip them before you send the prompt to
the LLM.

```python
clean = gate.mask_prompt(
    {"email": "alice@example.com", "ssn": "123-45-6789", "note": "..."},
    prompt_contract_id="my_prompt_shape_v1",
)
# clean == {"email": "hmac-sha256:...", "ssn": "<REDACTED>", "note": "..."}
prompt = f"Classify this message from {clean['email']}: {clean['note']}"
```

```ts
const clean = await gate.maskPrompt(
  { email: 'alice@example.com', ssn: '123-45-6789', note: '...' },
  { promptContractId: 'my_prompt_shape_v1' },
);
```

Under the hood this is `POST /v1/ingest/{contract_id}` with
`dry_run=true`; the transforms run server-side and the sanitized dict
comes back. Never send the raw dict to the LLM after you have a prompt
contract — the transformed form is the trustworthy one.

## §5 Provider quickstarts

### §5.1 OpenAI (JSON mode)

Covered in §2. For OpenAI's structured-outputs feature, pass
`response_format={"type": "json_schema", "json_schema": {...}}` — the
SDK forwards it to the provider unchanged. The schema you send to OpenAI
and the contract you deploy are independent surfaces; keep them in sync
(or infer both from the same sample set).

### §5.2 OpenAI tool calling

```python
tools = [{
    "type": "function",
    "function": {
        "name": "store_invoice",
        "description": "Store an extracted invoice",
        "parameters": {},   # same shape your contract expects
    },
}]
result = gate.completion(
    model="gpt-4o",
    messages=[{"role": "user", "content": "..."}],
    tools=tools,
    tool_choice={"type": "function", "function": {"name": "store_invoice"}},
)
# LLMGate picks tool_calls[0].function.arguments automatically.
```

### §5.3 Anthropic tool use

Anthropic uses `.message()` not `.completion()`.

```python
import anthropic

anthropic_client = anthropic.Anthropic()
gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
    llm=anthropic_client,
)
tools = [{
    "name": "store_invoice",
    "description": "Store an extracted invoice",
    "input_schema": {},
}]
result = gate.message(
    model="claude-sonnet-4-5",
    messages=[{"role": "user", "content": "Extract invoice JSON..."}],
    max_tokens=1024,
    tools=tools,
    tool_choice={"type": "tool", "name": "store_invoice"},
    auto_repair=True,
)
# LLMGate picks the first content[*].type == 'tool_use' block automatically.
```

```ts
import Anthropic from '@anthropic-ai/sdk';
const anthropicClient = new Anthropic();
const gate = new LLMGate({
  apiKey: process.env.CONTRACTGATE_API_KEY!,
  contractId: 'extract_invoice_v1',
  llm: anthropicClient,
});
const result = await gate.message({
  model: 'claude-sonnet-4-5',
  messages: [{ role: 'user', content: 'Extract invoice JSON...' }],
  maxTokens: 1024,
  tools: [{ name: 'store_invoice', input_schema: {} }],
  tool_choice: { type: 'tool', name: 'store_invoice' },
  autoRepair: true,
});
```

### §5.4 vLLM / Ollama (OpenAI-compat)

Both speak the OpenAI protocol, so `gate.completion()` works unchanged.
Point the `OpenAI` client at your local endpoint:

```python
from openai import OpenAI
local = OpenAI(base_url="http://localhost:11434/v1", api_key="ollama")
gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
    llm=local,
)
```

### §5.5 LangChain

Use `validate_only` after your chain returns. The LLMGate extractor
handles LangChain's `AIMessage` shape via the OpenAI chat fallback (it
reads `.content`); for tool-call outputs, pass the raw `ChatCompletion`
through.

```python
from langchain_openai import ChatOpenAI
llm = ChatOpenAI(model="gpt-4o-mini", response_format={"type": "json_object"})
chain = prompt_template | llm
ai_msg = chain.invoke({"input": "..."})
result = gate.validate_only(ai_msg.content)   # ai_msg.content is the JSON string
```

### §5.6 CrewAI

Same `validate_only` pattern: the crew returns a task output; pass the
string through.

```python
from crewai import Crew
crew = Crew(agents=[], tasks=[])
output = crew.kickoff(inputs={"input": "..."})
result = gate.validate_only(str(output))
```

### §5.7 MCP agents

If your agent runs through an MCP host that has
`@nightmoose/contractgate-mcp-server` connected, call the shipped
`egress_validate` tool directly — the agent already has the JSON object
after its LLM step, so `LLMGate`'s extractor is unnecessary. See
[`mcp-reference.md`](mcp-reference.md) for the full tool surface.

## §6 Verify

Before you declare the integration done, run one known-bad and one
known-good object through `validate_only` and check the result:

```python
# Known-bad: schema-valid wrong event (amount is negative — contract says min: 0).
bad = {"invoice_id": "INV-000001", "vendor": "ACME",
       "amount": -49.99, "currency": "USD", "due_date": "2026-11-01"}
assert gate.validate_only(bad).ok is False

# Known-good.
good = {"invoice_id": "INV-000001", "vendor": "ACME",
        "amount": 49.99, "currency": "USD", "due_date": "2026-11-01"}
assert gate.validate_only(good).ok is True
```

If both assertions hold, the gate is wired. Ship.

## §7 Troubleshooting

### `LLMResponseError: streaming responses are not supported in v1`

v1 of the SDK does not validate streamed tokens. Buffer the stream
yourself and pass the final string, or set `stream=False` on the
provider call. See [RFC-094](rfcs/094-sdk-llm-output-guard.md) §Non-goals
for the rationale. Follow-up RFC pending user demand.

### `LLMResponseError: unrecognized response shape (type=...)`

The extractor supports OpenAI chat (JSON mode and tool_calls), OpenAI
Responses `output_parsed`, Anthropic `tool_use` and text blocks, raw
JSON strings (with markdown fence tolerance), and dict passthrough.
Anything else — call `extract_json` yourself or pass the already-parsed
dict to `validate_only`.

### `LLMResponseError: response is not valid JSON`

The model returned prose, or a partial JSON fragment. Enable
`auto_repair=True` to re-prompt with a schema-only hint; or tighten your
system prompt. For OpenAI, `response_format={"type": "json_object"}` is
mandatory for JSON mode.

### `ValidationFailedError` (HTTP 422)

A whole-batch rejection from the gateway. Different from
`LLMResponseError`. Most commonly: your contract has no stable version
yet, or the version pin references a deprecated version. See
[`egress-validation-reference.md`](egress-validation-reference.md) and
[`deploy-contract-reference.md`](deploy-contract-reference.md).

### `result.payload` is `{}` after a failure

That is the egress-sanitized form of the rejected payload. The server
produced an empty object on `block` because none of the record passed.
Do not forward it; surface the `result.violations` to the human.

### `violations[].actual_value` for a transformed field looks "already fixed"

[RFC-004](rfcs/004-pii-masking.md) Q7: fields with a `transform:` carry
the **transformed** value in the violation, not the raw. If the model
produced an invalid email, the violation shows `hmac-sha256:...` not
the raw address. This is a privacy invariant — the raw never leaves the
validator.

### "Forward only `payload`" — what does that mean concretely?

RFC-093 §Design 1 — on `block` or `fail`, the original request body is
no longer safe to forward (it contains the rejected fields). The
`payload` field in the egress response is the sanitized form:
only-passing records, with RFC-004 transforms applied. Build your
downstream call from `result.payload` not from the raw LLM response.

## See also

- [RFC-094](rfcs/094-sdk-llm-output-guard.md) — `LLMGate` SDK design.
- [RFC-095](rfcs/095-llm-agent-integration-docs-and-positioning.md) —
  this playbook's design + aischemagate.com copy lock.
- [RFC-093](rfcs/093-egress-first-class.md) — egress as a first-class
  door; the engine behind `LLMGate`.
- [`llm-sdk-reference.md`](llm-sdk-reference.md) — full `LLMGate` API
  reference, Python + TypeScript side-by-side.
- [`llm-integration.md`](llm-integration.md) — agent-executable playbook
  for data ingestion (streaming, warehouse, Kafka, Kinesis).
- [`egress-validation-reference.md`](egress-validation-reference.md) —
  the HTTP endpoint and semantics `LLMGate` wraps.
