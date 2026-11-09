# Airflow ↔ DataContractGate quarantine gate

The load step sends `X-Batch-Id` set to the DAG run's `run_id`. The check
calls `GET /quarantine/summary?contract_id=&batch_id=` and treats `pending > 0`
as a signal to stop (ShortCircuit) or fail before dbt.

`requests` percent-encodes the run id, so a value that contains `+`
(`scheduled__2026-11-09T00:00:00+00:00`) round-trips. Send that same value as
the `X-Batch-Id` header on `POST /v1/ingest/{contract_id}` and
`POST /egress/{contract_id}`. `ingest_headers(run_id, api_key)` builds those
headers.

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
4. Replace `load_raw` so it posts events with `ingest_headers(run_id, api_key)`.
5. Replace the `dbt_build` EmptyOperator with your real dbt task.

## Behaviour

| Mode | Operator | Effect when pending &gt; 0 |
|---|---|---|
| Soft (default in the sample) | `ShortCircuitOperator` | Downstream skipped |
| Hard | `PythonOperator(fail_if_pending)` | Task fails red |

## Time-window fallback

Set Variable `dcg_quarantine_use_time_window` to `true` to page
`GET /quarantine` and keep `status=pending` rows with
`quarantined_at >= data_interval_start`. The check also does that when
`run_id` is missing, and when `GET /quarantine/summary` returns 404.

The list response includes `batch_id`. It does not include `direction`. The
summary's `by_direction` does. Use separate contracts, or self-host SQL, when
the gate must ignore one direction.

Free Cloud plans may redact `raw_event` bodies. The summary counts still work.

Reference: [`docs/quarantine-batch-reference.md`](../../docs/quarantine-batch-reference.md).
Architecture: [`docs/architecture/where-to-enforce-contracts.md`](../../docs/architecture/where-to-enforce-contracts.md).
