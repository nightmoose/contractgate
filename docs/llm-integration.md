# ContractGate — Integration Playbook for Coding Agents

**Read this file top to bottom, then execute it.** It is written for an AI
coding agent (Claude, Cursor, Codex, Copilot) working inside a user's
repository. Every step is concrete; no step requires the user to read
documentation.

- **Product:** ContractGate — semantic data-contract enforcement at ingestion.
  Rejects bad events *before* they reach a warehouse, lakehouse, or ML pipeline.
- **Repo:** <https://github.com/nightmoose/contractgate>
- **API base URL:** `https://app.datacontractgate.com`
- **Machine index:** <https://app.datacontractgate.com/llms.txt>

**What you are going to build:** a contract YAML committed to the user's repo,
deployed as a `stable` version, and their event producer calling the validation
gateway — verified with a dry run before anything writes.

---

## §0 — Preconditions

1. The user needs an API key. If they don't have one:
   sign in at <https://app.datacontractgate.com>, then
   **Account → API keys → Create key**. Keys look like `cg_live_` + 48 hex chars.
2. The key must be in the environment, never in the repo:

   ```bash
   export CONTRACTGATE_API_KEY="cg_live_..."
   ```

**Rules you must follow:**

- **Never** write the raw key into source, config, `.env` that is tracked, a
  commit message, or a chat message. Read it from `CONTRACTGATE_API_KEY`.
- If `CONTRACTGATE_API_KEY` is unset, stop and ask the user for it. Do not
  invent a key, and do not fall back to the self-hosted path silently.
- Add `.env` / `.env.local` to `.gitignore` if the user's project stores it there.

Every request below sends the key as the `X-Api-Key` header.

---

## §1 — Find the event shape in the user's repo

You need 5–20 **real** sample events. Do not invent them. Search the repo for
where structured events are produced or written:

| Look for | Typical location |
|---|---|
| `JSON.stringify(` / `json.dumps(` / `serde_json::to_string` | producer / client code |
| Kafka `produce(`, `send(`, `KafkaProducer` | streaming publishers |
| `INSERT INTO` / warehouse client `insert_rows` / `copy_from` | warehouse loaders |
| Webhook or API route handlers that accept a payload | `routes/`, `api/`, `handlers/` |
| Existing fixtures: `*.json`, `*.ndjson`, `fixtures/`, `testdata/` | tests |

Prefer real fixtures or logged payloads over hand-written examples — inference
quality is bounded by sample quality. Record, for later steps:

- the **event name** (snake_case, e.g. `user_events`, `property_listings`);
- the **file and line** where the event is sent;
- whether events go out **one at a time** or **in batches**.

---

## §2 — Infer a draft contract from the samples

```bash
curl -sS -X POST "https://app.datacontractgate.com/contracts/infer" \
  -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "user_events",
    "description": "Contract for user interaction events",
    "samples": [
      { "user_id": "u_123", "event_type": "purchase", "timestamp": 1714000000, "amount": 49.99 },
      { "user_id": "u_456", "event_type": "login",    "timestamp": 1714000001 }
    ]
  }'
```

Request body: `{ name, description?, samples: [ ... ] }` — every sample must be a
JSON **object**. Response:

```json
{ "yaml_content": "version: \"1.0\"\nname: user_events\n...", "field_count": 4, "sample_count": 2 }
```

Body cap on inference routes is **10 MB**. Other source formats, same auth and
response shape:

| Route | Input |
|---|---|
| `POST /contracts/infer` | JSON sample events |
| `POST /contracts/infer/csv` | CSV (header row + rows) |
| `POST /contracts/infer/url` | a public data URL to fetch and profile |
| `POST /contracts/infer/avro` | Avro schema |
| `POST /contracts/infer/proto` | Protobuf schema |
| `POST /contracts/infer/openapi` | OpenAPI spec |

Inference is a **starting point, not the answer.** Continue to §3.

