# Pydantic, Instructor, and LangGraph — AISchemaGate

**AISchemaGate** checks LLM structured output against a deployed contract at
runtime. The Python entry point is the `aischemagate` package (an alias of
`contractgate`). Validation is `POST /egress/{contract_id}` via
`LLMGate.validate_only` or `Client.egress`. JSON Schema structured outputs from
OpenAI or Anthropic do not enforce `min`, `max`, `pattern`, `enum`, or
`compliance_mode`. The contract does.

```bash
pip install aischemagate pydantic
# Optional, for the sections below:
# pip install openai anthropic instructor langgraph
```

```python
import os
from aischemagate import Client, LLMGate

client = Client(
    base_url=os.environ.get("CONTRACTGATE_BASE_URL", "https://api.datacontractgate.com"),
    api_key=os.environ["CONTRACTGATE_API_KEY"],  # cg_live_...
)
gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
    base_url=os.environ.get("CONTRACTGATE_BASE_URL", "https://api.datacontractgate.com"),
    client=client,
    max_retries=2,
)
```

Deploy this contract (or the equivalent) before calling egress. `compliance_mode:
true` rejects fields the model added that the contract does not name.

```yaml
version: "1.0.0"
name: invoice_extract
description: LLM invoice extraction
compliance_mode: true
ontology:
  entities:
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
    - name: amount
      type: float
      required: true
      min: 0
      max: 100000
    - name: invoice_id
      type: string
      required: true
      pattern: "^INV-[0-9]{6}$"
```

Forward `result.payload` only. On `ok=False`, do not use the model object.

## OpenAI structured outputs

Provider-side JSON Schema gets the shape right. AISchemaGate checks the
semantic constraints and, with `auto_repair=True`, re-prompts up to
`max_retries`.

```python
from openai import OpenAI

llm = OpenAI()
gate_with_llm = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
    llm=llm,
    max_retries=2,
)
result = gate_with_llm.completion(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
    response_format={"type": "json_object"},
    auto_repair=True,
)
if result.ok:
    forward(result.payload)
else:
    # result.violations: field, kind, message (enum_violation, range_violation, ...)
    raise SystemExit(result.violations)
```

If you already hold the chat completion, skip the provider wrapper:

```python
completion = llm.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
    response_format={"type": "json_object"},
)
result = gate.validate_only(completion)
```

## Anthropic structured output

`LLMGate.message` calls `messages.create`. `validate_only` also reads a tool-use
block's `input`.

```python
import anthropic

anthropic_llm = anthropic.Anthropic()
result = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
    llm=anthropic_llm,
    max_retries=2,
).message(
    model="claude-sonnet-4-5",
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
    max_tokens=1024,
    tools=[{
        "name": "emit_invoice",
        "description": "Return the invoice object.",
        "input_schema": {
            "type": "object",
            "properties": {
                "currency": {"type": "string"},
                "amount": {"type": "number"},
                "invoice_id": {"type": "string"},
            },
            "required": ["currency", "amount", "invoice_id"],
        },
    }],
    tool_choice={"type": "tool", "name": "emit_invoice"},
    auto_repair=True,
)
```

## Pydantic

Parse the egress payload into the application model after the gate passes.
Pydantic's own constraints are a local mirror; the deployed contract is the
authority, including `compliance_mode`.

```python
from typing import Literal
from pydantic import BaseModel, Field

class Invoice(BaseModel):
    currency: Literal["USD", "EUR", "GBP"]
    amount: float = Field(ge=0, le=100_000)
    invoice_id: str = Field(pattern=r"^INV-[0-9]{6}$")

result = gate.validate_only({"currency": "USD", "amount": 12.5, "invoice_id": "INV-000123"})
if result.ok:
    invoice = Invoice.model_validate(result.payload)
```

A field the model invented fails the contract when `compliance_mode` is true
(`kind` `undeclared_field`) even if you discard it before `model_validate`.
Fix the object and call `validate_only` again, or use `auto_repair=True` on
`completion` / `message`.

## Instructor

Instructor returns a Pydantic instance. Run that instance through AISchemaGate,
and on a semantic failure call Instructor again with the violation text.

```python
import instructor
from openai import OpenAI

instructor_client = instructor.from_openai(OpenAI())

def extract(repair_prompt: str | None) -> Invoice:
    messages = [{"role": "user", "content": "Extract the invoice as JSON."}]
    if repair_prompt:
        messages.append({"role": "user", "content": repair_prompt})
    return instructor_client.chat.completions.create(
        model="gpt-4o-mini",
        response_model=Invoice,
        messages=messages,
    )

def validate_or_repair(max_retries: int = 2) -> dict:
    repair = None
    last = None
    for _ in range(max_retries + 1):
        draft = extract(repair)
        last = gate.validate_only(draft.model_dump())
        if last.ok:
            return last.payload
        repair = "\n".join(
            f"- field {v.field!r}: kind {v.kind}: {v.message}"
            for v in last.violations
        )
        repair = (
            "The previous JSON failed AISchemaGate validation "
            "(min, max, pattern, enum, compliance_mode).\n"
            "Return only the corrected object.\n" + repair
        )
    raise RuntimeError(last.violations if last else "no attempt")
```

## LangGraph

Put the gate on the state transition after a tool call. `compliance_mode: true`
is what makes an extra tool argument a violation instead of a silent extra key.

```python
from langgraph.graph import END, StateGraph
from typing import TypedDict

class State(TypedDict, total=False):
    draft: dict
    payload: dict
    violations: list
    repair_prompt: str

def contract_gate(state: State) -> dict:
    result = gate.validate_only(state["draft"])
    if result.ok:
        return {"payload": result.payload, "violations": [], "repair_prompt": ""}
    lines = [
        f"- field {v.field!r}: kind {v.kind}: {v.message}" for v in result.violations
    ]
    return {
        "violations": [
            {"field": v.field, "kind": v.kind, "message": v.message} for v in result.violations
        ],
        "repair_prompt": (
            "AISchemaGate rejected the tool output. Fix min, max, pattern, enum, "
            "and drop fields rejected by compliance_mode.\n" + "\n".join(lines)
        ),
    }

def route(state: State) -> str:
    return "repair" if state.get("violations") else "accept"

graph = StateGraph(State)
graph.add_node("gate", contract_gate)
graph.add_node("repair", lambda state: state)  # replace with the model node
graph.add_node("accept", lambda state: state)
graph.set_entry_point("gate")
graph.add_conditional_edges("gate", route, {"repair": "repair", "accept": "accept"})
graph.add_edge("repair", "gate")
graph.add_edge("accept", END)
app = graph.compile()
```

Bound the repair edge in the model node (stop after the same cap you would pass
as `max_retries`, default 2). The gate does not call the provider. The node
that produced `draft` does.

Direct egress, when you are not using `LLMGate`:

```python
response = client.egress(
    contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
    events=[draft],
    disposition="fail",
)
if response.failed == 0:
    forward(response.payload[0])
```
