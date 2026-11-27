# Gate an agent's tools

Check what an AI agent sends to a tool, and what the tool sends back, against a contract. A bad call stops at the gate instead of reaching the tool, and a bad result stops before the agent reads it.

- **AISchemaGate** is the name for this use: structured model output, tool-call arguments and tool results.
- **DataContractGate** is the same engine, API key, contracts and quarantine for data pipelines.

Both call the same gateway. The commands and snippets below were run against a 0.4.2 gateway with the CLI, the published SDKs and the MCP server.

**Where this stops.** The gate checks the *values* an agent passes and receives: types, enums, ranges, patterns, undeclared keys, masked or dropped fields. It does not decide *who* may call a tool, it does not read the agent's instructions, and it cannot tell whether a well-formed call is a wise one. See [What DataContractGate is not](#what-datacontractgate-is-not).

## What a review of an agent asks

Reviews of agent changes usually ask three questions. Here is what the gate covers for each.

| Question | What the gate does today | Limit |
|---|---|---|
| **Instructions.** What is the agent told to do? | The contract is a reviewable, versioned statement of what its tool calls and tool results must look like. It lives in YAML in your repo and changes in the same pull request as the prompt. | It never sees or stores prompts. |
| **Access.** Which operations may the agent perform? | An `enum` on the operation field plus `compliance_mode: true` rejects any operation you did not list and any argument you did not declare. | It is a value allowlist. It does not know *who* is calling, and there is no deny-wins-over-allow composition. API keys can be limited to a set of contracts (`allowed_contract_ids`, see [auth-reference.md](../auth-reference.md)), which is not per-user policy. |
| **Data.** What may a tool response expose? | Per-field `transform` (`mask`, `hash`, `drop`, `redact`) and `egress_leakage_mode: strip` or `fail` remove or reject fields you did not declare. See [pii-masking-reference.md](../pii-masking-reference.md). | Top-level fields only. A nested object is checked for its declared keys, but undeclared keys inside it pass, and transforms apply to top-level strings. |

## Worked example: a read-only CRM lookup tool

Two contracts: one for the arguments the agent may pass, one for what the tool may return.

`contracts/crm_lookup_args.yaml`

```yaml
version: "1.0"
name: crm_lookup_args
description: Arguments an agent may pass to the read-only CRM lookup tool
compliance_mode: true
ontology:
  entities:
    - name: operation
      type: string
      required: true
      enum: ["get_customer", "search_customers"]
    - name: customer_id
      type: string
      required: true
      pattern: "^cus_[0-9]{6}$"
    - name: limit
      type: integer
      required: false
      min: 1
      max: 25
```

- `operation` is an allowlist: `delete_customer` is rejected even though the tool function might support it.
- `customer_id` has a pattern, so `cus_000001; DROP TABLE customers` is rejected.
- `limit` is optional and bounded.
- `compliance_mode: true` rejects any key not declared, such as a `debug` flag the model invented.

`contracts/crm_lookup_result.yaml`

```yaml
version: "1.0"
name: crm_lookup_result
description: What the CRM lookup tool may hand back to the agent
egress_leakage_mode: strip
ontology:
  entities:
    - name: customer_id
      type: string
      required: true
      pattern: "^cus_[0-9]{6}$"
    - name: plan
      type: string
      required: true
      enum: ["free", "pro", "enterprise"]
    - name: email
      type: string
      required: true
      transform:
        kind: mask
        style: format_preserving
    - name: ssn
      type: string
      required: false
      transform:
        kind: drop
```

- `email` is masked with the same length and character classes, so the agent can still reason about its shape.
- `ssn` is dropped from what the agent receives.
- `egress_leakage_mode: strip` removes any field the tool returned that the contract does not declare (an `internal_notes` column, say) and lists it in `stripped_fields`.

Check both compile:

```bash
contractgate validate --dir contracts
# PASS  contracts/crm_lookup_args.yaml
# PASS  contracts/crm_lookup_result.yaml
```

