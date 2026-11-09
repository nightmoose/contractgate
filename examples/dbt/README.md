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

## Run

```bash
dbt test --select assert_no_pending_quarantine --vars '{
  "dcg_quarantine_contract_ids": ["YOUR-CONTRACT-UUID"],
  "dcg_quarantine_lookback_hours": 24
}'
```

Optional var `dcg_quarantine_direction`: `ingress` or `egress`.

Warn instead of fail: set `severity: warn` on the test config.

## Honest limits

- No per-run / Airflow `run_id` on quarantine rows — filter by contract + time.
- Requires self-host SQL access to `quarantine_events`.

See [`docs/architecture/where-to-enforce-contracts.md`](../../docs/architecture/where-to-enforce-contracts.md).
