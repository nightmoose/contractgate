# Airflow ↔ DataContractGate quarantine gate

Calls `GET /quarantine` with your API key, paginates, and treats
`status=pending` rows with `quarantined_at >= data_interval_start` as a
signal to stop (ShortCircuit) or fail before dbt.

Works for **Cloud and self-host** (HTTP). For direct SQL on self-host, see
[`../dbt/`](../dbt/).

## Setup

1. Airflow connection `datacontractgate_api` (type HTTP):
   - Host: `https://api.datacontractgate.com` (or your gateway base URL)
   - Password: your `cg_live_…` API key  
   Or set Variable `datacontractgate_api_key`.
2. Variable `dcg_quarantine_contract_ids` = JSON array of contract UUIDs.
3. Drop `dcg_quarantine_gate_dag.py` into your `dags/` folder (or import the
   helpers into your own DAG).
4. Replace the `dbt_build` EmptyOperator with your real dbt task.

## Behaviour

| Mode | Operator | Effect when pending &gt; 0 |
|---|---|---|
| Soft (default in the sample) | `ShortCircuitOperator` | Downstream skipped |
| Hard | `PythonOperator(fail_if_pending)` | Task fails red |

## Honest limits

- No `batch_id` / Airflow `run_id` on quarantine rows — time window + contract.
- List API returns `status` and `quarantined_at`; it does **not** return
  `direction`. Use separate contracts for ingest vs egress, or self-host SQL.
- Free Cloud plans may redact `raw_event` bodies; counts still work.

Architecture: [`docs/architecture/where-to-enforce-contracts.md`](../../docs/architecture/where-to-enforce-contracts.md).