## Test the contracts before an agent uses them

`contractgate synth` writes rows from a contract's own rules. No server, no network, no real data. See [synthetic-data-reference.md](../synthetic-data-reference.md).

```bash
# Rows that should pass
contractgate synth contracts/crm_lookup_args.yaml --rows 50 --seed 1 -o good.json
contractgate test --contract contracts/crm_lookup_args.yaml --data good.json --quiet   # exit 0
```

### Negative tests with `synth --invalid`

`--invalid` makes every row break exactly one rule and labels it:

```bash
contractgate synth contracts/crm_lookup_args.yaml --invalid --rows 4 --seed 1
```

```json
[
  {
    "broken_rule": {
      "field": "operation",
      "message": "Required field 'operation' is missing",
      "rule": "required",
      "violation_kind": "missing_required_field"
    },
    "record": {"customer_id": "cus_069616", "limit": 7}
  }
]
```

(One row of the output is shown. Other rows break `type`, `compliance_mode`, `enum`, `pattern` and `min` / `max`, one rule each.) Exit codes for `synth`: 0 rows written, 1 a rule cannot be satisfied, 2 the contract cannot be read.

The recipe: every invalid row must be rejected by `contractgate test`, and for the rule it was built to break. Save as `scripts/negative-tests.sh`:

```bash
#!/usr/bin/env bash
# Every row from `synth --invalid` must be rejected, and for the rule it was built to break.
set -euo pipefail
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for contract in contracts/*.yaml; do
  name=$(basename "$contract" .yaml)
  contractgate synth "$contract" --invalid --rows 100 --seed 1 -o "$work/$name.rows.json"

  # synth --invalid writes [{"record": ..., "broken_rule": {...}}, ...].
  python3 - "$work/$name.rows.json" "$work/$name.records.json" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
json.dump([r["record"] for r in rows], open(sys.argv[2], "w"))
PY

  # contractgate test exits 1 when records fail. Anything else means the harness broke.
  code=0
  contractgate test --contract "$contract" --data "$work/$name.records.json" \
    --format json --quiet > "$work/$name.result.json" || code=$?
  [ "$code" -eq 1 ] || { echo "$name: expected exit 1 from contractgate test, got $code"; exit 1; }

  python3 - "$name" "$work/$name.rows.json" "$work/$name.result.json" <<'PY'
import json, sys
name, rows_path, result_path = sys.argv[1:4]
rows = json.load(open(rows_path))
result = json.load(open(result_path))
assert result["fail"] == result["total"] == len(rows), f"{name}: some invalid rows passed"
by_index = {r["record"]: r for r in result["records"]}
for i, row in enumerate(rows):
    broken = row["broken_rule"]["field"]
    fields = {v["field"] for v in by_index[i]["violations"]}
    assert broken in fields, f"{name}: row {i} was rejected, but not for '{broken}' ({sorted(fields)})"
print(f"{name}: {len(rows)} of {len(rows)} invalid rows rejected, each for its labeled rule")
PY

  # Hand-written cases: the calls you must never allow. synth cannot see a rule
  # that was deleted from the contract, so these catch that.
  cases="tests/must-reject/$name.json"
  if [ -f "$cases" ]; then
    code=0
    contractgate test --contract "$contract" --data "$cases" --format json --quiet \
      > "$work/$name.cases.json" || code=$?
    [ "$code" -eq 1 ] || { echo "$name: a must-reject case was accepted (exit $code)"; exit 1; }
    python3 - "$name" "$work/$name.cases.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[2]))
assert r["fail"] == r["total"], f"{sys.argv[1]}: a must-reject case passed"
print(f"{sys.argv[1]}: {r['total']} of {r['total']} must-reject cases rejected")
PY
  fi
done
```

`tests/must-reject/crm_lookup_args.json` holds the calls you must never allow, written by hand:

