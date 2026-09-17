# ContractGate + Redpanda Connect

Validate every JSON message with ContractGate before it lands on a clean topic.
Uses the built-in HTTP processor — no custom Go plugin.

## Pipeline

```
topic raw  →  POST /v1/ingest/{id}  →  topic clean       (failed == 0)
                                  ↘  topic quarantine  (else)
```

HTTP 422 is a **rejected event**, not an outage. The recipe lists it in
`successful_on` so Redpanda Connect parses the body and routes instead of retrying.

## Run

```bash
export CONTRACTGATE_API_KEY=cg_live_...
export CONTRACTGATE_CONTRACT_ID=11111111-1111-1111-1111-111111111111
export REDPANDA_BROKERS=localhost:9092

rpk connect run docs/examples/redpanda-connect/contractgate.yaml
```

Smoke test without Kafka:

```bash
rpk connect run docs/examples/redpanda-connect/generate.yaml
```

Hosted API: `https://contractgate-api.fly.dev`. Self-hosted: set `CONTRACTGATE_URL`.

## Community blurb (Redpanda Slack / forum)

> ContractGate is a semantic contract gateway in front of your topics.
> Drop this Redpanda Connect YAML in: every JSON record is POSTed to
> `/v1/ingest/{id}`; pass goes to `events.clean`, fail goes to
> `events.quarantine`. No Connect cluster, no Schema Registry.
> Recipe: https://github.com/nightmoose/contractgate/tree/main/docs/examples/redpanda-connect
