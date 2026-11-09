# dbt ↔ DataContractGate quarantine check (self-host)

SQL against your **self-hosted** Postgres `quarantine_events` table. Cloud
customers should use the HTTP example under [`../airflow/`](../airflow/)
instead — hosted tenants do not get direct SQL to Supabase.

## Drop-in

1. Copy `sources.yml` into your dbt project’s `models/` (or merge the source).
2. Copy `tests/assert_no_pending_quarantine.sql` into your dbt `tests/` folder.
3. Optionally copy the `data_tests` block from `schema.yml`, or set severity
   in your own YAML.
4. Point the dbt profile at the gateway database (same as `make demo` /
   self-host compose).
5. On the ingest and egress calls for this invocation, send
   `X-Batch-Id: {{ invocation_id }}` (or `?batch_id=`). The test reads
   `invocation_id` unless you override it.

## Run

```bash
dbt test --select assert_no_pending_quarantine --vars '{
  "dcg_quarantine_contract_ids": ["YOUR-CONTRACT-UUID"]
}'
```

The test fails when `quarantine_events.batch_id` equals this dbt invocation
(or `dcg_quarantine_batch_id` when a different job stamped the rows) and
`status` is `pending`.

Optional var `dcg_quarantine_direction`: `ingress` or `egress`.

Time-window fallback, for rows written without a run id:

```bash
dbt test --select assert_no_pending_quarantine --vars '{
  "dcg_quarantine_use_time_window": true,
  "dcg_quarantine_contract_ids": ["YOUR-CONTRACT-UUID"],
  "dcg_quarantine_lookback_hours": 24
}'
```

Warn instead of fail: set `severity: warn` on the test config.

## Limits

- Requires self-host SQL access to `quarantine_events`, including migration
  `049_quarantine_batch_id.sql`.
- Rows with a null `batch_id` are invisible to the default filter. Use
  `dcg_quarantine_use_time_window` for those.

See [`docs/quarantine-batch-reference.md`](../../docs/quarantine-batch-reference.md)
and [`docs/architecture/where-to-enforce-contracts.md`](../../docs/architecture/where-to-enforce-contracts.md).
