# Synthetic data reference

DataContractGate can generate test rows from a contract. Rows come only from the contract's rules. No stored events, quarantine rows or other real samples are read, and nothing is written. Design: [RFC-117](rfcs/117-synthetic-data.md), [RFC-118](rfcs/118-synthetic-data-v1-1.md) (v1.1 and v1.2).

## What the generator reads

| Contract rule | Valid rows | Invalid rows break it by |
|---|---|---|
| `type` | value of that type | a value of another type |
| `required: true` | always present | leaving the field out |
| `required: false` | present in about 3 of 4 rows | |
| `enum` | one of the listed values | a value of the right type outside the list |
| `min` / `max` | inside the range | a value below `min` or above `max` |
| `distribution` (integer, number) | drawn from that shape, inside `min` / `max` (see [Value distributions](#value-distributions)) | as `min` / `max` above |
| `min_length` / `max_length` | length inside the bounds | a shorter or longer string |
| free-form `type: string` (no `pattern`, `format` or `enum`) | a realistic value guessed from the field name when it passes every declared rule (`email`, `first_name`, `customer_name`, `phone`, `website`, `country` / `country_code`, `city`, `address`, `zip`, `currency`, `*_id` as a UUID, and more), otherwise a random string | |
| `pattern` | a string built from the regex | a string the regex does not match |
| `format: email`, `uuid`, `uri`, `url` | a value in that format | a string that is not in that format |
| `type: date`, `type: timestamp` | `YYYY-MM-DD`, RFC 3339 | a string that does not parse |
| nested `properties` | generated recursively | as above, on the nested field |
| array `items` | generated recursively | as above, on the first item (`lines[0].qty`), patterns included |
| quality `completeness` | present, non-null, non-empty | absent, `null` or `""` |
| quality `freshness` | inside `max_age_seconds` of the anchor time: Unix seconds on number fields, RFC 3339 on `timestamp` and string fields | an older timestamp |
| quality `uniqueness` | no repeated value in the batch | repeating an earlier row's value |
| `metrics` with `field`, `min`, `max` | inside the metric range | a value outside it |
| `compliance_mode: true` | declared fields only | one extra undeclared field |
| `cross_field` comparison | holds (for example `end_date >= start_date`) | values that make it fail; label rule `cross_field`, `constraint` holds the rule |
| `cross_field` `required_if` | the field is present when the condition holds | condition true and the field left out |
| `cross_field` on array items (`lines[].qty`) | holds for every item | the first item fails it (`lines[0].qty`) |

Cross-field rules are described in the [cross-field rules reference](cross-field-rules-reference.md).

Every row is checked with the same validator ingest uses before it is returned. Valid rows pass it, with no batch uniqueness duplicates. Each invalid row fails exactly one rule, and its `broken_rule` label names that rule.

When a rule cannot be met, the call fails with the rule and field named. Examples: `min_length` above `max_length`, an `enum` with no value inside `min_length`, `uniqueness` on a boolean for more than 2 rows, or `freshness` on a boolean field.

In invalid mode the generator tries to break every rule alone. When breaking the target also breaks another rule, it moves the other fields to keep them valid while the target stays broken: the other side of a comparison is shifted, a `required_if` condition is flipped, an optional field is dropped. A rule is left out only when no row can fail it alone, and it is listed in `skipped_rules` with a `reason` that names the rule it is tied to. These are the cases:

| Rule | Why it cannot fail alone |
|---|---|
| `required` on a field a metric or `completeness` rule also reads | a missing field fails that rule too |
| `completeness` on a required field | a missing or `null` value fails `required`; `""` fails `type`, `min_length`, `pattern` or `format` |
| `min` / `max` inside a stricter metric range | a value outside the field range is outside the metric range too |
| `type` on a float or other non-integer field that a metric reads | any non-number fails the metric too (an integer field is broken with a fractional number instead) |
| `min` / `max` tied by a comparison to a required field that cannot move, for example `max_guests >= min_guests` with both `min: 1` | `max_guests` below 1 is below every allowed `min_guests` |
| `format: date-time` on a `timestamp` field with `freshness` | freshness reads the same RFC 3339 value, so a bad string fails both |

Fix the contract (widen the field range, drop the duplicate rule) if one of these should be testable alone.

The same seed, anchor time, contract and DataContractGate version give the same rows. The anchor time defaults to now, so without it freshness values (and only those) change between runs. Rows anchored far from now fail real-time freshness on ingest.

## Value distributions

By default numbers are drawn evenly (uniform) between `min` and `max`. With no bounds, integers come from 0 to 10,000 and numbers from 0 to 1,000, and numbers are rounded to 2 decimals. To shape the values, add a `distribution` hint to an `integer` or `number` field:

```yaml
ontology:
  entities:
    - name: order_total
      type: number
      min: 0
      distribution: { type: lognormal, mean: 3.5, sd: 0.6 }
    - name: age
      type: integer
      min: 18
      max: 90
      distribution: { type: normal, mean: 42, sd: 12 }
    - name: wait_seconds
      type: number
      min: 0
      distribution: { type: exponential, rate: 0.05 }
```

| `type` | Parameters | Values |
|---|---|---|
| `uniform` | none (`distribution: uniform`) | Evenly between `min` and `max`. The default, and the same rows as no hint. |
| `normal` | `mean`, `sd` (greater than 0) | Bell curve around `mean`. |
| `lognormal` | `mean`, `sd` (greater than 0) | `exp(N(mean, sd))`. `mean` and `sd` describe the underlying normal, the way NumPy and R do, not the resulting values. For values with a mean near 100 and a long right tail, try `mean: 4.5, sd: 0.5`. Always positive. |
| `exponential` | `rate` (greater than 0) | `min + Exp(rate)` (from 0 with no `min`). The spread above the start averages `1 / rate`. Right-skewed. |

Rules:

- `min` and `max` still hold. A draw outside them is redrawn, so the result is the distribution cut off at the bounds. If the bounds leave almost no mass (a mean of 1000 with `max: 10`), the call fails with `rule: distribution` and the field named.
- Integers are rounded to the nearest whole number. Numbers keep 2 decimals, or more when the values are small.
- An `enum` on the field wins: its values are picked evenly and the hint is ignored.
- A hint on a string, date or other non-numeric field fails the call (`rule: distribution`). Dates and timestamps have no distribution hint yet.
- The same seed gives the same rows.
- It is synth-only. Validation, ingest and every other feature ignore it. A bad hint (`sd: 0`, an unknown `type`, a missing parameter) is rejected when the contract is read, with a message that names the problem.
- ODCS: put it in the property's `customProperties` as `x-cg-distribution` with the same map as the value (see [odcs/using-odcs-directly.md](odcs/using-odcs-directly.md)).

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
| `anchor_time` | RFC 3339 string | Optional. Time freshness rules are measured from. Now when omitted; returned either way. |
| `envelope` | boolean | Optional. Also return the rows wrapped in the contract's `envelope` as `envelope_payload`. |

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
  "anchor_time": "2026-11-25T18:00:00Z",
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
  "skipped_rules": [],
  "generation_ms": 3
}
```

`generation_ms` is the server time spent compiling the contract and generating rows.

In `valid` mode each entry of `rows` is the record itself. A broken cross-field rule also carries `constraint`, the rule's label (for example `end_date >= start_date`).

With `envelope: true`, `envelope_payload` holds the records (valid or invalid) at the contract's `records_path`. When the envelope has `validate_wrapper: true`, the wrapper also gets `success: true` and `pagination: {page: 1, limit: n, total: n, hasMore: false}`.

Errors:

| Status | `error` | When |
|---|---|---|
| 400 | `invalid_rows` | `rows` is 0 or above 10000 |
| 400 | `no_envelope` | `envelope: true` and the contract declares no envelope |
| 400 | | `anchor_time` is not RFC 3339 |
| 400 | | both or neither of `yaml_content` and `contract_id`, or the contract has no stable version and no `version` was sent |
| 404 | | `contract_id` or `version` is not in your org |
| 422 | `invalid_contract` | the YAML does not parse or compile |
| 422 | `unsatisfiable_rule` | a rule cannot be met. `rule` and `field` name it. |

## CLI

```
contractgate synth <CONTRACT> [--rows N] [--invalid] [--seed S] [--anchor RFC3339] [--envelope] [--format json|csv] [--schema-object NAME] [--synth-config FILE] [-o FILE]
```

Runs locally on a contract YAML file. No API key or network is needed.

- `--format json` (default) prints a JSON array. With `--invalid`, each entry is `{record, broken_rule}`.
- `--format csv` flattens nested objects into dot-path columns and writes arrays as JSON text. A field whose name itself contains a dot or brackets (`Rate.x`, `Amount [A]`) is one column, written under that name. With `--invalid`, the `_broken_field` and `_broken_rule` columns hold the label.
- `--synth-config FILE` reads per-field generation hints from a separate YAML file. See [Synth config files](#synth-config-files).
- Without `--seed`, the chosen seed is printed to stderr so the run can be repeated.
- `--anchor` sets the time freshness rules are measured from (default now).
- `--envelope` (JSON only) prints the wrapped payload. With `--invalid` it prints `{payload, labels}`.

Exit codes: 0 rows written, 1 a rule cannot be met (the message names it), 2 the contract cannot be read or compiled.

```bash
contractgate synth contracts/orders.yaml --rows 500 --seed 42 > orders.json
contractgate synth contracts/orders.yaml --rows 50 --invalid --format csv -o bad_orders.csv
contractgate synth datacontract.yaml --rows 100 --seed 1 --synth-config synth.yaml
contractgate test --contract contracts/orders.yaml --data orders.json   # all pass
```

## Synth config files

A distribution describes observed data. It is not a rule the validator enforces,
so it can live outside the contract. `--synth-config` points at a YAML file
whose `fields` map is keyed by field name. The key is the literal name,
including dots (`Rate.x`). A path such as `shipping.country` addresses a nested
property of a native contract. Several files can describe the same contract;
each run uses the one you pass.

```yaml
fields:
  "Rate.x":
    distribution: { type: normal, mean: 40, sd: 8 }
  sku:
    prefix: "SKU-"
    suffix: "-A"
    pattern: "^SKU-[A-Z]{4}-A$"
```

| Key | Applies to | Effect |
|---|---|---|
| `distribution` | integer, number | Same shapes as an in-contract hint: `uniform`, `normal` (`mean`, `sd`), `lognormal` (`mean`, `sd` of the underlying normal), `exponential` (`rate`). Overrides `distribution` on the field and ODCS `x-cg-distribution`. |
| `prefix`, `suffix`, `pattern` | string | Shape the generated text. `pattern` here is a hint. The contract's own `pattern` is what the row must match. |

Contract rules win. `min` and `max` still clip a distribution (a draw outside them is redrawn). `enum` is what gets picked, and the hint is ignored. `format`, length bounds, and the contract `pattern` still have to pass, or the run fails and names the field. In-contract `distribution` hints keep working when you do not pass a config file.

## Dashboard

Open a contract and pick the **Synthetic data** tab. Set rows, mode, an optional seed and anchor time, and whether to wrap rows in the envelope, then **Generate synthetic data**. The tab sends the YAML of the version on screen, so drafts work. Download the result as JSON or CSV (same layout as the CLI).

## Python SDK

The Python package (0.4.3) adds a `contractgate-py synth` command (also `python -m contractgate synth`) that calls the API, plus `Client.synth(...)`.

```
contractgate-py synth contracts/orders.yaml --rows 100 --seed 42 --anchor 2026-01-01T00:00:00Z
contractgate-py synth --contract-id <uuid> --invalid --format csv -o bad.csv
python -m contractgate synth contracts/orders.yaml --rows 10
```

It takes the same options as the Rust CLI plus `--contract-id`, `--version`, `--api-key` (default `$CONTRACTGATE_API_KEY`) and `--base-url` (default `$CONTRACTGATE_BASE_URL`, then `https://api.datacontractgate.com`). Exit codes: 0 rows written, 1 a rule cannot be met, 2 any other error. The command is named `contractgate-py` so it never shadows the Rust `contractgate` binary: both can be installed side by side. `contractgate` runs locally, `contractgate-py` calls the API.

## ODCS input

`contractgate synth` and `POST /contracts/synth` accept an ODCS v3.x document
with no `ontology`. Property names are literal columns, including dots and brackets.
Pass `--schema-object NAME` (or `schema_object` in the API) to pick one object of a multi-object file. The ignored-objects note is omitted when you pass it. Other unmapped content is still listed in `notes`.
See [`odcs/using-odcs-directly.md`](odcs/using-odcs-directly.md).
