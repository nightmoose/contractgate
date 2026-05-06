# cg scaffold — Reference

Generate a draft ContractGate contract from a live Kafka topic or a local
schema file, then validate events against it in shadow mode before promoting.

---

## Overview

`cg scaffold` samples events from a source, profiles field statistics, detects
PII candidates, and emits a draft CG YAML contract ready for human review.

`cg enforce --mode shadow` replays sampled events through the existing
validation engine against a draft contract and reports violations — without
touching the live ingest path.

The scaffolder is **read-only**. It never commits Kafka offsets, never writes
to a topic, and never auto-applies PII transforms. Emitted contracts are
always drafts until a human promotes them.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `cg` CLI ≥ MVP release | Build with `cargo build --features scaffold` |
| Kafka broker access | PLAINTEXT or SASL/PLAIN for MVP; mTLS / SCRAM in Phase 2 |
| Schema Registry (optional) | Required for Avro/Protobuf schema-driven inference |
| `$CG_KAFKA_BROKER` env var | Default broker address; overridden with `--broker` |
| `$CG_SR_URL` env var | Default Schema Registry URL; overridden with `--schema-registry` |

Credentials are read from environment variables or `~/.contractgate/credentials.toml`
(mode `0600`). They are never written into the emitted contract YAML.

---

## Scaffold from a file

The fastest way to get started — no Kafka required.

```bash
# JSON array or NDJSON
cg scaffold --from-file samples.json --output contracts/orders.yaml

# Avro schema
cg scaffold --from-file schema.avsc --output contracts/orders.yaml

# Protobuf schema
cg scaffold --from-file events.proto --output contracts/orders.yaml
```

Supported file extensions: `.json`, `.ndjson`, `.avsc`, `.proto`.

---

## Scaffold from a live Kafka topic

```bash
cg scaffold orders \
  --broker kafka:9092 \
  --schema-registry http://sr:8081 \
  --records 5000 \
  --output contracts/orders.yaml
```

Sample output:

```
Sampling topic 'orders' … 5000 records in 12.3s
Detected format: Avro (SR schema id=42)
Fields: 9   PII candidates: 2 (confidence ≥ 0.4)
Wrote contracts/orders.yaml
Exit: 0 (review PII annotations before promoting)
```

The CLI creates an ephemeral consumer group (`contractgate-scaffold-<uuid4>`)
that is never committed and expires automatically. It will not affect your
existing consumer offsets.

By default sampling starts at `high_watermark - N_records` per partition
(i.e. recent events). Use `--from-earliest` to start at the beginning of
the topic.

---

## CLI reference

### `cg scaffold`

```
cg scaffold <topic>
    --broker <host:port>            (default: $CG_KAFKA_BROKER)
    --schema-registry <url>         (default: $CG_SR_URL; omit for SR-less mode)
    --auth <plaintext|sasl-plain>   (default: plaintext; mTLS/SCRAM in Phase 2)
    --records <N>                   (default: 1000)
    --wall-clock <seconds>          (default: 30)
    --output <file.yaml>            (default: stdout)
    --contract-id <uuid>            (merge mode — requires existing contract)
    --dry-run                       (print merge diff; write no files)
    --from-earliest                 (start from topic beginning instead of tail)

cg scaffold --from-file <path>
    [--output, --dry-run same as above]
```

Sampling stops at whichever limit fires first: `--records` or `--wall-clock`.

