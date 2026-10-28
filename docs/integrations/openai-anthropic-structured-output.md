# OpenAI, Anthropic, Pydantic, Instructor + AISchemaGate

Provider structured outputs and Pydantic models guarantee a JSON shape. They do not guarantee the values your contract names. A negative total, an enum in the wrong case, and an extra key all parse cleanly. AISchemaGate checks those rules on `POST /egress/{contract_id}` and can re-prompt with the violations.

```bash
pip install aischemagate openai anthropic pydantic instructor
```

`aischemagate` is the LLM-facing name for the same gateway and the same `cg_live_...` key as DataContractGate.

## Contract

```yaml
version: "1.0.0"
name: invoice_extract
description: Semantic checks JSON Schema does not express
compliance_mode: true
ontology:
  entities:
    - name: total
      type: float
      required: true
      min: 0
      max: 1000000
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
    - name: invoice_id
      type: string
      required: true
      pattern: "^INV-[0-9]{6}$"
```

Deploy that YAML, then point the gate at the contract id.

## OpenAI JSON schema or Anthropic tool input

The schema below is what structured outputs enforce. `total` is only a number. `currency` is only a string. `additionalProperties` is often left open. All three of the following are accepted by that schema and rejected by the contract:

| Payload | Why the schema allows it | Contract violation |
|---|---|---|
| `{"total": -20, "currency": "USD", "invoice_id": "INV-000123"}` | number has no minimum | `range_violation` on `total` (`min: 0`) |
| `{"total": 20, "currency": "usd", "invoice_id": "INV-000123"}` | string has no enum | `enum_violation` on `currency` |
| `{"total": 20, "currency": "USD", "invoice_id": "INV-000123", "debug": true}` | extra key | `undeclared_field` because `compliance_mode: true` |

```python
import os
from aischemagate import LLMGate
from openai import OpenAI

gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
    llm=OpenAI(),
)

result = gate.completion(
    model="gpt-5.4",
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
    response_format={"type": "json_object"},
    auto_repair=True,  # up to 2 extra calls; hard cap 5
)

if result.ok:
    store(result.payload)  # egress payload, transforms already applied
else:
    review(result.violations)
```

Anthropic is the same gate with `gate.message(model="claude-sonnet-5.5", messages=[...], auto_repair=True)`. The repair prompt includes each violation's field and message. `result.attempts` is how many provider calls ran.

## Pydantic and Instructor

A Pydantic model mirrors the JSON schema, not the contract. Instructor will return an `Invoice` for a negative total and for `currency="usd"`.

```python
import os
import instructor
from openai import OpenAI
from pydantic import BaseModel
from aischemagate import LLMGate

class Invoice(BaseModel):
    total: float
    currency: str
    invoice_id: str

llm = instructor.from_openai(OpenAI())
draft = llm.chat.completions.create(
    model="gpt-5.4",
    response_model=Invoice,
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
)

gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
    llm=OpenAI(),
)

# validate_only accepts a dict. It does not call the provider.
checked = gate.validate_only(draft.model_dump())
if not checked.ok:
    # One bounded repair: feed the violations back through Instructor.
    fixed = llm.chat.completions.create(
        model="gpt-5.4",
        response_model=Invoice,
        messages=[
            {"role": "user", "content": "Extract the invoice as JSON."},
            {
                "role": "user",
                "content": "Fix these semantic violations and keep the same keys:\n"
                + "\n".join(f"- {v.field}: {v.message}" for v in checked.violations),
            },
        ],
    )
    checked = gate.validate_only(fixed.model_dump())

if checked.ok:
    store(checked.payload)
```

Use `gate.completion(..., auto_repair=True)` when you do not need the Pydantic instance and want the gate to own the retry loop. Use `validate_only` when Instructor (or any other parser) already built the object and you only need the semantic check.

To run the structural check before Pydantic field validation, install
`aischemagate[instructor]` and decorate the response model with
`AISchemaGateHook.guard`. A contract rejection is reported then, and field
validators do not run. See
[`python-framework-adapters.md`](python-framework-adapters.md).

Do not persist `draft` when `checked.ok` is false. The interesting failures are value failures on a payload that already parsed.
