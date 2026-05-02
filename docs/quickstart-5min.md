# Get started in 5 minutes

Validate your first event against a contract using only `curl`.

---

## Step 1 — Create a contract

```bash
export BASE=https://contractgate.io
export KEY=cg_live_<your_api_key>

curl -s -X POST "$BASE/contracts" \
  -H "X-Api-Key: $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "user_events",
    "yaml_content": "version: \"1.0\"\nname: user_events\ndescription: Demo contract\n\nontology:\n  entities:\n    - name: user_id\n      type: string\n      required: true\n    - name: event_type\n      type: string\n      required: true\n      enum: [\"click\", \"view\", \"purchase\", \"login\"]\n    - name: timestamp\n      type: integer\n      required: true\n    - name: amount\n      type: number\n      required: false\n      min: 0\n"
  }' | jq '{id: .id, name: .name}'
```

Save the `id` from the response:

```bash
export CONTRACT_ID=<paste id here>
```

---

## Step 2 — Publish a stable version

The contract starts as a draft. Promote it to `stable` so you can ingest
against it:

```bash
curl -s -X POST "$BASE/contracts/$CONTRACT_ID/versions/1.0.0/promote" \
  -H "X-Api-Key: $KEY" | jq '{version: .version, state: .state}'
```

---

## Step 3 — Post a valid event

```bash
curl -s -X POST "$BASE/v1/ingest/$CONTRACT_ID" \
  -H "X-Api-Key: $KEY" \
  -H "Content-Type: application/json" \
  -d '[{
    "user_id": "u_demo",
    "event_type": "purchase",
    "timestamp": 1714000000,
    "amount": 49.99
  }]' | jq '{passed: .passed, total: .total}'
```

Expected response:
```json
{ "passed": 1, "total": 1 }
```

---

## Step 4 — Post an invalid event

```bash
curl -s -X POST "$BASE/v1/ingest/$CONTRACT_ID" \
  -H "X-Api-Key: $KEY" \
  -H "Content-Type: application/json" \
  -d '[{
    "event_type": "unknown_type",
    "timestamp": 1714000001
  }]' | jq '{failed: .failed, violations: .results[0].violations}'
```

The missing `user_id` and invalid `event_type` will produce violation details.
The event is automatically quarantined — the `quarantine_id` in the response
lets you replay it after fixing the data.

---

## Step 5 — Send a batch as NDJSON

```bash
printf '{"user_id":"u1","event_type":"login","timestamp":1714000100}\n{"user_id":"u2","event_type":"view","timestamp":1714000101}\n' \
  | curl -s -X POST "$BASE/v1/ingest/$CONTRACT_ID" \
    -H "X-Api-Key: $KEY" \
    -H "Content-Type: application/x-ndjson" \
    --data-binary @- \
  | jq '{total: .total, passed: .passed}'
```

---

## What's next

- Pin a contract version: `?version=1.0.0`
- Dry-run (validate without persisting): `?dry_run=true`
- All-or-nothing batch: `?atomic=true`
- Deduplicate with idempotency: add `Idempotency-Key: <uuid>` header
- Replay quarantined events: `POST /contracts/{id}/quarantine/replay`
- Full endpoint reference: [docs/v1-ingest-reference.md](v1-ingest-reference.md)
- OpenAPI spec: `GET /openapi.json`
