# ContractGate Semantic Validator for Kafka Connect

A Kafka Connect **Single Message Transform (SMT)** that validates every record against a [ContractGate](https://datacontractgate.com) semantic contract in real-time — before it reaches storage or AI systems.

Invalid records are routed to a **dead-letter topic**. Passing records continue downstream unchanged. Zero schema registry dependency; works with any connector.

---

## Quick Start

Add two lines to any existing connector config:

```properties
transforms=contractgate
transforms.contractgate.type=io.datacontractgate.connect.smt.ContractGateValidator
transforms.contractgate.contractgate.api.url=https://contractgate-api.fly.dev
transforms.contractgate.contractgate.api.key=YOUR_API_KEY
transforms.contractgate.contractgate.contract.id=YOUR_CONTRACT_UUID

# Route failures to a dead-letter topic
errors.deadletterqueue.topic.name=your-topic.dlq
errors.deadletterqueue.context.headers.enable=true
```

That's it. Every record is now validated against your contract in real-time.

---

## Configuration Reference

| Key | Default | Description |
|-----|---------|-------------|
| `contractgate.api.url` | *(required)* | Base URL of your ContractGate API server (no trailing slash) |
| `contractgate.contract.id` | *(required)* | UUID of the contract to validate against |
| `contractgate.api.key` | `""` | `x-api-key` header value. Leave blank for dev/no-auth mode |
| `contractgate.contract.version` | `""` | Pin to a specific version, e.g. `"1.2.0"`. Blank = latest stable |
| `contractgate.dry.run` | `false` | Validate without writing to the audit log (reduces DB pressure at high throughput) |
| `contractgate.on.failure` | `DLQ` | `DLQ` — throw DataException for DLQ routing. `TAG_AND_PASS` — add violation headers and pass through |
| `contractgate.connect.timeout.ms` | `5000` | TCP connection timeout in ms |
| `contractgate.request.timeout.ms` | `10000` | Total HTTP request/response timeout in ms |
| `contractgate.add.result.headers` | `true` | Stamp `contractgate.*` metadata headers on every record |
| `contractgate.max.violation.headers` | `5` | Max individual violation headers per record |

---

## Result Headers

When `contractgate.add.result.headers=true`, the following headers are added to every record:

| Header | Value |
|--------|-------|
| `contractgate.passed` | `"true"` or `"false"` |
| `contractgate.contract.version` | Resolved version string, e.g. `"1.2.0"` |
| `contractgate.violations.count` | Number of violations (0 on pass) |
| `contractgate.violation.0.field` | Dot-separated field path, e.g. `"customer.address.country"` |
| `contractgate.violation.0.kind` | Machine-readable kind: `missing_required_field`, `type_mismatch`, `enum_violation`, etc. |
| `contractgate.violation.0.message` | Human-readable explanation |

---

## Failure Modes

### DLQ (default)
The SMT throws a `DataException`. Kafka Connect's built-in error handling routes the original record to `errors.deadletterqueue.topic.name`. Enable `errors.deadletterqueue.context.headers.enable=true` to surface the violation summary in the DLQ record's headers.

### TAG_AND_PASS
The SMT adds violation headers and forwards the record downstream unchanged. Consumers can inspect `contractgate.passed` and decide what to do.

### API Unavailable (fail-open)
If ContractGate is unreachable, the SMT logs a warning and passes the record through. This prevents a transient API outage from halting your connector. Tighten this by configuring Kafka Connect's task-level retry policies.

---

## Installation

### Confluent Hub CLI
```bash
confluent-hub install datacontractgate/kafka-connect-contractgate:0.1.0
```

### Manual
Extract the ZIP to your Connect plugin path:
```bash
unzip kafka-connect-contractgate-0.1.0.zip -d /usr/share/confluent-hub-components/
```
Then restart your Connect workers.

---

## Requirements

- Java 11+
- Kafka Connect 2.8+
- A running ContractGate API instance ([free tier available](https://datacontractgate.com/pricing))

---

## License

Apache License 2.0 — see [LICENSE](https://www.apache.org/licenses/LICENSE-2.0).

Documentation: [datacontractgate.com/docs/kafka-connect](https://datacontractgate.com/docs/kafka-connect)
