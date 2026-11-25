# Batch: Snowflake and Databricks

DataContractGate can now run the same contract rules against a warehouse
table, not only against events that arrive through the API or a stream.
Two surfaces share one SQL compiler (RFC-119):

1. `contractgate export dbt-tests` (and `GET /contracts/{id}/versions/{version}/export/dbt-tests`) writes a dbt `schema.yml` plus singular SQL tests.
2. `contractgate check-table` compiles those rules into one `UNION ALL` query that returns every violating row key, the rule it broke, and a plain-English reason.

Supported dialects: Postgres, DuckDB, Snowflake, and Databricks. Generation is tested for all four with committed golden files. Live `--execute` in this build is Postgres only (see [Limits](#limits)).

This page is about checking a warehouse table against the contract. It is not the same as [per-run quarantine](quarantine-batch-reference.md), which counts `quarantine_events` for a dbt or Airflow run id after rows have already gone through ingest or egress.

## What is generated

| Contract rule | dbt export | `check-table` |
|---|---|---|
| `required: true` | generic `not_null` | `{col} is null` |
| `quality: uniqueness` | generic `unique` | `GROUP BY ... HAVING count(*) > 1` |
| `enum` | generic `accepted_values` | `NOT IN (...)` |
| `pattern`, `format` | singular SQL test | dialect regex (`~`, `regexp_matches`, `regexp_like`, `rlike`) |
| `min` / `max`, `min_length` / `max_length` | singular SQL test | range or `length()` comparison |
| `quality: freshness` with `max_age_seconds` | singular SQL test | `{col} < now() - interval` (dialect interval syntax) |
| `cross_field` comparison or `required_if` | singular SQL test | comparison or "condition holds and field is null" |
| `relationships` / foreign keys | not generated (no FK concept in the contract format) | not generated |
| object, array, or `[]` per-item rules | skipped, with a note | skipped, with a note |

Generic dbt tests use only what dbt ships: `not_null`, `unique`, and `accepted_values`. There is no `dbt_utils` or `dbt_expectations` dependency. A singular test is `select * from {{ ref(model) }} where <violates>`: a non-empty result fails the test, which is dbt's convention.

`quality: validity` is not emitted as its own predicate. The ontology-level pattern, enum, and range rules already cover it.

## CLI

```bash
# dbt tests for a Postgres-flavored warehouse (default dialect).
# Writes <out>/schema.yml and <out>/tests/*.sql
contractgate export dbt-tests contracts/orders.yaml --model orders --out dbt/orders

# Singular tests use Snowflake regex syntax instead.
contractgate export dbt-tests contracts/orders.yaml --model orders --dialect snowflake --out dbt/orders

# Print the check-table violation query (default when --execute is omitted).
contractgate check-table contracts/orders.yaml \
  --table analytics.orders \
  --dialect snowflake \
  --key order_id

# Run it against Postgres and write violating keys to a CSV.
contractgate check-table contracts/orders.yaml \
  --table orders \
  --dialect postgres \
  --key order_id \
  --execute \
  --conn postgres://localhost/mydb \
  --quarantine-out /tmp/orders-violations.csv
```

`--table` is used verbatim in the generated SQL, so schema-qualify and quote it yourself if the warehouse needs that (`analytics.orders`, `"ORDERS"."PUBLIC"."ORDERS"`). `--key` is required and may be a comma-separated list. Exit codes for `check-table`: `0` clean or printed SQL, `1` violations found, `2` contract or argument error, `3` database error.

`--execute` on `--dialect duckdb`, `snowflake`, or `databricks` exits with an error and points you at `--print-sql` plus your own client (DuckDB CLI, SnowSQL, a Databricks SQL warehouse, or `dbt test`).

## API

`GET /contracts/{id}/versions/{version}/export/dbt-tests` with an `x-api-key` header.

| Query param | Notes |
|---|---|
| `model` | dbt model name used in `{{ ref(model) }}`. Defaults to the contract's name. |
| `dialect` | `postgres` (default), `duckdb`, `snowflake`, or `databricks`. Only the singular tests' regex and freshness SQL change. |

```bash
curl -s "https://api.datacontractgate.com/contracts/$CONTRACT_ID/versions/2.0.0/export/dbt-tests?dialect=snowflake" \
  -H "x-api-key: $CONTRACTGATE_API_KEY"
```

The response is JSON: `model`, `dialect`, `schema_yml`, `tests` (each with `filename` and `sql`), and `skipped` (fields the compiler could not translate, each with a `reason`).

There is no API equivalent of `check-table`. Generate the SQL with the CLI and run it in the warehouse.

## Dialects

Range, length, enum, and cross-field comparisons are plain ANSI SQL and are the same in all four dialects. The compiler only changes identifier quoting, regex predicates, text casts on the report's key columns, and "now minus N seconds" arithmetic.

| | Identifier | Regex | Now minus N seconds |
|---|---|---|---|
| Postgres | `"col"` | `col ~ 'pattern'` | `now() - interval 'N seconds'` |
| DuckDB | `"col"` | `regexp_matches(col, 'pattern')` | `now() - interval 'N seconds'` |
| Snowflake | `"col"` | `regexp_like(col, 'pattern')` | `dateadd(second, -N, current_timestamp())` |
| Databricks | `` `col` `` | `col rlike 'pattern'` | `current_timestamp() - interval N seconds` |

Golden files in `tests/batch_sql_golden/` pin the exact generated SQL for the fixture contract in `tests/fixtures/batch_sql/orders.yaml` (a slightly richer copy of the Impact demo `orders` contract, with uniqueness, freshness, length, and two cross-field rules). A change in generated SQL is a reviewable text diff.

## Timed setup on the demo orders contract

The "1 to 2 day setup" claim is about going from an existing contract to working batch checks in a warehouse, not about writing the contract from scratch. The DataContractGate side of that path is the two commands above. Measured at 3:44 PM America/New_York on 2026-11-25 against `tests/fixtures/batch_sql/orders.yaml`, on a Linux x86_64 host with 8 CPUs, using a release build of the `contractgate` CLI already on `PATH`:

| Step | What ran | Elapsed |
|---|---|---|
| 1. Export dbt tests (Postgres dialect) | `contractgate export dbt-tests tests/fixtures/batch_sql/orders.yaml --model orders --out /tmp/dcg-batch-dbt` | 1.9 ms (best of 5; 1.9 to 3.0 ms) |
| 2. Re-export for Snowflake | same command with `--dialect snowflake --out /tmp/dcg-batch-dbt-sf` | 2.1 ms (best of 5; 2.1 to 2.4 ms) |
| 3. Print check-table SQL for all four dialects | `contractgate check-table ... --key order_id` for postgres, duckdb, snowflake, and databricks | 6.5 ms (best of 5; 6.5 to 7.7 ms) |
| 4. Total, steps 1-3 | generation only; no warehouse round-trip | 10.2 ms (one consecutive run of steps 1-3) |

The export wrote `schema.yml` (generic `not_null`, `unique`, and `accepted_values` tests) plus 7 singular SQL tests (`pattern`, `range`, `length`, `freshness`, and two `cross_field` rules). The check-table query compiled 15 predicates into one `UNION ALL` statement per dialect.

Live `--execute` against Postgres is covered by the ignored test `postgres_check_table_reports_planted_violations` in `tests/batch_sql.rs`: it loads one valid row and seven planted violations (bad pattern, negative amount, bad enum, stale freshness, backwards timestamps, cancelled without a discount code, short discount code) and asserts the generated SQL returns exactly those seven `(order_id, rule)` pairs. Run it with `DATABASE_URL=postgres://... cargo test --test batch_sql -- --ignored --nocapture`. It talks to a throwaway table and does not need DataContractGate's own migrations. This environment had no Postgres server, so that test was not executed as part of the timing above.

What remains of a "1 to 2 day" warehouse setup, after these milliseconds of generation, is work on the warehouse side: copying `schema.yml` and the singular tests into the dbt project (or pasting the check-table SQL into SnowSQL / a Databricks SQL warehouse), pointing `--table` at the real relation, and scheduling the check. That is outside DataContractGate and depends on the team's existing dbt or warehouse access.

## Limits

- `--execute` is Postgres only. DuckDB execution was attempted with the `duckdb` crate's bundled build and had to be reverted after it filled the shared build disk. There is no Snowflake or Databricks driver in the dependency allow-list, and no credentials to test one against. SQL generation for those dialects is golden-file tested.
- `relationships` dbt tests are not generated. The contract format has no foreign-key concept. Add them by hand if the warehouse has the other table.
- Object fields, arrays, and `[]` per-item rules are skipped, with a note in the CLI or the API `skipped` list. Batch SQL runs against a flat table column; those rules describe an event shape and would need a warehouse-specific unnesting convention.
- No database migration. This feature reads an external warehouse table, not DataContractGate's own schema.

See [RFC-119](rfcs/119-batch-sql-contracts.md).
