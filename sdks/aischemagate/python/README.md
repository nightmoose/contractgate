# aischemagate

**AISchemaGate** — thin alias package over
[`contractgate`](https://pypi.org/project/contractgate/) for LLM
structured-output validation. Same Rust engine, same `cg_live_...` API
keys, same tenant service, same backend as
[DataContractGate](https://datacontractgate.com).

```bash
pip install aischemagate
```

## Why this package exists

The underlying engine, SDK, and API are **DataContractGate** — one validator,
one gateway. `aischemagate` is the LLM / AI-agent-shaped front door: it
re-exports `contractgate` under a name that reads correctly from the
LLM-output-validation angle. Use whichever import name matches your
mental model; the behavior is identical.

See:

- `aischemagate.com` — product site
- `aischemagate.dev` — developer documentation
- RFC-094 (`docs/rfcs/094-sdk-llm-output-guard.md`) — SDK design

## 10-line quickstart

```python
import os
from aischemagate import LLMGate
from openai import OpenAI

gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
    llm=OpenAI(),
)
result = gate.completion(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Extract invoice JSON from: ..."}],
    response_format={"type": "json_object"},
    auto_repair=True,
)
if result.ok:
    forward_downstream(result.payload)   # egress-sanitized; safe to forward
```

For Anthropic, call `gate.message(model="claude-sonnet-4-5", messages=[...])`.
For vLLM / Ollama OpenAI-compat, call `gate.completion(...)` unchanged.

## Framework adapters

Each adapter is an optional extra. `import aischemagate` does not import
LangChain, LlamaIndex, or Instructor. The adapters call
`LLMGate.validate_only`, which posts the JSON object to
`POST /egress/{contract_id}`. A contract name is resolved with
`GET /contracts`. A UUID is used as the path id directly. `parse` (and the
Instructor hook, after a passing check) returns the egress payload, with
declared transforms already applied.

They validate a finished response. A token stream is rejected. Streaming
chunk validation is not part of this package.

```bash
pip install 'aischemagate[langchain]'
pip install 'aischemagate[llamaindex]'
pip install 'aischemagate[instructor]'
```

### LangChain

`AISchemaGateParser` is a `BaseOutputParser`. `parse` returns the egress
payload, or raises `OutputParserException`.

```python
import os
from aischemagate.langchain import AISchemaGateParser

parser = AISchemaGateParser(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",  # UUID or contract name
)
payload = parser.parse(model_text)  # dict, or OutputParserException
```

`parser` can sit at the end of a chain. `prompt` is a
`langchain_core.prompts.ChatPromptTemplate`, and `llm` is a chat model:
`chain = prompt | llm | parser`.

### LlamaIndex

Same parser for LlamaIndex `BaseOutputParser`. `parse` returns the egress
payload, or raises `AISchemaGateRejected` (a `ValueError`). A
`llama_index.core.prompts.PromptTemplate` calls `format` (and
`format_messages`) on its `output_parser`, which appends a JSON instruction
to the query.

```python
import os
from aischemagate.llamaindex import AISchemaGateParser

parser = AISchemaGateParser(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
)
payload = parser.parse(response_text)
```

### Instructor

`AISchemaGateHook.guard` puts the structural check on the response model
Instructor validates. The check runs before Pydantic field validation. A
structural failure is the validation error; field validators do not run.
When the check passes, Pydantic validates the egress payload.

```python
import os
import instructor
from openai import OpenAI
from pydantic import BaseModel
from aischemagate.instructor import AISchemaGateHook

hook = AISchemaGateHook(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="extract_invoice_v1",
)

@hook.guard
class Invoice(BaseModel):
    total: float
    currency: str

llm = instructor.from_openai(OpenAI())
invoice = llm.chat.completions.create(
    model="gpt-4o-mini",
    response_model=Invoice,
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
)
```

## What's new

See [CHANGELOG.md](CHANGELOG.md). The root import still tracks
[`contractgate`](../../python/CHANGELOG.md). Framework adapters are
versioned on this package.
