# Batch guardrail before DuckDB or MotherDuck

Gate a batch or micro-batch with the **DataContractGate** Python SDK
(`contractgate`) before inserting rows into a warehouse table. Rows that fail
`min`, `max`, `pattern`, `enum`, or `compliance_mode` stay out of the table.
DataContractGate quarantines them. Replay with `POST /quarantine/replay` after
the contract or the extract is fixed, then insert only the rows that pass.

`gate.py` is the runnable core. Dagster and Prefect call the same function.
The file does not import either orchestrator, so `python gate.py --self-check`
needs only the standard library. Install `contractgate` and `duckdb` for a
live run.

```bash
python gate.py --self-check

export CONTRACTGATE_API_KEY=cg_live_...
export CONTRACTGATE_CONTRACT_ID=<deployed uuid>
python -c "
from gate import open_client, run_batch
run_batch(open_client(), database='warehouse.duckdb')
"
```

MotherDuck is the same call with `database='md:my_db'` (DuckDB's MotherDuck
URL). Local DuckDB is a file path. The self-check uses sqlite when DuckDB is
not installed, and checks that rejected rows are absent from the table.

## Dagster

```python
import os
from dagster import asset
from gate import dagster_asset_body, open_client

@asset
def orders():
    return dagster_asset_body(
        records=load_records(),  # the extract this asset already owns
        client=open_client(),
        contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
        database="md:analytics",
    )
```

## Prefect

```python
import os
from prefect import flow, task
from gate import gate_batch, materialize, open_client

@task
def gated_orders(records: list[dict]) -> list[dict]:
    gated = gate_batch(
        records,
        client=open_client(),
        contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
    )
    return gated["accepted"]

@flow
def orders_flow(records: list[dict]):
    accepted = gated_orders(records)
    materialize(accepted, database="warehouse.duckdb", table="orders")
```

Contract: [`contract.yaml`](contract.yaml).

Micro-batches use the same gate. `run_batch(..., batch_size=500)` slices the
extract so each `client.ingest` stays under the 1 000-event cap on
`POST /v1/ingest/{contract_id}`.