---

## §3 — Write the contract file

Write `contracts/<name>.yaml` in the user's repo. Use exactly this structure —
this format is locked; do not introduce keys that are not listed in the field
reference below.

```yaml
version: "1.0"
name: "user_events"
description: "Contract for user interaction events"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative"

metrics:
  - name: total_revenue
    formula: "sum(amount) where event_type = 'purchase'"
```

### Field reference (`ontology.entities[]`)

| Key | Applies to | Notes |
|---|---|---|
| `name` | all | Field name exactly as it appears in the JSON event. |
| `type` | all | One of `string`, `integer`, `number` (alias for `float`), `boolean`, `object`, `array`, `date`, `any`. `date` = `YYYY-MM-DD` string, real calendar date. Use `any` sparingly — it weakens the contract. |
| `required` | all | **Defaults to `true` when omitted.** Write it explicitly on every field so intent is visible in review. |
| `pattern` | `string` | Regex the value must match. |
| `enum` | `string`, `integer` | Allowed value set. |
| `min` / `max` | `integer`, `number` | Inclusive numeric bounds. |
| `min_length` / `max_length` | `string` | Length bounds. |
| `properties` | `object` | Nested list of field definitions, same shape. |
| `items` | `array` | Element constraints — a single field definition. |
| `transform` | `string` only | PII transform applied *after* validation (`kind: mask`/`hash`, optional `style`). A transform on a non-string field is a load-time error. |

`glossary[]` takes `field`, `description`, and optional `constraints` —
documentation only, not enforced. `metrics[]` and `quality[]` are optional;
leave them out rather than guessing.

### Tighten the inference before committing

Do this pass explicitly — it is where the contract stops being a schema and
starts being a contract:

1. **Enums:** if a field has a small closed set of values in the samples and the
   repo confirms it (an enum type, a constant list, a DB check constraint),
   declare `enum`. If the samples merely happen to show 3 values, do not.
2. **Patterns:** add `pattern` for IDs and codes with an obvious shape.
3. **Required:** a field absent from any single sample must be
   `required: false`.
4. **Bounds:** add `min: 0` to amounts, counts, and durations.
5. **Types:** prefer `integer` over `number` for epoch timestamps and counts.

Optional top-level keys you may set when the user asks for them, not by default:
`compliance_mode: true` (reject events containing undeclared fields).

---

## §4 — Deploy the contract as a stable version

```
POST /contracts/deploy
X-Api-Key: cg_live_…
Content-Type: application/json
```

Build the JSON body from the file — never hand-escape YAML into a shell literal:

```bash
jq -n \
  --arg name "user_events" \
  --rawfile yaml "contracts/user_events.yaml" \
  '{name: $name, yaml_content: $yaml, source: "app-backend", deployed_by: "claude-code"}' \
| curl -sS -X POST "https://app.datacontractgate.com/contracts/deploy" \
    -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
    -H "Content-Type: application/json" \
    --data-binary @-
```

