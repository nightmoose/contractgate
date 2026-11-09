# Per-run quarantine

An orchestrator can stamp one pipeline run on ingest and egress, then ask how many of that run's rows are still quarantined. dbt passes `invocation_id`. Airflow passes `run_id`. The id is optional: a call that omits it behaves as before, and the row's `batch_id` is null.

## Sending the id

`POST /v1/ingest/{contract_id}`, `POST /ingest/{contract_id}`, and `POST /egress/{contract_id}` accept the same optional id. The header is the form to send.

| Spelling | Where |
|---|---|
| `X-Batch-Id` | Header. Preferred. |
| `X-Run-Id` | Header alias. |
| `batch_id` | Query alias. |
| `run_id` | Query alias. |

When more than one is set, they must be the same string or the gateway returns **400**. A blank value is ignored. The id is at most 255 characters and must not contain ASCII control characters.

Prefer the header. Airflow's default `run_id` looks like `scheduled__2026-11-09T00:00:00+00:00`. In a query string, `+` is a space unless the client percent-encodes it as `%2B`. `requests` does that encoding for `params=`. A header keeps the `+`.

`Idempotency-Key` is a different header. It still returns the cached response for a repeated body and does not restamp `batch_id`. A dry run validates the id and stores nothing. Drop mode (`on_fail=drop`) stores no quarantine row, so there is nothing to count. Replay does not copy the id onto a new quarantine row.

```bash
curl -s -X POST "https://api.datacontractgate.com/v1/ingest/$CONTRACT_ID" \
  -H "X-Api-Key: $CONTRACTGATE_API_KEY" \
  -H "X-Batch-Id: $RUN_ID" \
  -H "Content-Type: application/json" \
  -d '{"user_id":"u_1"}'
```

The same header on `POST /egress/{contract_id}` stamps egress rows with `direction = egress`.

## Counting the run

```
GET /quarantine/summary?contract_id={uuid}&batch_id={run_id}
```

Auth is `X-Api-Key`, same as `GET /quarantine`. `run_id` is an alias of `batch_id`. `X-Batch-Id` and `X-Run-Id` are accepted on this request too, and must match the query when both are set.

`contract_id` is optional. Together with `batch_id` it is the indexed lookup (`quarantine_events (contract_id, batch_id, status, direction)` where `batch_id` is not null). Omit `batch_id` to count every source row for that contract, or for the whole org when `contract_id` is omitted too. A `contract_id` outside the caller's org is **404**.

The count includes source rows only. A row created by a failed replay is excluded, the same way `GET /quarantine` excludes it. Soft-deleted contracts are excluded.

```json
{
  "contract_id": "11111111-1111-1111-1111-111111111111",
  "batch_id": "scheduled__2026-11-09T00:00:00+00:00",
  "by_status": { "pending": 2, "reviewed": 0, "replayed": 0, "purged": 0 },
  "by_direction": { "ingress": 1, "egress": 1 },
  "by_status_and_direction": [
    { "status": "pending", "direction": "ingress", "count": 1 },
    { "status": "pending", "direction": "egress", "count": 1 }
  ],
  "total": 2,
  "pending": 2
}
```

`pending` is `by_status.pending`. Fail the task when `pending` is above zero. The four statuses and both directions are present even when the count is zero. `by_status_and_direction` always includes those eight cells; the example above shows the two cells that are non-zero for that run.

`GET /quarantine` returns `batch_id` on each row (null when the caller omitted it) and accepts `batch_id` or `run_id` to list one run.

## Self-host SQL

```sql
SELECT status, direction, count(*)
FROM quarantine_events
WHERE contract_id = :contract_id
  AND batch_id = :batch_id
  AND replay_of_quarantine_id IS NULL
GROUP BY status, direction;
```

Rows written before migration `049_quarantine_batch_id.sql`, and calls that omit the id, have `batch_id` null. Those rows are invisible to a batch filter. Use the time window for them:

```sql
SELECT count(*) AS pending_count
FROM quarantine_events
WHERE status = 'pending'
  AND contract_id = :contract_id
  AND created_at >= :window_start
  AND replay_of_quarantine_id IS NULL;
```

## Orchestrators

- dbt (self-host SQL, `invocation_id`, time window as a fallback): [`examples/dbt/`](../examples/dbt/)
- Airflow (HTTP summary, `run_id`, time window as a fallback): [`examples/airflow/`](../examples/airflow/)

See also [`docs/architecture/where-to-enforce-contracts.md`](architecture/where-to-enforce-contracts.md).
