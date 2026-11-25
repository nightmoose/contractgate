# RFC-119: Batch SQL, dbt test export and `check-table`

**Status:** Accepted
**Date:** 2026-11-25

## Problem

DataContractGate is proven for data arriving through the API (`/ingest`,
`/egress`) or event streams (Kafka, Kinesis). Teams whose data lands in a
warehouse first and is enforced on a schedule (dbt run, Airflow task) have no
way to run the same contract rules against a Snowflake, Databricks, Postgres,
or DuckDB table. They either hand-write dbt tests that drift from the
contract, or skip batch enforcement entirely.

## Decision

Two related, contract-driven surfaces, both pure Rust with no network calls
and no new mandatory dependencies:

1. **dbt test export.** `contractgate export dbt-tests <contract.yaml>` and
   `GET /contracts/{id}/versions/{version}/export/dbt-tests` generate:
   - `schema.yml` generic tests: `not_null` (from `required`), `unique`
     (from a `quality: uniqueness` rule), `accepted_values` (from `enum`).
     No `dbt_utils` / `dbt_expectations` dependency.
   - Singular SQL tests, one per field/rule that a generic test can't
     express: `pattern`, `format`, `min`/`max`, `min_length`/`max_length`,
     and one per `cross_field` rule. Each is `select * from {{ ref(model) }}
     where <violates>`: the dbt convention that a non-empty result fails
     the test.
   - `relationships` tests are not generated: the contract format has no
     foreign-key concept today. Documented as a gap; users add them by hand.
   - Fields of type `object` or `array`, and `[]`-wildcard paths (per-item
     rules), are skipped with a comment in the output. dbt tests run
     against a flat table column, and the contract's nested/array rules are
     an event-shape concept that batch SQL can't express without knowing the
     warehouse's unnesting convention. Documented gap.

2. **`check-table`.** `contractgate check-table <contract.yaml> --table
   <name> --dialect <postgres|duckdb|snowflake|databricks> --key <cols>`
   compiles the same rules into one SQL statement: a `UNION ALL` of
   per-rule `SELECT <key columns>, '<rule>' , '<plain-English reason>'`
   subqueries, so the result is a flat violation report with human-readable
   reasons, not just failed-row counts.
   - `--print-sql` (also the default without `--execute`) emits the SQL for
     any dialect. This is what makes Snowflake/Databricks support testable
     without live credentials: golden-file tests assert the generated text.
   - `--execute` runs the query and prints a summary. Supported for
     `postgres` only, via the `sqlx` dependency already in the tree, with no
     new dependency. `--execute` on `duckdb`, `snowflake`, or `databricks`
     errors and points at `--print-sql` plus the user's own SQL client.
     DuckDB was the original plan for a second local-execution dialect (the
     `duckdb` crate, bundled C++ build, same pattern as the existing
     `scaffold`/`kafka-ingress` features that need system build deps), but
     compiling the bundled amalgamation exhausted the shared build sandbox's
     disk during implementation and had to be reverted (see "Not in v1").
     There is also no Snowflake/Databricks driver in the dependency
     allow-list and no credentials to test one against.
   - `--quarantine-out <file>` (requires `--execute`) writes the violating
     key column values plus reasons as CSV, for a remediation / re-ingest
     workflow.

### Dialect handling

A small `SqlDialect` enum covers identifier quoting, regex predicates
(`~` / `REGEXP_LIKE` / `RLIKE` / `regexp_matches`), and `now() - interval`
arithmetic, which is the only part of the generated SQL that differs across
Postgres, DuckDB, Snowflake, and Databricks. Range, length, enum, and
cross-field comparisons are plain ANSI SQL and are identical across all
four. Uniqueness is `GROUP BY ... HAVING count(*) > 1` (no window function
in `WHERE`, which several dialects reject).

### Testing without real warehouse credentials

- Golden-file tests (`tests/batch_sql_golden/`) commit the exact generated
  SQL text for all four dialects against the demo `orders` contract. A
  diff in generated SQL is a reviewable text diff.
- A Postgres integration test (using the existing local test-DB script)
  loads sample rows into a real table and runs the generated SQL end to
  end, asserting the actual violation rows. This is the "proven for
  batch" evidence, standing in for a live Snowflake/Databricks/DuckDB run.

No database migration. This feature reads an external warehouse table, not
DataContractGate's own Supabase schema.

## Not in v1

- Live execution against Snowflake or Databricks (no driver dependency,
  no credentials to test one against). SQL generation is tested with golden
  files; execution is left to the user's own client (SnowSQL, Databricks
  SQL warehouse, `dbt test`).
- Live execution against DuckDB. The `duckdb` crate's bundled build (the
  only option; no system `libduckdb` is installed in this environment)
  compiles a large vendored C++ amalgamation; attempting it filled the
  shared build sandbox's disk (`/workspace` hit 100% used) and had to be
  reverted. `--dialect duckdb` generates correct SQL (golden-file tested)
  but `--execute` is not supported for it, same as Snowflake/Databricks.
  Revisit on a host with more disk, or with a non-bundled `libduckdb`
  system package.
- `relationships` dbt tests (no FK concept in the contract format).
- Per-item (`[]`) and nested-object rules in batch SQL.

## Docs

`docs/batch-snowflake-databricks-reference.md`, including a timed
full-setup walkthrough on the demo `orders` contract.
