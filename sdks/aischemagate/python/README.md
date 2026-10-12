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

The underlying engine, SDK, and API are **ContractGate** — one validator,
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

## What's new

See [CHANGELOG.md](CHANGELOG.md) and the parent
[`contractgate` CHANGELOG](../../python/CHANGELOG.md) — the alias version
tracks initial release only; feature work is versioned on `contractgate`.
