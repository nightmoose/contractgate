# DataContractGate and dbt contracts

This page compares DataContractGate contracts with dbt model contracts,
and shows how to convert one into the other. It is not a replacement for
dbt's own documentation, and it does not replace
[Batch: Snowflake and Databricks](batch-snowflake-databricks-reference.md),
which covers *tests* generated from a contract (`export dbt-tests` and
`check-table`).

## What each format is for

A **dbt model contract** lives in `schema.yml`. With
`config.contract.enforced: true`, dbt checks that a built relation has
the declared column names, `data_type` values, and constraints
(`not_null`, `unique`, `primary_key`, `check`, `foreign_key`) before it
considers the model successful. Generic tests (`not_null`, `unique`,
`accepted_values`, `relationships`) can sit on the same file, with or
without a contract.

A **DataContractGate contract** is a YAML document the gateway uses to
validate events at ingest, on a stream, or at egress. The same document
can also drive batch SQL (RFC-119). Besides types and required fields it
can declare enums, patterns, formats, lengths, freshness, uniqueness,
cross-field rules, an envelope, and optional PII transforms.

The two overlap on flat columns: type, required / not-null, uniqueness,
enums, and simple numeric ranges. They do not overlap on everything
else, which is why the converter has a lossless path and a best-effort
path.

## What maps

| Idea | dbt model contract | DataContractGate |
|---|---|---|
| Name | `models[].name` | `name` |
| Description | `models[].description` and column `description` | `description` and `glossary` |
| Type | column `data_type` (warehouse spelling, typmods allowed on import) | `type` (`string`, `integer`, `float` / `number`, `boolean`, `date`, `timestamp`) |
| Required | `not_null` constraint or test | `required: true` |
| Unique | `unique` / `primary_key` constraint or `unique` test | `quality` rule `type: uniqueness` |
| Enum | `accepted_values` test | `enum` |
| Numeric range | `check` of the form `{col} >= N` / `{col} <= N`, or `dbt_utils.accepted_range` | `min` / `max` |
| Pattern, format, min/max length | no model-contract equivalent | `pattern`, `format`, `min_length`, `max_length` |
| Freshness, cross-field, envelope, PII transforms | no model-contract equivalent | `quality`, `cross_field`, `envelope`, `transform` |
| Nested object or array fields | not a flat column | `type: object` / `array` |
| Foreign keys | `foreign_key` constraint or `relationships` test | not in the contract format today |

Export writes portable `data_type` names: `varchar`, `integer`,
`float`, `boolean`, `date`, `timestamp`. Import accepts common warehouse
spellings, including `varchar(256)` and `numeric(10,2)`.

## CLI

```bash
# DataContractGate contract -> dbt schema.yml
contractgate export dbt contracts/orders.yaml --model orders --out models/schema.yml

# dbt schema.yml -> DataContractGate contract
contractgate import dbt models/schema.yml --model orders --out contracts/orders.yaml

# Fail a CI job when the two have drifted
contractgate import dbt models/schema.yml --model orders --check-against contracts/orders.yaml
```

`--model` is required when `schema.yml` contains more than one model.
`--out -` writes to stdout. Import warnings (unmapped checks, skipped
foreign keys, missing `data_type`) go to stderr. `--check-against`
exits 0 when the files agree on the compared fields, 1 when they differ,
and 2 on a load or parse error.

## API

These routes follow the same authentication, ownership, and persist
rules as the ODCS import and export endpoints.

- `POST /contracts/import/dbt` with JSON
  `{ "dbt_yaml": "...", "model": "orders", "name_override": "...", "target_contract_id": "..." }`
  creates a native draft, or a new version on a contract the caller
  owns. A free plan is capped the same way as `POST /contracts/import`.
- `GET /contracts/{id}/versions/{version}/export/dbt?model=orders`
  returns `application/yaml`.

v1 does not add a new `import_source` value, so there is no database
migration.

## Lossless round-trip

`export dbt` writes a `meta.contractgate` block on the model and on
columns for rules dbt model contracts cannot express. `import dbt`
rebuilds the contract from the live columns, tests, and constraints,
then overlays that block. A file produced by `export dbt` therefore
round-trips: patterns, lengths, freshness, cross-field rules, and
nested fields come back. Keep the `meta.contractgate` block if you want
that behavior. A dbt-native `schema.yml` without the block is imported
best-effort, and skipped items are reported as warnings.

## How this relates to `export dbt-tests`

`export dbt` emits the *model contract* (`enforced: true`, types,
constraints). `export dbt-tests` (RFC-119) emits *tests*: generic
`not_null` / `unique` / `accepted_values` plus singular SQL tests for
patterns, formats, ranges, lengths, freshness, and cross-field rules.
Use the model-contract export when you want dbt to enforce types at
build time. Use the test export when you want warehouse tests for rules
the model contract cannot express. The two can sit in the same project.

## GitHub Action

`.github/actions/dbt-contract-check` runs the `--check-against` command.
The `contractgate` CLI must already be on PATH, or pass its path as
`binary`.

```yaml
- uses: nightmoose/contractgate/.github/actions/dbt-contract-check@main
  with:
    contract: contracts/orders.yaml
    schema: models/schema.yml
    model: orders
```

## Limits

- `sources:` and `semantic_models:` are ignored. Only `models:` are converted.
- Foreign keys and `relationships` tests are reported and skipped.
- Check expressions that are not a simple `{column} >= N` or `{column} <= N`
  comparison are reported and skipped.
- Object, array, and `any` fields are not written as dbt columns. They
  are stored in `meta.contractgate` on export so a round-trip can restore
  them.
- A `>` or `<` check is imported as an inclusive `min` / `max`. Adjust
  the contract if you need a strictly open bound.
