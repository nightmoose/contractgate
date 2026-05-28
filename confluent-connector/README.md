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
| `contractgate.contract.version` | `""` | Pin to a specific version, e.g. `"1.2.0"`. Blank = latest stable. Sent as the `X-Contract-Version` request header (highest server precedence). |
| `contractgate.dry.run` | `false` | Validate without writing to the audit log (reduces DB pressure at high throughput) |
| `contractgate.on.failure` | `DLQ` | `DLQ` — throw DataException for DLQ routing. `TAG_AND_PASS` — add violation headers and pass through |
| `contractgate.connect.timeout.ms` | `5000` | TCP connection timeout in ms |
| `contractgate.request.timeout.ms` | `10000` | Total HTTP request/response timeout in ms |
| `contractgate.add.result.headers` | `true` | Stamp `contractgate.*` metadata headers on every record |
| `contractgate.max.violation.headers` | `5` | Max individual violation headers per record |

---

## Dynamic Contract Reload (RFC-064)

By default the SMT reads its contract reference once at task start. Enable hot-reload to pick up contract version changes without bouncing the Connect task.

### Enable

```properties
transforms.contractgate.contractgate.reload.enabled=true
transforms.contractgate.contractgate.reload.poll.ms=30000
transforms.contractgate.contractgate.reload.failure.action=warn
```

### How it works

A background thread polls `GET /v1/contracts/{id}/version` (a cheap endpoint added in RFC-064) every `poll.ms` milliseconds. On a hash change it fetches the new contract body to verify it is well-formed, then atomically swaps the cached version reference via an `AtomicReference`. The `apply()` hot path does a single volatile read per record — no lock contention with the reloader.

If the fetched contract is malformed or the gateway is unreachable, the old contract is kept and a failure counter is incremented.

### Config keys

| Key | Default | Description |
|-----|---------|-------------|
| `contractgate.reload.enabled` | `false` | Enable hot reload. Off by default — opt in to preserve existing behaviour. |
| `contractgate.reload.poll.ms` | `30000` | Polling interval in ms. Minimum 5000 (enforced at config parse). |
| `contractgate.reload.failure.action` | `warn` | `warn` — keep old contract and log (default). `fail-task` — throw `ConnectException` on the next `apply()` call, failing the Connect task. Use when stale-contract processing is unacceptable. |

### Metrics

The reloader exposes two `AtomicLong` counters accessible for monitoring:

| Counter | Description |
|---------|-------------|
| `contractgate.reload.success` | Incremented on each successful version swap |
| `contractgate.reload.failure` | Incremented on each failed reload attempt |

### Troubleshooting

- **Reload never fires** — verify `contractgate.reload.enabled=true` and check that the API key has read access to `GET /v1/contracts/{id}/version`.
- **Reload fires but SMT uses stale contract** — the gateway returned a contract body that started with `{` (JSON error response) or was otherwise invalid. Check `reload.failure` counter and gateway logs.
- **Task fails after reload error** — `contractgate.reload.failure.action=fail-task` is set. Change to `warn` if you want the SMT to stay up with the last good contract.

---

## Per-Violation DLQ Routing (RFC-064)

By default all failing records go to the single `errors.deadletterqueue.topic.name`. Enable per-violation routing to send records to different topics based on violation metadata.

### Enable

```properties
transforms.contractgate.contractgate.dlq.routing.enabled=true
transforms.contractgate.contractgate.dlq.routing.rules=[{"match":{"severity":"error","type":"pii_leak"},"topic":"audit.pii_failures"},{"match":{"severity":"error"},"topic":"dlq.errors"},{"match":{"severity":"warn"},"topic":"dlq.warnings"}]
transforms.contractgate.contractgate.dlq.routing.default=dlq.fallback
transforms.contractgate.contractgate.dlq.routing.producer.bootstrap.servers=broker1:9092,broker2:9092
```

### Rule evaluation

Rules are evaluated top-to-bottom; the **first match wins**. If no rule matches, the default topic is used. Each rule is a JSON object `{"match": {...}, "topic": "..."}`. All keys in `match` must match for the rule to fire (AND semantics).

**Available match fields:**

| Field | Values | Description |
|-------|--------|-------------|
| `severity` | `error`, `warn` | Hard violations (missing/type/pattern/enum/range/length/metric/pii_leak) map to `error`; `undeclared_field` and unknowns map to `warn` |
| `type` | e.g. `enum_violation` | Violation `kind` string from the gateway response |
| `field` | e.g. `amount` | Field path that violated |
| `contract` | contract UUID | For future multi-contract setups |

### Why a dedicated producer (not errantRecordReporter)?

Kafka Connect 3.6.0's `ErrantRecordReporter` interface routes errors to the single `errors.deadletterqueue.topic.name` configured on the connector. It does not support per-record topic override. Per-violation routing therefore uses a dedicated internal `KafkaProducer` — a well-established pattern used by Debezium, Lenses, and other production SMTs.

The `DataException` is still thrown after the routed send, so Connect's own error-handling (DLQ headers, context headers, dead-letter reporter) continues to fire normally.

### Config keys

| Key | Default | Description |
|-----|---------|-------------|
| `contractgate.dlq.routing.enabled` | `false` | Enable per-violation routing. Off by default. |
| `contractgate.dlq.routing.rules` | `[]` | JSON array of `{match, topic}` rule objects. Evaluated top-to-bottom. |
| `contractgate.dlq.routing.default` | *(none)* | Fallback DLQ topic. **Required** when `enabled=true`. |
| `contractgate.dlq.routing.producer.bootstrap.servers` | *(none)* | `bootstrap.servers` for the internal producer. **Required** when `enabled=true`. |
| `contractgate.dlq.routing.producer.*` | — | Any additional key under this prefix is passed through to the internal `KafkaProducer` (e.g. `contractgate.dlq.routing.producer.security.protocol=SSL`). |

### Example: route by violation severity and type

```properties
contractgate.dlq.routing.enabled=true
contractgate.dlq.routing.default=dlq.fallback
contractgate.dlq.routing.producer.bootstrap.servers=broker:9092
contractgate.dlq.routing.rules=[
  {"match":{"severity":"error","type":"pii_leak"},"topic":"audit.pii_failures"},
  {"match":{"severity":"error"},                  "topic":"dlq.errors"},
  {"match":{"severity":"warn"},                   "topic":"dlq.warnings"}
]
```

---

## Server-side version probe (RFC-064)

A new lightweight endpoint was added to the ContractGate gateway to support dynamic contract reload:

```
GET /v1/contracts/{contract_id}/version
Authorization: x-api-key <key>

200 OK
{"version":"2.1.0","hash":"a3f9..."}
```

The `hash` is a SHA-256 hex digest of the contract YAML content. The SMT compares hashes on each poll and only fetches the full contract body when the hash changes, keeping poll overhead minimal.

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
