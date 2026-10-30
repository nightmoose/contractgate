# RFC-109 — Drop mode for failing records

**Status:** Accepted
**Date:** 2026-10-30

## Problem

Every non-dry-run validation failure on ingest and egress writes a quarantine row, and a failing record also writes an audit row. A bad producer can fill both tables as fast as it can send. `dry_run` skips every write, including passes. `store_event_payloads` still writes a row per failure.

## Decision

- `on_fail` is `quarantine` (default) or `drop`. Drop writes neither a quarantine row nor an audit row for the failure. Passing records are unchanged.
- A request may override with `on_fail` or `X-On-Fail` unless the contract owner locks the setting.
- While an enforcing safety cap is tripped, a request cannot force quarantine.
- The cap ships watch-only. It records would-trip events and metrics and does not switch modes until `enforce` is true. Thresholds are configured by the caller. This RFC does not set numeric defaults.
- An account cap with a threshold overrides the contract cap. Either resets when its cooldown elapses or when an owner resets it.
- Drop counts are minute buckets by failure reason, kept 30 days, and are included in `purge_expired_logs`.
- An opt-in sample keeps at most 5 metadata rows per minute bucket. Samples hold violation kind and field path only.
- The setting is available on every plan. AISchemaGate keeps quarantine as the default and exposes drop as an option.

## Follow-ups

Zapier, the Kafka Connect transform, and the hosted Kafka and Kinesis consumers are unchanged. They still quarantine failures.