```json
[
  {"operation": "delete_customer", "customer_id": "cus_000001"},
  {"operation": "get_customer", "customer_id": "cus_000001", "debug": true},
  {"operation": "get_customer", "customer_id": "cus_000001; DROP TABLE customers"},
  {"operation": "search_customers", "customer_id": "cus_000001", "limit": 5000}
]
```

Why both:

- `synth --invalid` can only break rules that are in the contract. If someone deletes the `enum`, no row is generated for it and the generated rows still all fail, so the script stays green. The must-reject file catches that: `delete_customer` is accepted, and the script fails.
- For an `enum` the generated bad value is a random string outside the list. It will not be `delete_customer`, so name the operations you care about yourself.
- `synth` and `test` run on the contract only. They do not run `transform` or `egress_leakage_mode`; check those through the gateway (below).
- `contractgate test --format json` prints a per-record line before the JSON unless you add `--quiet`. Use `--quiet` whenever a script parses the output.

Run it in CI after installing the `contractgate` binary (see the README):

```yaml
name: agent-tool-contracts
on:
  pull_request:
    paths: ["contracts/**", "tests/must-reject/**"]
jobs:
  negative-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # Install the contractgate binary here (release tarball or `cargo install`).
      - run: ./scripts/negative-tests.sh
```

## Deploy the contracts

The CLI reads the gateway URL from `.contractgate.yml`. Without one it defaults to `http://localhost:3000`.

```yaml
# .contractgate.yml
version: "1.0"
gateway:
  url: https://api.datacontractgate.com
contracts:
  dir: ./contracts
  pattern: "*.yaml"
```

```bash
export CONTRACTGATE_API_KEY=cg_live_...
contractgate deploy-contract contracts/crm_lookup_args.yaml
contractgate deploy-contract contracts/crm_lookup_result.yaml
curl -s https://api.datacontractgate.com/contracts -H "x-api-key: $CONTRACTGATE_API_KEY"   # ids and names
```

`GET /contracts` lists ids and names. `deploy-contract` needs a key allowed to deploy. See [deploy-contract-reference.md](../deploy-contract-reference.md). SDKs and the MCP server accept the contract's name or its UUID.

## Gate the call

Tool arguments and tool results go through `POST /egress/{contract_id}`. It is the same validation engine as ingest; the name is the direction. Use `disposition=fail` so one bad field rejects the whole call, and **forward only `payload`**, never the original object.

### HTTP

```bash
# Allowed: 200, payload is returned
curl -s -X POST "https://api.datacontractgate.com/egress/$ARGS_ID?disposition=fail&dry_run=true" \
  -H "x-api-key: $CONTRACTGATE_API_KEY" -H "content-type: application/json" \
  -d '{"operation":"get_customer","customer_id":"cus_123456"}'

# Refused: 422, payload is empty, violations name each problem
curl -s -X POST "https://api.datacontractgate.com/egress/$ARGS_ID?disposition=fail&dry_run=true" \
  -H "x-api-key: $CONTRACTGATE_API_KEY" -H "content-type: application/json" \
  -d '{"operation":"delete_customer","customer_id":"cus_123456","debug":true}'
```

The second call returns `422` with two violations, `enum_violation` on `operation` and `undeclared_field` on `debug`. Passing the tool result through the result contract with `{"customer_id":"cus_123456","plan":"pro","email":"ada@example.com","ssn":"123-45-6789","internal_notes":"vip"}` returns `200` and a payload with the email masked, `ssn` removed and `internal_notes` listed in `stripped_fields`.

`dry_run=true` checks without writing an audit row or quarantine row. Leave it off in production so the decision is recorded.

### Python (AISchemaGate)

```bash
pip install aischemagate
```

