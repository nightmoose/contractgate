# Kafka Ingress Reference

**Status:** Available (RFC-025)

ContractGate can consume events directly from Confluent Cloud on your behalf.
You produce to an input topic; ContractGate validates each message against your
contract and routes it to either a clean output topic or a quarantine topic —
no consumer code required on your side.

---

## Enabling Kafka Ingress

1. Open a contract in the dashboard.
2. Click the **Kafka** tab.
3. Toggle **Enable Kafka Ingress**.

On enable, ContractGate:

- Provisions three Confluent Cloud topics (see [Topic Names](#topic-names)).
- Creates a scoped Confluent API key with produce-only access to the input topic
  and consume access to the output topics.
- Returns the bootstrap server address and credentials. **Copy the password now
  — it is shown only once.**

---

## Topic Names

| Purpose              | Topic                            |
|----------------------|----------------------------------|
| Your producers write | `cg-{contract_id}-raw`           |
| Valid events appear  | `cg-{contract_id}-clean`         |
| Invalid events appear| `cg-{contract_id}-quarantine`    |

`contract_id` is the UUID shown in the contract detail page URL.

---

## Connection Settings

| Setting              | Value                                         |
|----------------------|-----------------------------------------------|
| Security protocol    | `SASL_SSL`                                    |
| SASL mechanism       | `PLAIN`                                       |
| Bootstrap servers    | Shown in dashboard after enabling             |
| Username             | Shown in dashboard (Confluent API key ID)     |
| Password             | Shown **once** on enable (Confluent API secret) |

---

## Quick Start (Python)

```python
from confluent_kafka import Producer

p = Producer({
    "bootstrap.servers": "<bootstrap-servers>",
    "security.protocol": "SASL_SSL",
    "sasl.mechanisms": "PLAIN",
    "sasl.username": "<api-key>",
    "sasl.password": "<api-secret>",
})

event = '{"user_id": "u1", "event_type": "click", "timestamp": 1746000000}'
p.produce("cg-<contract_id>-raw", value=event)
p.flush()
```

---

## Validation & Routing

ContractGate runs the same validation engine used by the HTTP ingest path
(`POST /ingest/{contract_id}`). Events are validated against the **latest
stable version** of the contract.

| Outcome   | Destination topic           | Headers added                      |
|-----------|-----------------------------|------------------------------------|
| Valid     | `cg-{contract_id}-clean`    | `cg-contract-version`              |
| Invalid   | `cg-{contract_id}-quarantine` | `cg-contract-version`, `cg-violation-reason` |

The original payload is always preserved in the message value — invalid events
are never mutated or dropped.

Invalid events also appear in the **Quarantine** tab of the dashboard and can
be replayed after fixing the contract.

---

## Audit Log

Every event processed via Kafka is written to the audit log with
`source = 'kafka'`. The `contract_version` field always reflects the version
that produced the validation decision (audit honesty rule).

---

## Credential Rotation

1. In the **Kafka** tab, disable ingress (credentials revoked immediately).
2. Re-enable to receive a new API key and secret.

The consumer pool picks up the new credentials within one poll cycle.

---

## Disabling Ingress

Toggle the switch off in the **Kafka** tab. On disable:

- Confluent credentials are revoked immediately.
- Topics remain live for **24 hours** (drain window) so in-flight messages
  are not lost.
- Topics are deleted after the drain window elapses.

---

## Limitations

- One active ingress configuration per contract.
- Schema Registry (Avro/Protobuf) is not yet supported — send JSON payloads.
- The clean topic does not support header-only (skip) mode — routing is always full.
- Self-hosted Kafka brokers are not yet supported.

---

## Environment Variables (self-hosted / operators)

| Variable                    | Required | Description                                      |
|-----------------------------|----------|--------------------------------------------------|
| `CONFLUENT_CLOUD_API_KEY`   | Yes      | Confluent Cloud management API key               |
| `CONFLUENT_CLOUD_API_SECRET`| Yes      | Confluent Cloud management API secret            |
| `CONFLUENT_ENVIRONMENT_ID`  | Yes      | Confluent environment ID (e.g. `env-abc123`)     |
| `CONFLUENT_CLUSTER_ID`      | Yes      | Kafka cluster ID (e.g. `lkc-abc123`)             |
| `CONFLUENT_BOOTSTRAP_SERVERS`| Yes     | Bootstrap address shown to users                 |
| `ENCRYPTION_KEY`            | Yes      | 32-byte hex key for AES-256-GCM secret storage   |
