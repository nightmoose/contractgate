# `cg test` — Local Contract Dry-Run Reference

**RFC-076 | Added 2026-06-01**

Run every record in a local data file through the ContractGate validation engine
against a contract YAML. No server, no Kafka, no network required.

---

## Synopsis

```
cg test --contract <FILE> --data <FILE|-> [OPTIONS]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `--contract <FILE>` | *(required)* | Path to contract YAML file. |
| `--data <FILE>` | *(required)* | Input data: NDJSON file, JSON array file, or single JSON object file. Use `-` to read from stdin. |
| `--format human\|json` | `human` | Output format. `json` emits a machine-readable summary with per-record violation detail. |
| `--fail-fast` | off | Stop at the first failing record. Uniqueness batch checks are skipped in this mode. |
| `--quiet` | off | Print summary line only; suppress per-violation detail in human mode; suppress passing records in json mode. |

## Input formats

`--data` accepts three shapes, auto-detected:

- **NDJSON** — one JSON object per line (streams large files without loading all into memory).
- **JSON array** — `[{...}, {...}]`.
- **Single JSON object** — `{...}` (one record; fastest inner-loop iteration).

A malformed line in NDJSON is reported as a record-level error (with its index)
rather than aborting — the remaining records still run.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All records pass. |
| `1` | One or more records fail validation. |
| `2` | Load or parse error: contract won't compile, data file unreadable, or unparseable top-level JSON. |

Codes `0` and `1` mean the engine ran successfully. Code `2` means the tool
couldn't start — distinguish it from `1` in CI to tell "my data is bad" from
"my harness is broken."

## Human output (default)

```
contract: user_events (v1.0)
data:     samples.ndjson  (1000 records)

  PASS  994
  FAIL    6

  record   12  event_type           enum_violation       value "delete" not in allowed set
  record   12  timestamp            range_violation      value -1 is below minimum 0
  record   88  user_id              pattern_mismatch     value "Bad ID!" does not match required pattern
  ...

6/1000 records failed (0.6%)   validated in 1.2ms
```

## JSON output (`--format json`)

```json
{
  "contract": "user_events",
  "data_source": "samples.ndjson",
  "total": 1000,
  "pass": 994,
  "fail": 6,
  "elapsed_ms": 1.2,
  "records": [
    {
      "record": 12,
      "status": "fail",
      "violations": [
        { "field": "event_type", "kind": "enum_violation", "message": "..." }
      ]
    }
  ]
}
```

Passing records are included by default; add `--quiet` to suppress them.

## Recipes

### Inner edit loop

```bash
# Edit the contract, re-run immediately:
cg test --contract contracts/events.yaml --data samples.ndjson
```

### Chain with `cg infer`

```bash
curl "https://api.example.com/events" \
  | cg infer --from-stdin --name events \
  | cg test --contract contracts/events.yaml --data -
```

### CI assertion

```bash
cg test --contract contracts/events.yaml --data tests/fixtures/events.ndjson \
  --format json --quiet
# exit 1 fails the build; exit 2 flags a harness problem
```

### Fail fast during development

```bash
cg test --contract c.yaml --data big_sample.ndjson --fail-fast
```

## What `cg test` does NOT do

- No remote contract lookup (`id:uuid` is not supported; pass a local YAML path).
- No metric formula evaluation — `validate_metric` checks min/max envelope only;
  formula metrics are skipped (the same way the hot-path engine skips them today).
  The dev loop reports exactly what production enforces.
- No Kafka, no server, no `--features scaffold` required.

## See also

- [`cg validate`](./cli-reference.md) — check whether a contract YAML is well-formed.
- [`cg enforce`](./cli-reference.md) — shadow-enforce against live Kafka traffic.
- [`cg infer`](./cli-reference.md) — derive a contract from a JSON response or Newman export.
