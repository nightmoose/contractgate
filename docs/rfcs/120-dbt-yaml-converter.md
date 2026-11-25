# RFC-120: dbt YAML import and export

**Status:** Accepted
**Date:** 2026-11-25

## Problem

Teams that already declare dbt model contracts (`schema.yml` models with
`config.contract.enforced: true`, column `data_type`, constraints, and
tests) have no supported way to turn those files into DataContractGate
contracts, or to publish a DataContractGate contract as a dbt model
contract. The RFC-119 exporter writes dbt *tests* (generic tests plus
singular SQL). It does not emit or consume the dbt *model contract*
shape, so type and constraint declarations can drift from the gateway
contract by hand.

## Decision

A two-way converter, no new crate dependencies, and no database
migration:

1. **CLI.** `contractgate export dbt <contract.yaml> [--model name]
   [--out schema.yml]` writes a dbt `schema.yml` with
   `config.contract.enforced: true`, a portable `data_type` on each
   flat column, `not_null` / `unique` / simple `check` constraints, and
   `accepted_values` tests for enums. `contractgate import dbt
   <schema.yml> [--model name] [--out contract.yaml]` does the reverse.
   `--check-against <contract.yaml>` on import compares the dbt file to
   an existing contract and exits 1 when they differ.

2. **API, mirroring ODCS conventions.**
   `POST /contracts/import/dbt` accepts `{ "dbt_yaml": "...", "model",
   "name_override", "target_contract_id" }` and creates a native draft
   (or a new version on a target the caller owns).
   `GET /contracts/{id}/versions/{version}/export/dbt?model=` returns
   `application/yaml`. v1 does not add an `import_source = dbt` value,
   so this change does not alter the shared database check constraint.

3. **Round-trip.** Export writes a `meta.contractgate` block for rules
   dbt model contracts cannot express (pattern, format, lengths, nested
   fields, freshness, cross-field rules, PII transforms, envelope).
   Import prefers live dbt columns, tests, and constraints, then
   overlays that block so a file we exported comes back lossless. A
   dbt-native file without the block is a best-effort import; skipped
   items (relationships / foreign keys, unparsed check expressions,
   object and array columns) are reported as warnings.

4. **GitHub Action.** `.github/actions/dbt-contract-check` runs
   `contractgate import dbt <schema.yml> --check-against <contract.yaml>`
   so a PR can fail when a dbt model contract and a DataContractGate
   contract drift. The CLI must already be on PATH (or passed as
   `binary`).

5. **Docs.** `docs/datacontractgate-and-dbt-contracts.md` compares the
   two formats factually and points at RFC-119 for warehouse *tests*.

### Type mapping

Import accepts common warehouse spellings (`varchar`, `varchar(256)`,
`integer`, `bigint`, `float`, `double`, `numeric`, `boolean`, `date`,
`timestamp`, `timestamptz`, and close variants) and typmods. Missing
`data_type` becomes `string` with a warning. Export writes portable
names: `varchar`, `integer`, `float`, `boolean`, `date`, `timestamp`.
Object, array, and `any` fields are not dbt columns; they are stored
in `meta.contractgate` for a lossless round-trip and skipped with a
warning otherwise.

Simple `check` expressions of the form `{column} >= N` or `{column} <=
N` map to `min` / `max`. Other expressions are warnings.

## Not in v1

- A new `import_source` enum value and a database migration.
- `sources:` or `semantic_models:` stanzas (models only).
- Foreign-key / `relationships` tests (no FK concept in the contract
  format; same gap as RFC-119).
- Live compilation of the consumer's dbt project.

## Docs

`docs/datacontractgate-and-dbt-contracts.md`.
