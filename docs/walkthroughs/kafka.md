# Kafka Walkthrough

Validate a Kafka stream in place: producers write to a raw topic, ContractGate
validates each event with the same engine as the HTTP path, and routes valid
events to a clean topic and invalid ones to a quarantine topic. Full detail in
the [Kafka ingress reference](../kafka-ingress-reference.md).

## 1. The contract

The full runnable file is
[`examples/contracts/kafka/clickstream.yaml`](../../examples/contracts/kafka/clickstream.yaml).

```yaml
version: "1.0"
name: "clickstream"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "scroll", "login"]
    - name: timestamp
      type: integer
      required: true
quality:
  - field: timestamp
    type: freshness
    max_age_seconds: 86400
```

## 2. The command

Validate locally with [`cg test`](../cg-test-reference.md) before enabling the
stream:

```
cg test --contract examples/contracts/kafka/clickstream.yaml --data events.ndjson
```

Then enable Kafka ingress from the contract's **Kafka** tab in the dashboard.
This provisions three topics (`raw` → producers, `clean` → valid,
`quarantine` → invalid) and returns scoped credentials (copy the password — it
is shown once).

## 3. A passing record

A producer writes to `cg-{contract_id}-raw`:

```python
event = '{"user_id": "u1", "event_type": "click", "timestamp": <recent_epoch>}'
p.produce("cg-<contract_id>-raw", value=event)
```

Valid events land on `cg-{contract_id}-clean` with a `cg-contract-version`
header. Locally, `cg test` reports:

```
  PASS  1
1/1 records passed
```

## 4. A failing record

`event_type` is off the allowlist:

```json
{ "user_id": "u1", "event_type": "delete", "timestamp": 1714000000 }
```

```
  FAIL  1
record   0  event_type   enum_violation       Field 'event_type' value "delete" not in allowed set: [click, view, scroll, login]
record   0  timestamp    freshness_violation  Quality freshness: field 'timestamp' timestamp is 31536000s old (max 86400s)
```

In the stream, this event routes to `cg-{contract_id}-quarantine` with
`cg-contract-version` and `cg-violation-reason` headers — the original payload
is never mutated or dropped, and it appears in the dashboard Quarantine tab for
replay.

## 5. Wire it in

Producers keep writing to the raw topic; consumers read from the clean topic.
No application change beyond pointing at the provisioned topics:

```python
# producer → raw
p.produce("cg-<contract_id>-raw", value=event)

# consumer ← clean (only validated events)
c.subscribe(["cg-<contract_id>-clean"])
```

Gate the contract itself in CI with `cg test` before shipping a contract change.
See the [Kafka ingress reference](../kafka-ingress-reference.md) for connection
settings and credential rotation.
