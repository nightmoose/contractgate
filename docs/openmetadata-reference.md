# OpenMetadata Integration Reference

**Added 2026-10-15.** Code and full walkthrough: [`integrations/openmetadata/`](../integrations/openmetadata/README.md).

## CLI

| Command | Purpose |
|---|---|
| `python -m dcg_openmetadata publish --config <file> [--dry-run]` | Post DCG validation stats to OM as test case results |
| `python -m dcg_openmetadata sync --om-contract-id <id> [--target-contract-id <uuid>] [--name-override <name>]` | Import an OM data contract (ODCS YAML) into DCG as a draft |

## Env vars

`OM_HOST`, `OM_JWT`, `DCG_API_URL`, `DCG_API_KEY`, `DCG_ORG_ID` (dev gateway only), `DCG_OM_CONFIG`.

## Mapping file keys

`window_minutes`, `skip_empty`, `test_definition`, `defaults.max_violation_rate`,
`contracts[].contract_id`, `contracts[].table_fqn`, `contracts[].test_case_name`,
`contracts[].max_violation_rate`.

## Contract metadata keys

- Top-level contract YAML key `x-openmetadata-table: <table fqn>`
- Or `openmetadata-table: <table fqn>` in the contract description

## DCG endpoints used

`GET /contracts/{id}`, `GET /contracts/{id}/versions/{version}`, `GET /contracts/{id}/report`,
`GET /contracts/{id}/versions/{version}/export`, `POST /contracts/import`.

## Results posted

`testCaseStatus` (Success/Failed against `max_violation_rate`). `testResultValue`: `total`,
`passed`, `quarantined`, `violation_rate`, `contract_version`. Also `passedRows` and `failedRows`.