```python
import os

from aischemagate import LLMGate

BASE_URL = os.environ.get("CONTRACTGATE_BASE_URL", "https://api.datacontractgate.com")

args_gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="crm_lookup_args",   # contract name or UUID
    base_url=BASE_URL,
    disposition="fail",              # reject the whole call if anything is wrong
)
result_gate = LLMGate(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id="crm_lookup_result",
    base_url=BASE_URL,
    disposition="fail",
)

DRY_RUN = os.environ.get("GATE_DRY_RUN", "0") == "1"  # True: check only, no audit row


def lookup_customer(call: dict) -> dict:
    """The tool body. `call` is what the model asked for."""
    checked = args_gate.validate_only(call, dry_run=DRY_RUN)
    if not checked.ok:
        return {"error": "rejected", "violations": [v.message for v in checked.violations]}

    raw = crm_lookup(**checked.payload)  # your tool; only gated arguments reach it

    shaped = result_gate.validate_only(raw, dry_run=DRY_RUN)
    if not shaped.ok:
        return {"error": "tool result rejected", "violations": [v.message for v in shaped.violations]}
    return shaped.payload  # masked, stripped; the agent never sees `raw`


def crm_lookup(operation, customer_id, limit=None):  # stand-in for the real tool
    return {"customer_id": customer_id, "plan": "pro", "email": "ada@example.com",
            "ssn": "123-45-6789", "internal_notes": "vip"}


if __name__ == "__main__":
    print(lookup_customer({"operation": "get_customer", "customer_id": "cus_123456"}))
    print(lookup_customer({"operation": "delete_customer", "customer_id": "cus_123456"}))
    print(lookup_customer({"operation": "get_customer", "customer_id": "cus_123456", "debug": True}))
```

`validate_only` takes a plain dict (a parsed tool call) as well as a provider response. For a LangGraph version with state transitions, see [langgraph-tool-gate.md](langgraph-tool-gate.md).

### TypeScript (AISchemaGate)

```bash
npm install @nightmoose/aischemagate
```

```js
import { LLMGate } from '@nightmoose/aischemagate';

const baseUrl = process.env.CONTRACTGATE_BASE_URL ?? 'https://api.datacontractgate.com';
const apiKey = process.env.CONTRACTGATE_API_KEY;

const argsGate = new LLMGate({ apiKey, contractId: 'crm_lookup_args', baseUrl, disposition: 'fail' });
const resultGate = new LLMGate({ apiKey, contractId: 'crm_lookup_result', baseUrl, disposition: 'fail' });

const dryRun = process.env.GATE_DRY_RUN === '1';

export async function lookupCustomer(call) {
  const checked = await argsGate.validateOnly(call, { dryRun });
  if (!checked.ok) {
    return { error: 'rejected', violations: checked.violations.map((v) => v.message) };
  }
  const raw = await crmLookup(checked.payload); // only gated arguments reach the tool
  const shaped = await resultGate.validateOnly(raw, { dryRun });
  if (!shaped.ok) {
    return { error: 'tool result rejected', violations: shaped.violations.map((v) => v.message) };
  }
  return shaped.payload; // masked and stripped; the agent never sees `raw`
}

async function crmLookup({ customer_id }) { // stand-in for the real tool
  return { customer_id, plan: 'pro', email: 'ada@example.com', ssn: '123-45-6789', internal_notes: 'vip' };
}

console.log(await lookupCustomer({ operation: 'get_customer', customer_id: 'cus_123456' }));
console.log(await lookupCustomer({ operation: 'delete_customer', customer_id: 'cus_123456' }));
```

### MCP server

The MCP server exposes `egress_validate` (and `validate_events`, `infer_contract`, `deploy_contract`, `get_quarantine`, `list_contracts`, `get_drop_counts`). An agent host calls it like any other tool:

```json
{
  "name": "egress_validate",
  "arguments": {
    "contract_id": "crm_lookup_args-or-its-uuid",
    "events": [{"operation": "delete_customer", "customer_id": "cus_123456"}],
    "disposition": "fail",
    "dry_run": true
  }
}
```

