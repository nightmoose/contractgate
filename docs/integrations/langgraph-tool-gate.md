# LangGraph tool-call gate + AISchemaGate

A tool schema tells the model which keys exist. It does not stop the model from adding a key, flipping an enum's case, or moving the agent into a state your contract does not name. Put `compliance_mode: true` on the contract and run every tool-argument object and every state patch through `POST /egress/{id}` before the tool or the transition runs.

```bash
pip install aischemagate langgraph
```

## Contract

`compliance_mode: true` rejects any top-level key that is not declared. The enum and the minimum still apply to the keys that are declared.

```yaml
version: "1.0.0"
name: book_shipment
description: Tool arguments and the agent state that may follow them
compliance_mode: true
ontology:
  entities:
    - name: destination
      type: string
      required: true
      enum: ["us-east", "eu-west"]
    - name: weight_kg
      type: float
      required: true
      min: 0
      max: 30
    - name: service
      type: string
      required: true
      enum: ["ground", "air"]
```

`{"destination": "US-EAST", "weight_kg": -1, "service": "air", "debug": true}` fails three ways: enum case, minimum, and an undeclared `debug` key. The graph must not call the carrier tool with that object.

## Gate

`LLMGate.validate_only` posts one object to egress and returns `ok`, `payload`, and `violations`. `dry_run=True` performs the check without an audit row. Use `dry_run=False` when this call should be the record of the decision.

```python
import os
from aischemagate import LLMGate

gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
)

def allow(payload: dict, *, dry_run: bool = True) -> dict:
    result = gate.validate_only(payload, dry_run=dry_run)
    if not result.ok:
        return {
            "ok": False,
            "violations": [
                {"field": v.field, "kind": v.kind, "message": v.message}
                for v in result.violations
            ],
        }
    return {"ok": True, "payload": result.payload}
```

## Tool arguments

Wrap the tool. Call the carrier only with `decision["payload"]`.

```python
from langchain_core.tools import tool

@tool
def book_shipment(destination: str, weight_kg: float, service: str) -> str:
    """Book a shipment. Arguments are checked before this body runs."""
    decision = allow(
        {"destination": destination, "weight_kg": weight_kg, "service": service}
    )
    if not decision["ok"]:
        return "rejected: " + "; ".join(
            f"{v['field']}: {v['message']}" for v in decision["violations"]
        )
    booked = carrier.book(**decision["payload"])
    return booked.confirmation
```

If you already have a `ToolNode`, validate inside the tool body as above so a bad call never reaches `carrier.book`. The model's next step sees the violation text and can correct the arguments.

## State transitions

Treat the patch the model wants to write as another egress event. Undeclared keys and illegal enum values never land in `state`.

```python
from langgraph.graph import StateGraph, END

def apply_transition(state: dict) -> dict:
    patch = state.get("proposed") or {}
    decision = allow(patch)
    if not decision["ok"]:
        return {
            "phase": "needs_repair",
            "violations": decision["violations"],
            "proposed": None,
        }
    return {"phase": "booked", "shipment": decision["payload"], "violations": []}

def route(state: dict) -> str:
    return "repair" if state.get("phase") == "needs_repair" else END

graph = StateGraph(dict)
graph.add_node("transition", apply_transition)
graph.add_node("repair", lambda state: {"proposed": ask_model_to_fix(state)})
graph.set_entry_point("transition")
graph.add_conditional_edges("transition", route, {"repair": "repair", END: END})
graph.add_edge("repair", "transition")
```

`ask_model_to_fix` should include `state["violations"]` in the prompt, the same list egress returned (`field`, `kind`, `message`). Cap that loop in the node (two extra attempts is enough). After the cap, leave `phase` at `needs_repair` instead of writing `shipment`.

The same `allow` helper is the check for both surfaces. Tool arguments and state patches that violate `compliance_mode` stop in the graph; they are not forwarded to the tool or stored as the new state.
