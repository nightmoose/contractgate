# Request timeouts

**Added:** 2026-10-16

A request that waits on the database returns **504** instead of hanging.
The server log line for that 504 names the **route** and the **stage**.

## Stages

| Stage | Meaning |
| --- | --- |
| `pool_acquire` | Checkout from the pool, including the ping of an idle connection |
| `lock_wait` | Postgres `lock_timeout` (`55P03`) |
| `db_statement` | Postgres `statement_timeout` (`57014`), or the request budget expired while a statement was in flight |
| `handler` | The request budget expired before a connection was checked out |

## Limits

| Limit | Value |
| --- | --- |
| Request budget | 30s. Response is `{"error":"request timed out during <stage>","status":504,"stage":"<stage>"}` |
| Pool acquire | 8s. Response is 504 `timed out acquiring a database connection` |
| Idle-connection ping | 2s, then the connection is discarded |
| Idle connection lifetime | 60s |
| `lock_timeout` | 5s. Response is 504 `timed out waiting for a database lock` |
| `statement_timeout` | 15s. Response is 504 `database statement timed out` |
| `idle_in_transaction_session_timeout` | 15s |

`GET /demo/stream` and `GET /demo/events` are not under the 30s budget (SSE).
Every other route is, including `POST /contracts/{id}/versions/{version}/promote`.

A timed-out promote applies nothing. Retrying is safe: promote is one
conditional `UPDATE` (`WHERE state = 'draft'`).
