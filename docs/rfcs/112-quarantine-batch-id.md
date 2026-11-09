# RFC-112 — Per-run quarantine

**Status:** Accepted
**Date:** 2026-11-09

## Problem

`quarantine_events` has no run identifier. dbt and Airflow can only treat "pending rows for this contract since the window started" as the signal that a pipeline run held data back. A later row, or a row from an overlapping run, is indistinguishable from this run's failures. `Idempotency-Key` is not stored on the quarantine row.

## Decision

Callers may pass an optional run id on `POST /v1/ingest/{contract_id}`, `POST /ingest/{contract_id}`, and `POST /egress/{contract_id}`.

The preferred spelling is the `X-Batch-Id` header. `X-Run-Id`, and the query parameters `batch_id` and `run_id`, are aliases. Every non-blank spelling on one request must be the same string. The value is trimmed, at most 255 characters, and must not contain ASCII control characters. The header is preferred because an Airflow `run_id` contains `+`, which a query string decodes as a space unless the client percent-encodes it.

The id is stored on `quarantine_events.batch_id` and indexed by `(contract_id, batch_id, status, direction)` where `batch_id` is not null. Omitting it leaves the column null. Existing callers are unchanged. Replay-created rows do not copy the id. Dry run and drop mode still write no quarantine row.

`GET /quarantine/summary?contract_id=&batch_id=` returns counts by status and by direction, the status×direction cells, `total`, and `pending` (`by_status.pending`). An orchestrator fails the task when `pending` is above zero. The count is source rows only, scoped to the caller's org, and excludes soft-deleted contracts. `contract_id` outside that org is 404. `GET /quarantine` returns `batch_id` and can filter on it.

dbt's example test filters on `invocation_id` (or `dcg_quarantine_batch_id`). Airflow's example sends and reads `run_id`. Both keep the time-window check for rows whose `batch_id` is null.

Migration `049_quarantine_batch_id.sql` adds the column, a length check, and the index.

## Follow-ups

Hosted Kafka and Kinesis consumers do not insert `quarantine_events`. They are out of scope. The dashboard quarantine tab receives `batch_id` on each row and does not filter by it yet.
