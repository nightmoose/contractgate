# Synthetic data reference

DataContractGate can generate test rows from a contract. Rows come only from the contract's rules. No stored events, quarantine rows or other real samples are read, and nothing is written. Design: [RFC-117](rfcs/117-synthetic-data.md).

## What the generator reads

| Contract rule | Valid rows | Invalid rows break it by |
|---|---|---|
| `type` | value of that type | a value of another type |
| `required: true` | always present | leaving the field out |
| `required: false` | present in about 3 of 4 rows | |
| `enum` | one of the listed values | a value of the right type outside the list |
| `min` / `max` | inside the range | a value below `min` or above `max` |
| `min_length` / `max_length` | length inside the bounds | a shorter or longer string |
| `pattern` | a string built from the regex | a string the regex does not match |
| `format: email`, `uuid`, `uri`, `url` | a value in that format | a string that is not in that format |
| `type: date`, `type: timestamp` | `YYYY-MM-DD`, RFC 3339 | a string that does not parse |
| nested `properties`, array `items` | generated recursively | (nested object fields: as above) |
| quality `completeness` | present, non-null, non-empty | absent, `null` or `""` |
| quality `freshness` | Unix seconds inside `max_age_seconds` of now | an older timestamp |
| quality `uniqueness` | no repeated value in the batch | repeating an earlier row's value |
| `metrics` with `field`, `min`, `max` | inside the metric range | a value outside it |
| `compliance_mode: true` | declared fields only | one extra undeclared field |

Cross-field rules (for example one date after another) are not part of the contract format yet, so they are not generated.

Every row is checked with the same validator ingest uses before it is returned. Valid rows pass it, with no batch uniqueness duplicates. Each invalid row fails exactly one rule, and its `broken_rule` label names that rule.

When a rule cannot be met, the call fails with the rule and field named. Examples: `min_length` above `max_length`, an `enum` with no value inside `min_length`, `uniqueness` on a boolean for more than 2 rows, or `freshness` on a string field.

In invalid mode, a rule that cannot be broken without breaking another one is left out and listed in `skipped_rules`. Example: leaving out a field that a metric also reads fails both the `required` rule and the metric.

The same seed, contract and DataContractGate version give the same rows. Freshness values are relative to the current time.

## API

`POST /contracts/synth` with an `x-api-key` header.

| Body field | Type | Notes |
|---|---|---|
| `yaml_content` | string | Contract YAML. Send this or `contract_id`. |
| `contract_id` | uuid | A contract in your org. |
| `version` | string | Version of `contract_id`. Latest stable when omitted. |
| `rows` | integer | 1 to 10000. Default 100. |
| `mode` | `valid` or `invalid` | Default `valid`. |
| `seed` | integer | Optional. A random seed is used and returned when omitted. |

```bash
curl -s https://api.datacontractgate.com/contracts/synth \
  -H "x-api-key: $CONTRACTGATE_API_KEY" -H "content-type: application/json" \
  -d '{"contract_id": "<uuid>", "rows": 3, "mode": "invalid", "seed": 42}'
```

Response:

```json
{
  "contract": {"name": "orders", "version": "1.0"},
  "source": {"type": "stored", "contract_id": "<uuid>", "version": "1.0"},
  "mode": "invalid",
  "seed": 42,
  "row_count": 3,
  "rows": [
    {
      "record": {"order_id": "ORD-861462", "amount": "not_a_number_20354"},
      "broken_rule": {
        "field": "amount",
        "rule": "type",
        "violation_kind": "type_mismatch",
        "message": "Field 'amount' expected type Float, got string"
      }
    }
  ],
  "skipped_rules": []
}
```

In `valid` mode each entry of `rows` is the record itself.

Errors:

| Status | `error` | When |
|---|---|---|
| 400 | `invalid_rows` | `rows` is 0 or above 10000 |
| 400 | | both or neither of `yaml_content` and `contract_id`, or the contract has no stable version and no `version` was sent |
| 404 | | `contract_id` or `version` is not in your org |
| 422 | `invalid_contract` | the YAML does not parse or compile |
| 422 | `unsatisfiable_rule` | a rule cannot be met. `rule` and `field` name it. |

## CLI

```
contractgate synth <CONTRACT> [--rows N] [--invalid] [--seed S] [--format json|csv] [-o FILE]
```

Runs locally on a contract YAML file. No API key or network is needed.

- `--format json` (default) prints a JSON array. With `--invalid`, each entry is `{record, broken_rule}`.
- `--format csv` flattens nested objects into dot-path columns and writes arrays as JSON text. With `--invalid`, the `_broken_field` and `_broken_rule` columns hold the label.
- Without `--seed`, the chosen seed is printed to stderr so the run can be repeated.

Exit codes: 0 rows written, 1 a rule cannot be met (the message names it), 2 the contract cannot be read or compiled.

```bash
contractgate synth contracts/orders.yaml --rows 500 --seed 42 > orders.json
contractgate synth contracts/orders.yaml --rows 50 --invalid --format csv -o bad_orders.csv
contractgate test --contract contracts/orders.yaml --data orders.json   # all pass
```