(No `jq`? Any equivalent works — the only requirement is that `yaml_content` is
the file's contents as a JSON string.)

Request: `{ name, yaml_content, source?, deployed_by? }`. Response:

```json
{
  "contract_id": "…uuid…",
  "version_id": "…uuid…",
  "name": "user_events",
  "version": "1.0",
  "source": "app-backend",
  "deployed_by": "claude-code",
  "deployed_at": "2026-08-06T12:00:00Z",
  "deprecated_count": 0
}
```

What this does: finds or creates the contract identity by `name`, inserts the
version from the YAML as **`stable`**, and deprecates any previously stable
version (`deprecated_count`).

- **`409 Conflict`** — that `(name, version)` already exists. Bump `version:` in
  the YAML (e.g. `"1.1"`) and re-deploy. Never edit a deployed version in place.
- Deploy is refused while the contract has pending quarantined events.

**Save `contract_id`.** It is the only value from this step you need next.
Put it in the user's config or environment (e.g. `CONTRACTGATE_CONTRACT_ID`) —
it is not a secret.

CLI alternative, if the user already has the binary
(`cargo install --git https://github.com/nightmoose/contractgate contractgate`):

```bash
contractgate deploy-contract contracts/user_events.yaml \
  --source app-backend --deployed-by "$USER" --json
```

---

## §5 — Wire the producer to the gateway

```
POST /v1/ingest/{contract_id}
X-Api-Key: cg_live_…
Content-Type: application/json          # or application/x-ndjson
Idempotency-Key: <opaque string>        # optional, 24-hour at-most-once window
```

Body: a JSON **array** of event objects, a **single** object (treated as a
1-event batch), or NDJSON (one object per line). Query parameters:

| Param | Default | Meaning |
|---|---|---|
| `version` | latest `stable` | Pin a specific contract version. |
| `dry_run` | `false` | Validate only — no audit row, no quarantine, no forward, no metered usage. |
| `atomic` | `false` | All-or-nothing: if any event fails, nothing is persisted. |

Response body (`V1IngestResponse`):

```json
{
  "total": 2, "passed": 1, "failed": 1,
  "dry_run": false, "atomic": false,
  "resolved_version": "1.0",
  "version_pin_source": "default_stable",
  "results": [
    { "index": 0, "passed": true,  "violations": [], "validation_us": 31,
      "forwarded": true, "contract_version": "1.0", "quarantine_id": null,
      "transformed_event": { "…": "…" } },
    { "index": 1, "passed": false,
      "violations": [ { "field": "amount", "kind": "…", "message": "…" } ],
      "validation_us": 27, "forwarded": false, "contract_version": "1.0",
      "quarantine_id": "…uuid…", "transformed_event": { "…": "…" } }
  ]
}
```

**HTTP status encodes the batch outcome:** `200` all passed · `207 Multi-Status`
partial · `422 Unprocessable Entity` all failed. Treat `207` and `422` as
"contract violations", not as transport errors — read `results[].violations`.

### TypeScript

```ts
const CG_URL = "https://app.datacontractgate.com";

export async function validateAndSend(events: unknown[]) {
  const res = await fetch(
    `${CG_URL}/v1/ingest/${process.env.CONTRACTGATE_CONTRACT_ID}`,
    {
      method: "POST",
      headers: {
        "X-Api-Key": process.env.CONTRACTGATE_API_KEY!,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(events),
    },
  );

  // 200 = all passed, 207 = partial, 422 = all failed.
  const body = await res.json();
  if (body.failed > 0) {
    for (const r of body.results.filter((r: any) => !r.passed)) {
      console.error("contract violation", r.index, r.violations);
    }
  }
  return body;
}
```

### Python (first-party SDK)

```bash
pip install contractgate
```

```python
import os
from contractgate import Client

cg = Client(
    base_url="https://app.datacontractgate.com",
    api_key=os.environ["CONTRACTGATE_API_KEY"],
)

result = cg.ingest(contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"], events=events)
for r in result.results:
    if not r.passed:
        for v in r.violations:
            print(v.field, v.kind, v.message)
```

The SDK also ships a pure-Python local validator (`Contract.from_yaml(...)`) for
unit tests and pre-commit hooks — no network required. Use it to gate CI.

### Where to put the call

Insert validation at the boundary where events **leave** the user's system —
immediately before the Kafka produce, warehouse insert, or outbound POST you
found in §1. Do not scatter it across call sites; wrap the existing send in one
function like the above and call that.

---

## §6 — Verify with a dry run (do this before removing `dry_run`)

Send one event you know is **good** and one you know is **bad** (wrong enum
value, missing required field, negative amount):

```bash
curl -sS -X POST \
  "https://app.datacontractgate.com/v1/ingest/$CONTRACTGATE_CONTRACT_ID?dry_run=true" \
  -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '[
    { "user_id": "u_1", "event_type": "purchase", "timestamp": 1714000000, "amount": 10 },
    { "user_id": "u_2", "event_type": "not_a_real_type", "timestamp": 1714000001 }
  ]'
```

Expect `207`, `passed: 1`, `failed: 1`, and a violation on `event_type` for
index 1. If **both** pass, your contract is too loose — go back to §3 and
tighten enums, patterns, and `required`. If **both** fail, read the violations:
usually a type mismatch (epoch seconds typed as `string`) or a `required` field
the samples didn't have.

Only when the dry run behaves correctly, remove `dry_run=true`.

---

## §7 — Confirm in the dashboard

At <https://app.datacontractgate.com> → **Contracts → your contract**:

- **Audit** — every validated event with its decision and contract version.
- **Quarantine** — rejected events, replayable after you fix the contract or the
  producer.
- **Usage** — metered event counts against the plan.

---

## §8 — Errors you will actually hit

| Status | Meaning | Fix |
|---|---|---|
| `401` | Missing or invalid `X-Api-Key`. | Check `CONTRACTGATE_API_KEY` is exported and not truncated. |
| `403` | Key is not authorized for this contract (keys can be scoped to a contract set). | Use a key scoped to this contract, or widen the key's scope in the dashboard. |
| `404` | Unknown `contract_id`, or it belongs to another org. | Re-read `contract_id` from the §4 response. |
| `409` | `(name, version)` already deployed. | Bump `version:` in the YAML. |
| `413` | Body too large. | 1 MB on most endpoints; 10 MB on `/contracts/infer/*` and `/v1/ingest/*`. Split the batch. |
| `422` | Every event in the batch failed validation. | Read `results[].violations` — this is a data or contract problem, not a transport problem. |
| `429` | Rate limited. | Back off and retry; batch events instead of sending one per request. |

---

## §9 — Do not

- **Do not invent contract fields** that are not in the samples or confirmed by
  the repo. A wrong contract rejects good data.
- **Do not "fix" failing validation by removing the gate**, widening a field to
  `any`, or deleting the offending constraint. Report the violations to the user
  and ask which side is wrong — the contract or the producer.
- **Do not commit API keys**, or interpolate one into a code sample, README, or
  chat message.
- **Do not use `POST /playground/validate`** for production wiring — that is the
  dashboard's scratchpad endpoint.
- **Do not edit a deployed version in place** — deploy a new `version:`.
- **Do not skip §6.** Dry-run first is the only reason a mistake here is
  reversible.

---

## Reference docs

| Doc | Covers |
|---|---|
[`docs/v1-ingest-reference.md`](https://github.com/nightmoose/contractgate/blob/main/docs/v1-ingest-reference.md) | Full `/v1/ingest` semantics: NDJSON, idempotency, atomic batches |
[`docs/deploy-contract-reference.md`](https://github.com/nightmoose/contractgate/blob/main/docs/deploy-contract-reference.md) | Deploy endpoint + CLI, version promotion rules |
[`docs/csv-inference-reference.md`](https://github.com/nightmoose/contractgate/blob/main/docs/csv-inference-reference.md) | CSV and URL inference |
[`docs/quarantine-replay-reference.md`](https://github.com/nightmoose/contractgate/blob/main/docs/quarantine-replay-reference.md) | Reviewing and replaying rejected events |
[`docs/pii-masking-reference.md`](https://github.com/nightmoose/contractgate/blob/main/docs/pii-masking-reference.md) | `transform:` field masking and hashing |
[`/openapi.json`](https://app.datacontractgate.com/openapi.json) | Machine-readable route inventory |

**Self-hosting instead?** `git clone https://github.com/nightmoose/contractgate
&& make demo` runs the real gateway locally at `http://localhost:3000` with no
account, no key, and no cloud dependency. The steps above still apply — swap the
base URL and drop the `X-Api-Key` header.
