# Schema drift agent

A data engineer wanted one thing automated: catch an upstream schema change before it breaks the pipeline, flag what is affected downstream, and draft the fix for a person to review. The alternative is finding out from a broken dashboard.

This folder is a small local version of that loop, on the DataContractGate engine in this repo. It is vendor-neutral: DuckDB via dbt, JSON fixtures, no warehouse account, no API key. Docker is not required.

`make demo` does three things and then stops:

1. **Catch.** A good orders batch and a drifted batch are checked with the `contractgate` CLI. The drifted batch has a renamed field (`amount` → `order_total`), a dropped field (`customer_id`), a type change (`ordered_at` integer → RFC3339 string), and schema-valid-but-wrong values (dollars sent as cents, unix seconds sent as milliseconds). Failures are written to `out/catch/quarantine.ndjson`. Passing rows are written to `out/catch/landed.ndjson`.
2. **Flag.** A tiny dbt project (staging → marts, two dashboard exposures) is compiled with dbt-duckdb. The agent reads `manifest.json` and lists every model, test, and exposure that depends on each drifted column.
3. **Draft.** A unified diff updates the staging model (rename, `/100`, timestamp cast) and the ingest contract. A pull-request description explains what to review. Nothing is applied or merged.

Deterministic heuristics do all of this. An optional LLM rewrite of the pull-request text runs only when `SCHEMA_DRIFT_LLM_BASE_URL` is set, and the demo is complete without it.

## 60-second quickstart

From a checkout with Rust (the repo pins 1.98.0) and Python 3.11+:

```bash
cd examples/schema-drift-agent
make demo
```

The first run compiles the `contractgate` CLI and installs dbt into `.venv`. That compile is the slow part. Later runs are a few seconds. The demo itself does not call a warehouse or an API.

On Debian or Ubuntu, if `python3 -m venv` says `ensurepip is not available`, install `python3-venv` and run `make demo` again.

Generated files live under `out/` and `dbt/target/`. Both are gitignored. The contract and the dbt models are not modified.

## Sample output

This is the report from a successful `make demo` (the CLI timing line is not part of the report).

### 1. Catch

```
# 1. Catch

Command: `contractgate test --contract contracts/orders.yaml --data fixtures/orders_drifted.json --format json`

This is the DataContractGate CLI. It runs the same `validate()` engine as
`POST /v1/ingest/{contract_id}`. Failed records are written to
`out/catch/quarantine.ndjson` and are not written to `out/catch/landed.ndjson`.
The CLI does not open Postgres, so those files are the local stand-in for
`quarantine_events`. A running gateway stores the same failures when you POST
the batch to ingest.

## Good batch — `fixtures/orders_good.json`

4 passed, 0 failed.

## Drifted batch — `fixtures/orders_drifted.json`

1 passed, 5 failed.

## What failed

- Renamed field: `amount` is missing where `order_total` was sent (ord_2001, ord_2002).
- Dropped field: `customer_id` (ord_2001, ord_2002).
- Type change: `ordered_at` is an RFC3339 string, the contract wants an integer (ord_2001, ord_2002, ord_2005).
- Schema-valid but wrong: `amount` is a number above the dollar max, at about 100× (ord_2003, ord_2004); `ordered_at` is an integer above the seconds max, at about 1000× (ord_2003).

### Landed

- record 0 `ord_1001` passed and would land

### Quarantined

#### record 1 — `ord_2001`

| field | kind | why |
| --- | --- | --- |
| `customer_id` | `missing_required_field` | Required field 'customer_id' is missing |
| `amount` | `missing_required_field` | Required field 'amount' is missing |
| `ordered_at` | `type_mismatch` | Field 'ordered_at' expected type Integer, got string |
| `order_total` | `undeclared_field` | Field 'order_total' is not declared in the contract ontology. Compliance mode rejects undeclared fields. |

#### record 3 — `ord_2003`

| field | kind | why |
| --- | --- | --- |
| `amount` | `range_violation` | Field 'amount' value 1550 exceeds maximum 300 |
| `ordered_at` | `range_violation` | Field 'ordered_at' value 1731456000000 exceeds maximum 1893456000 |

#### record 4 — `ord_2004`

| field | kind | why |
| --- | --- | --- |
| `amount` | `range_violation` | Field 'amount' value 25000 exceeds maximum 300 |

#### record 5 — `ord_2005`

| field | kind | why |
| --- | --- | --- |
| `ordered_at` | `type_mismatch` | Field 'ordered_at' expected type Integer, got string |
```

`ord_2002` is the same shape as `ord_2001` (rename, drop, and the string timestamp). The full table is in `out/catch/report.md`.

### 2. Downstream impact