Setup is in [mcp-reference.md](../mcp-reference.md). This server is a tool the agent *can* call. It does not sit in front of your other MCP servers, so a model that skips the call skips the check. Put the check in the tool's own code (the Python and TypeScript examples above) or in your agent host, where the model cannot choose to bypass it.

## Keep evidence of what was allowed

Send the agent run id as `X-Batch-Id` on every gated call:

```bash
curl -s -X POST "https://api.datacontractgate.com/egress/$ARGS_ID?disposition=fail" \
  -H "x-api-key: $CONTRACTGATE_API_KEY" -H "X-Batch-Id: $RUN_ID" \
  -H "content-type: application/json" -d '{"operation":"get_customer","customer_id":"cus_123456"}'
```

At the end of the run, ask `GET /quarantine/summary` how much was held back:

```bash
curl -s "https://api.datacontractgate.com/quarantine/summary?contract_id=$ARGS_ID&batch_id=$RUN_ID" \
  -H "x-api-key: $CONTRACTGATE_API_KEY"
```

Alert when `pending` is above zero. Dry runs store nothing, so they leave no evidence; run live in production. Each stored violation carries the contract version and the per-field reason. How long records are kept depends on your plan, see [aischemagate-plans-reference.md](../aischemagate-plans-reference.md). Details: [quarantine-batch-reference.md](../quarantine-batch-reference.md).

## What DataContractGate is not

Said plainly, so you can place it correctly in your stack.

- **Not a post-load check.** It enforces before data lands or before an action runs: on the way in, between stages and on the way out. Tools that test tables after they are loaded (dbt tests, Great Expectations, Soda) stay useful, and the two are complementary. See [where-to-enforce-contracts.md](../architecture/where-to-enforce-contracts.md).
- **Not a data catalog.** It does not discover datasets, track ownership or hold a glossary. It holds contracts and the quarantine of records that broke them. The public catalog is a set of ready-made contracts for open-data sources you can fork, and an OpenMetadata integration publishes validation results into a catalog you already run.
- **Not an authorization system.** It does not know who the caller is or whether this user may run this tool. It checks values. Keep your identity and permission checks where they are.
- **Not an MCP gateway or proxy.** It does not sit in front of other MCP servers. An integration has to call it, and has to forward only the returned `payload`.
- **Not a check on intent.** A well-formed, in-range call can still be a bad idea, and a prompt injection that produces a valid object passes. The gate checks shape and declared constraints. It does not judge whether a valid-looking value is true, and it does not detect malicious instructions.
- **Not a prompt or instruction store.** It never sees the agent's instructions.
- **Not an undo.** Rolling a contract back to an earlier version does not undo records already forwarded or actions already taken.
- **Not a transformation tool.** The only changes it makes to data are the declared `mask`, `hash`, `drop` and `redact` transforms and the stripping of undeclared fields.
- **Not an orchestrator.** It does not run or schedule your agent. Your orchestrator reads the quarantine summary and decides what to do.

## Limits today

- Non-streaming only. A streamed model response must be buffered first.
- `compliance_mode`, `transform` and `egress_leakage_mode` apply to top-level fields. Nested tool results need flat shapes or an extra check.
- Masks are opaque or format-preserving. There is no keep-last-four style.
- `synth --invalid` does not exercise transforms or leakage rules.

## See also

- [LangGraph tool-call gate](langgraph-tool-gate.md), [OpenAI, Anthropic, Pydantic, Instructor](openai-anthropic-structured-output.md), [Vercel AI SDK](vercel-ai-sdk.md)
- [LLM output validation playbook](../llm-output-validation.md), [LLMGate reference](../llm-sdk-reference.md)
- [Egress validation reference](../egress-validation-reference.md), [PII masking and leakage guard](../pii-masking-reference.md)
- [Synthetic data reference](../synthetic-data-reference.md), [`contractgate test` reference](../cg-test-reference.md)
