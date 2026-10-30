# RFC-110 — Quarantine by plan

**Status:** Accepted
**Date:** 2026-10-30

## Problem

Free quarantine has no row cap. Replay has no explicit plan check. A Pro to Free downgrade can leave stored record bodies in place. Bulk replay does not exist.

## Decision

DataContractGate and AISchemaGate share one pending-row cap. Placeholders in `src/plan.rs`:

- 50,000 pending rows per organization
- 10,000 pending rows per contract
- Paid plans and self-host have no cap
- Pending means `status = 'pending'`. Marking a row reviewed frees that space immediately. There is no lower watermark.

When a Free cap is full, new failures use the counted drop path with `on_fail_source: plan_cap`. Passing records are unchanged. Ingest does not return an error because quarantine is full.

Precedence, highest first: `dry_run`, an enforcing safety-cap trip (including over a locked `on_fail`), the plan cap, the lock, the request, the contract, then the default. An atomic rejection stores no quarantine rows and does not advance the safety-cap volume counter or the plan cap.

Replay and bulk replay return HTTP 403 with `error: plan_required`. Bulk replay is Pro, runs as a background job, and stops at `BULK_REPLAY_JOB_LIMIT` (the per-contract placeholder, overridable with `CONTRACTGATE_BULK_REPLAY_JOB_LIMIT`).

Free can export a CSV of failure time, version, reason, and field. The CSV has no record body.

Migration 046 redacts leftover Free bodies where storage was never enabled, and keeps the rows. No notice is sent. A later paid-to-free change waits 30 days, then redacts bodies the same way. Upgrading before that clears the schedule. Self-hosted organizations that enabled storage are not part of the one-time redaction.

The client-side AISchemaGate repair loop uses the in-memory output and works on every plan.

## Follow-ups

Pricing pages in `datacontractgate_website` and `aischemagate-www` are outside this repository. Hosted Kafka and Kinesis consumers still publish failures to a quarantine topic and do not insert `quarantine_events`, so this cap does not apply to those topic routes.
