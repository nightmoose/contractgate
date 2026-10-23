# DataContractGate ingestion recipes

Three reference programs for the ingest path. Each file is runnable without a live gateway:

```bash
python examples/recipes/webhook_quarantine_firewall.py --self-check
python examples/recipes/pipeline_guardrail.py --self-check
python examples/recipes/quarantine_replay.py --self-check
```

Set `CONTRACTGATE_API_KEY` and `CONTRACTGATE_CONTRACT_ID` and run the file with no arguments to call `https://api.datacontractgate.com`. The API key is sent as `X-Api-Key`. Do not commit it.

| Recipe | Gateway calls | What you copy |
|---|---|---|
| [Webhook quarantine firewall](webhook_quarantine_firewall.py) | `POST /v1/ingest/{contract_id}` with `Idempotency-Key` | FastAPI or a worker in front of Hookdeck or Svix. Forward `transformed_event` only. Failures stay in quarantine. |
| [Dagster and Prefect guardrail](pipeline_guardrail.py) | `POST /v1/ingest/{contract_id}` via the Python SDK | Gate records before a DuckDB, MotherDuck, or Snowflake load. |
| [Quarantine inspect and replay](quarantine_replay.py) | `GET /quarantine`, `POST /contracts/{id}/quarantine/replay` | Group violations, fix the contract or the producer, replay the affected ids. |

AISchemaGate uses the same gateway for model output. Those guides are under [`docs/integrations/`](../../docs/integrations/).