`--contract-id` enables **merge mode**: the new scaffold is merged with the
existing contract version, preserving human edits. See [Merge semantics](#merge-semantics) below.

### `cg enforce --mode shadow`

```
cg enforce --mode shadow
    --contract <file.yaml | id:uuid>
    --broker <host:port>
    --topic <topic>
    --records <N>
    --wall-clock <T>
    --report <markdown|json|prometheus>   (default: markdown)
    --output <file | stdout>
    --dry-run                             (no Prometheus push; print only)
```

Shadow enforcement runs as a separate process. It calls the same `validate()`
function used by the live ingest path — identical logic, zero changes to hot
path code.

---

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success — no violations (shadow) or clean scaffold |
| 1 | Violations found in shadow mode (report written to `--output`) |
| 2 | Fatal error — connection failure, parse error, or auth failure |
| 3 | Schema drift detected during re-scaffold merge |
| 4 | PII candidates detected (non-fatal; suppress with `--no-pii-exit`) |

---

## Reading the emitted contract

`cg scaffold` emits a standard CG YAML contract. Fields are inferred from the
sampled events. PII candidates are flagged with `# TODO` annotations — they
are **never** emitted as live YAML `transform` blocks, regardless of
confidence.

Example output:

```yaml
version: "1.0"
name: "orders"
description: "Scaffolded from topic 'orders' — 5000 samples, 2026-05-06"

ontology:
  entities:
    - name: order_id
      type: string
      required: true

    # scaffold: pii-candidate confidence=0.45 reason="field_name:email"
    # TODO: review PII — consider transform.kind: hash or mask
    # transform:
    #   kind: hash
    #   salt_env: CG_SALT_ORDERS
    - name: customer_email
      type: string
      required: false

    - name: amount
      type: number
      required: true
      min: 0

glossary:
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative"
```

To apply a PII transform, uncomment the `transform` block and fill in the
salt env var, then promote the contract through your normal review process.

---

## PII confidence scores

Confidence is a combined score from two signals:

| Signal | Weight | Method |
|--------|--------|--------|
| Field name | 0.6 | Match against curated PII name list (`email`, `ssn`, `phone`, `password`, `creditcard`, etc.) |
| Sampled values | 0.4 | Regex match against non-null string values (≥ 10% hit rate required) |

The emit threshold is **confidence ≥ 0.4**. Common confidence levels:

| Example field | Confidence | Reason |
|---------------|------------|--------|
| `email` | 1.0 | Exact token in high-confidence list |
| `userEmail` | 1.0 | Token match on `email` |
| `phone_number` | 0.45 | Token match on `phone` (name signal only) |
| `contact` with email values | 0.40 | Value regex only (no name signal) |
| `amount` | 0.0 | No match |

A confidence score is a suggestion, not a verdict. Always review before
promoting.

---

## Merge semantics

When you run `cg scaffold --contract-id <uuid>`, the new scaffold output is
**merged** with the existing contract version at field granularity:

| Situation | Result |
|-----------|--------|
| You edited a field; schema unchanged | Your edit is preserved |
| Schema changed; you didn't touch the field | Scaffold update is accepted |
| Both you and the schema changed the field | **Conflict** — your version kept, annotated with `# scaffold: conflict` |
| Field disappeared from topic | Warning printed; your field is preserved (exit code 3) |
| New field appeared in topic | New field added from scaffold |

Conflicts are never silently resolved. The `# scaffold: conflict` annotation
marks fields that need human review before promotion.

Use `--dry-run` to preview the merge diff without writing any files.

---

## Schema Registry unavailability

If SR is unreachable, Avro and Protobuf fall back to raw-byte JSON inference.
The emitted contract is annotated:

```yaml
# scaffold: sr-unavailable; schema-driven inference skipped
# Quality warning: field types inferred from raw bytes only — review carefully
```

Pass `--require-sr` to abort instead of degrading. Pass `--accept-sr-fallback`
to suppress the exit code 2 when SR is down and only one `_raw` bytes field
can be inferred.

---

## Credentials file

`~/.contractgate/credentials.toml` (must be mode `0600`):

```toml
[kafka]
broker = "kafka.internal:9092"
security_protocol = "SASL_PLAINTEXT"
sasl_mechanism = "PLAIN"
sasl_username = "cg-reader"
sasl_password = "secret"

[schema_registry]
url = "http://sr.internal:8081"
basic_auth_user_info = "cg-reader:secret"
```

Environment variables take precedence over the credentials file.
Credentials are never written into emitted contract YAML or test fixtures.

---

## What's next

- Promote a scaffolded contract: `POST /contracts/{id}/versions/1.0.0/promote`
- Add metric formulas to the `metrics:` block manually before promoting
- Run shadow enforcement before going live: `cg enforce --mode shadow`
- Full ingest reference: [v1-ingest-reference.md](v1-ingest-reference.md)
- Quick start: [quickstart-5min.md](quickstart-5min.md)
- RFC-024 design decisions: [rfcs/024-brownfield-scaffolder.md](rfcs/024-brownfield-scaffolder.md)