```
# 2. Downstream impact

Manifest: `dbt/target/manifest.json` (3 models, 14 tests, 2 exposures).

## `amount`

Renamed to `order_total` on ord_2001, ord_2002.
Changed meaning (dollars → cents, divide by 100) on ord_2003, ord_2004.

- `amount` changed meaning -> stg_orders -> fct_orders -> exposure 'Orders dashboard'
- `amount` changed meaning -> stg_orders -> fct_revenue -> exposure 'Revenue dashboard'

Tests:

- `assert_amount_is_dollars`
- `not_null_fct_orders_amount` on `fct_orders.amount`
- `not_null_stg_orders_amount` on `stg_orders.amount`

## `customer_id`

Dropped on ord_2001, ord_2002. No same-type replacement was paired.

- `customer_id` dropped -> stg_orders -> fct_orders -> exposure 'Orders dashboard'

Tests:

- `not_null_fct_orders_customer_id` on `fct_orders.customer_id`
- `not_null_stg_orders_customer_id` on `stg_orders.customer_id`

## `ordered_at`

Changed type from integer to an RFC3339 string on ord_2001, ord_2002, ord_2005.
Changed meaning (seconds → milliseconds, divide by 1000) on ord_2003.

- `ordered_at` changed type / changed meaning -> stg_orders -> fct_orders -> exposure 'Orders dashboard'
- `ordered_at` changed type / changed meaning -> stg_orders -> fct_revenue -> exposure 'Revenue dashboard'

Tests:

- `not_null_stg_orders_ordered_at` on `stg_orders.ordered_at`

## `order_total`

New wire name for amount. Divide by 100 in staging.

No model names `order_total`.
It is the new name for `amount`. Follow `amount` for downstream impact.
```

`fct_revenue` is not on the `customer_id` path. The revenue dashboard sums `amount` and does not join the buyer. The orders dashboard does.

### 3. Draft fix (not applied)

```
# Draft: adapt orders staging to the upstream schema change

Draft for review. Nothing was applied or merged. schema-drift-agent does not
switch a branch or edit the working tree.

## What the gate caught

- Renamed field: `amount` is missing where `order_total` was sent (ord_2001, ord_2002).
- Dropped field: `customer_id` (ord_2001, ord_2002).
- Type change: `ordered_at` is an RFC3339 string, the contract wants an integer (ord_2001, ord_2002, ord_2005).
- Schema-valid but wrong: `amount` is a number above the dollar max, at about 100× (ord_2003, ord_2004); `ordered_at` is an integer above the seconds max, at about 1000× (ord_2003).

## Downstream

- `amount` changed meaning -> stg_orders -> fct_orders -> exposure 'Orders dashboard'
- `amount` changed meaning -> stg_orders -> fct_revenue -> exposure 'Revenue dashboard'
- `customer_id` dropped -> stg_orders -> fct_orders -> exposure 'Orders dashboard'
- `ordered_at` changed type / changed meaning -> stg_orders -> fct_orders -> exposure 'Orders dashboard'
- `ordered_at` changed type / changed meaning -> stg_orders -> fct_revenue -> exposure 'Revenue dashboard'

## Proposed change

- `dbt/models/staging/stg_orders.sql` maps `order_total` and out-of-range `amount` back to dollars, and casts RFC3339 or millisecond `ordered_at` back to unix seconds. The SQL is DuckDB (`try_cast`, `epoch`), matching this dbt project.
- `contracts/orders.yaml` declares `order_total` as cents, and stops requiring `amount` and `customer_id` on the raw feed.
- `contracts/orders_staged.yaml` is unchanged. It still requires dollars, unix seconds, and `customer_id`.

## Staged contract preview

- `ord_1001` pass
- `ord_2001` fail: `customer_id` missing_required_field
- `ord_2002` fail: `customer_id` missing_required_field
- `ord_2003` pass
- `ord_2004` pass
- `ord_2005` pass

## Raw contract draft, still not published

- `ord_1001` pass
- `ord_2001` fail: `ordered_at` type_mismatch
- `ord_2002` fail: `ordered_at` type_mismatch
- `ord_2003` fail: `amount` range_violation, `ordered_at` range_violation
- `ord_2004` fail: `amount` range_violation
- `ord_2005` fail: `ordered_at` type_mismatch

## Review before accepting

- `amount` values above the contract max are treated as cents. A cents value that still falls under that max would pass the range check and would not be divided. Prefer the renamed `order_total` field, which is always divided.
- `ordered_at` stays an integer of unix seconds in the contract. The drifted batch mixes RFC3339 strings and integers, and one field has one type. Strings and milliseconds keep failing ingest until you pick a wire type. The staging cast is ready for both.
- `customer_id` was dropped on some rows. The draft does not invent it. Those rows fail the staged contract.
```

The staging expressions in `out/draft/proposed.patch`:

```sql
case
    when order_total is not null then order_total / 100.0
    when amount > 300 then amount / 100.0
    else amount
end as amount,
case
    when try_cast(ordered_at as varchar) like '%-%' then cast(epoch(try_cast(ordered_at as timestamptz)) as bigint)
    when try_cast(ordered_at as bigint) >= 100000000000 then cast(try_cast(ordered_at as bigint) / 1000 as bigint)
    else try_cast(ordered_at as bigint)
end as ordered_at
```

