# Quarantine by plan

**RFC:** 110

DataContractGate and AISchemaGate share one pending quarantine cap on Free. Paid plans and self-host have no cap. The numbers live in `src/plan.rs` (`FREE_QUARANTINE_ORG_CAP` 50,000, `FREE_QUARANTINE_CONTRACT_CAP` 10,000).

Pending rows are `status = 'pending'`. Reviewed, replayed, and purged rows do not count. `POST /contracts/{id}/quarantine/review` marks rows reviewed and frees that space immediately.

## When the cap is full

New failures are counted in the drop ledger with `on_fail_source: plan_cap`. They are not stored. Passing records keep flowing. The response includes `quarantine_cap`:

```json
{ "limit": 50000, "pending": 50000, "state": "reached", "scope": "org" }
```

`state` is `ok`, `near` (80% of a cap), or `reached`. `GET /usage` adds `quarantine.pending`, `quarantine.cap`, `quarantine.contract_cap`, and `quarantine.state`. `GET /contracts/{id}/quarantine/summary` adds the contract count.

Precedence is in `docs/drop-mode-reference.md`. An enforcing safety cap still wins, including when `on_fail` is locked. A request cannot force quarantine past the plan cap. `dry_run` writes nothing. An atomic rejection stores no quarantine rows and does not consume the cap.

## Replay

`POST /contracts/{id}/quarantine/replay` and `POST /quarantine/replay` require Pro, AISchemaGate Pro, Enterprise, or self-host. Otherwise the gateway returns 403:

```json
{
  "error": "plan_required",
  "message": "Replay is included with Pro. Quarantined records on Free keep the failure details without the record body.",
  "required_plan": "pro",
  "current_plan": "free",
  "feature": "quarantine_replay",
  "upgrade_url": "https://app.datacontractgate.com/account#plan"
}
```

`feature` is `quarantine_replay` or `bulk_replay`. List items include `replayable` and `replay_unavailable_reason` (`plan`, `body_not_stored`, `purged`, or null).

## Bulk replay

`POST /contracts/{id}/quarantine/replay-bulk` starts a background job and returns 202. Optional filters: `target_version`, `contract_version`, `reason` (violation kind). The job replays stored bodies in batches of 1,000, up to `BULK_REPLAY_JOB_LIMIT`. Set `CONTRACTGATE_BULK_REPLAY_JOB_LIMIT` to a positive integer to change that ceiling. A value that is not a positive integer is ignored.

`GET /contracts/{id}/quarantine/replay-jobs/{job_id}` returns `status` (`queued`, `running`, `completed`, `failed`) and the counts. The job error text does not include record bodies.

## CSV

`GET /contracts/{id}/quarantine/export` returns `text/csv` for every plan. Columns: `quarantined_at`, `contract_version`, `reason`, `field`, `status`. No record body.

## Retention and downgrade

Free keeps rows 7 days, Pro 90, Enterprise 365. Migration 046 redacts leftover Free bodies where storage was never turned on, and keeps the rows. A paid-to-free change sets `orgs.payload_redact_after` to 30 days later. The hourly purge then redacts quarantine, audit, and forwarded bodies and keeps the rows. Paying again before that clears the schedule.

The AISchemaGate repair loop uses the in-memory model output, so it works on every plan. Replay of a stored output is Pro.

## Hosted streams

Hosted Kafka and Kinesis consumers use the same on-fail decision as HTTP ingest. They do not insert `quarantine_events`. When the decision is quarantine, the failure is published to the quarantine topic. When the decision is drop — including `on_fail_source: plan_cap` at the Free cap — the failure is counted in `dropped_event_counts` and is not published. Passing records still go to the clean topic or stream. Paid plans have no cap, so a quarantine decision on a paid plan is unchanged.

## Follow-ups

Pricing pages in the website repositories are not in this repo.
