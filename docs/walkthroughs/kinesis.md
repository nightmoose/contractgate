# Kinesis Walkthrough

Validate an AWS Kinesis stream in place: producers write to a raw stream,
ContractGate validates each record with the same engine as the HTTP path, and
routes valid records to a clean stream and invalid ones to a quarantine stream.
Full detail in the [Kinesis ingress reference](../kinesis-ingress-reference.md).

> Kinesis ingress requires the gateway built with `--features kinesis-ingress`.
> Without it the routes return `501 Not Implemented`.

## 1. The contract

The full runnable file is
[`examples/contracts/kinesis/telemetry.yaml`](../../examples/contracts/kinesis/telemetry.yaml).

```yaml
version: "1.0"
name: "telemetry"
ontology:
  entities:
    - name: device_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: metric
      type: string
      required: true
      enum: ["cpu", "mem", "temp", "battery"]
    - name: value
      type: number
      required: true
      min: 0
    - name: timestamp
      type: integer
      required: true
quality:
  - field: timestamp
    type: freshness
    max_age_seconds: 3600
```

## 2. The command

Validate locally with [`cg test`](../cg-test-reference.md) before enabling the
stream:

```
cg test --contract examples/contracts/kinesis/telemetry.yaml --data samples.ndjson
```

Then enable Kinesis ingress via
`POST /contracts/{contract_id}/kinesis-ingress`. This provisions three streams
(`raw` → producers, `clean` → valid, `quarantine` → invalid) and a scoped IAM
user; the access key is returned in plaintext only on first enable or after a
rotation.

## 3. A passing record

A producer writes to `cg-{contract_id}-raw`:

```json
{ "device_id": "dev-01", "metric": "cpu", "value": 42.5, "timestamp": <recent_epoch> }
```

[`examples/contracts/kinesis/pass.json`](../../examples/contracts/kinesis/pass.json)
pins a fixed epoch for repeatable `cg test` runs — regenerate `timestamp` to
the current epoch before re-running; the 1-hour `freshness` window here is
tighter than Kafka's:

```json
{ "device_id": "dev-01", "metric": "cpu", "value": 42.5, "timestamp": 1784249150 }
```

Valid records land on `cg-{contract_id}-clean`. Locally, `cg test` reports:

```
  PASS  1
1/1 records passed
```

## 4. A failing record

`metric` is off the allowlist, `value` is negative, and the timestamp is stale
([`examples/contracts/kinesis/fail.json`](../../examples/contracts/kinesis/fail.json)):

```json
{ "device_id": "dev-01", "metric": "disk", "value": -3, "timestamp": 1714000000 }
```

```
  FAIL  1
record   0  metric     enum_violation       Field 'metric' value "disk" not in allowed set: ["cpu", "mem", "temp", "battery"]
record   0  value      range_violation      Field 'value' value -3 is below minimum 0
record   0  timestamp  freshness_violation  Quality freshness: field 'timestamp' timestamp is 70249301s old (max 3600s)
```

In the stream, this record routes to `cg-{contract_id}-quarantine` for
inspection or replay; the original payload is preserved.

## 5. Wire it in

Producers write to the raw stream; downstream consumers read from the clean
stream — only validated records reach them. No application logic change beyond
the stream names. Gate the contract in CI with `cg test` before shipping a
change. See the [Kinesis ingress reference](../kinesis-ingress-reference.md) for
IAM policy, environment variables, and credential rotation.