The same expressions are executed in DuckDB against the drifted batch, and the rows are checked with `contractgate test` against `contracts/orders_staged.yaml` (dollars, unix seconds, `customer_id` required). Rows that lost `customer_id` still fail that contract. The others pass.

The contract diff declares `order_total` (cents, max 30000), and sets `amount` and `customer_id` to `required: false` on the raw feed. It does not raise `amount.max` and it does not change `ordered_at` to a string, so cents-on-`amount`, millisecond timestamps, and RFC3339 strings still fail the draft contract. That is intentional: those rows stay quarantined until someone accepts the unit change.

If you accept the draft, from this directory:

```bash
git apply --check out/draft/proposed.patch
git apply out/draft/proposed.patch
```

`make demo` runs `git apply --check` and does not apply the patch.

## How the gate is invoked

`contractgate test` (the `contractgate` binary, not the Python package on PyPI) loads the contract YAML and runs every record through `validate()` in-process. Exit 0 means every record passed. Exit 1 means at least one record failed. Exit 2 means the contract or the file could not be loaded. See [docs/cg-test-reference.md](../../docs/cg-test-reference.md).

`--format json` still prints the per-record lines before the JSON document, on the same stdout. The agent reads the JSON object from the end of that output.

The HTTP ingest API is the same check with a real quarantine table. It needs a running gateway, an API key, and a stored contract id, so it is not what `make demo` calls. When you already have those:

```bash
curl -s -X POST "$GATEWAY/v1/ingest/$CONTRACT_ID" \
  -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
  -H "Content-Type: application/json" \
  --data-binary @fixtures/orders_drifted.json
```

Failures then land in `quarantine_events`. The reference is [docs/v1-ingest-reference.md](../../docs/v1-ingest-reference.md).

## Point it at your own project

```bash
.venv/bin/python drift_agent.py \
  --contract path/to/orders.yaml \
  --staged-contract path/to/orders_staged.yaml \
  --good path/to/good.json \
  --drifted path/to/drifted.json \
  --dbt-project path/to/dbt \
  --staging-model models/staging/stg_orders.sql
```

- The ingest contract is normal DataContractGate YAML (`ontology.entities` with `type`, `required`, `min` / `max`, `enum`). Set `compliance_mode: true` or a renamed field is invisible: without it, extra keys are allowed.
- Glossary `synonyms` help the rename pairing. A single extra field whose JSON type matches the missing field is paired even without a synonym.
- `--good` is the last batch you trust. It is the baseline for the cents and milliseconds guess. The guess only considers `/100` and `/1000`, and only when the scaled values fall back inside the contract range.
- `--staged-contract` is the contract your marts already assume. The agent normalizes the drifted rows with the draft rules and checks them against this file. Use the current ingest contract here if staging is supposed to preserve today's meaning.
- `--dbt-project` must `dbt compile`. If `profiles.yml` sits in that directory, it is used (this example ships a DuckDB profile). Otherwise dbt uses `~/.dbt/profiles.yml`.
- `--staging-model` is a path relative to the dbt project. Bare select items named `amount` and `ordered_at` are rewritten. If the select item is not a bare column, the proposed expression is left as a SQL comment at the top of the draft file.
- The column scan reads model SQL, schema column names, and test `column_name`, then walks `child_map` to exposures of the models that named the column. dbt Core does not store warehouse column lineage. A model that selects the column under a new name, and never writes the old name, will not be linked.
- The generated cast uses DuckDB (`try_cast`, `epoch`, `typeof` is not required). Read it before pasting into Snowflake or BigQuery.
- The fixture-specific checks (exact order ids, the revenue-dashboard line) run only for this folder's bundled paths. Your files still get the three reports.

## Optional LLM rewrite

The heuristic pull-request text is always `out/draft/pr_description.md`. To ask an OpenAI-compatible server for a second draft:

```bash
export SCHEMA_DRIFT_LLM_BASE_URL=https://api.openai.com/v1
export SCHEMA_DRIFT_LLM_API_KEY=sk-...
export SCHEMA_DRIFT_LLM_MODEL=gpt-4o-mini   # optional
make demo
```

`SCHEMA_DRIFT_LLM_BASE_URL` should be the API root (`https://api.openai.com/v1`, or `http://localhost:11434/v1`). The agent POSTs `{base}/chat/completions`. A failure is printed and ignored. The heuristic file is not replaced. The rewrite, when the call succeeds, is `out/draft/pr_description.llm.md`.

## What the heuristics will not do

- They will not invent a value for a dropped field.
- They will not divide an `amount` that is still inside the contract max. A `$4.99` charge sent as `499` cents passes a max of `300` and is left alone. `order_total` is divided on every row.
- They will not change `ordered_at` to `type: timestamp` while the batch still contains integers. One field has one type.
- They will not apply the patch, open a pull request, or merge.

## CI

[`.github/workflows/schema-drift-agent.yml`](../../.github/workflows/schema-drift-agent.yml) runs `make demo` on pull requests and pushes that touch this folder, the workflow, or the CLI test path. The bundled checks at the end of `drift_agent.py` fail the job if the quarantine split, the impact paths, the patch, or the staged-contract preview drift.
