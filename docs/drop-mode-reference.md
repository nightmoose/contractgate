# Drop mode

Quarantine is the default for DataContractGate and AISchemaGate. A failing record is stored for review and replay.

Drop mode counts the failure and stores nothing: no `quarantine_events` row and no `audit_log` row for that record. Passing records are still audited and forwarded. `dry_run=true` still writes nothing at all, including the drop ledger.

Drop mode is available on every plan. It is not the AISchemaGate default. Egress stays on quarantine unless the contract or the request selects drop.

## Contract document

```yaml
on_fail: quarantine          # quarantine (default) | drop
on_fail_locked: false        # when true, a request cannot override on_fail
quarantine_guard:            # optional; omit to leave the cap unset
  enforce: false             # false = watch only
  # max_per_minute: <positive integer>   omit a window to leave it unlimited
  # max_per_hour: <positive integer>
  # cooldown_minutes: <positive integer> omit for manual reset only
  sample_drops: false        # up to 5 metadata samples per minute
```

The gateway does not choose threshold values. Set the ones you want, or leave them unset.

A live override on the contract (`contracts.fail_policy`) wins over the version document. The dashboard writes that override, including on a stable version. Publishing a version that sets `on_fail` does not by itself clear an override already saved from the dashboard.

## Request override

`?on_fail=quarantine|drop` on `POST /ingest/{id}`, `POST /v1/ingest/{id}`, and `POST /egress/{id}`.

The `X-On-Fail` header carries the same values. If the query and the header disagree, the gateway returns 400.

Precedence, highest first:

1. `dry_run=true` writes nothing.
2. An enforcing safety cap that is tripped forces drop, including when `on_fail` is locked. A request cannot choose quarantine while it is tripped.
3. The Free quarantine cap, when it is full (`on_fail_source: plan_cap`). A request cannot force quarantine past that cap. DataContractGate and AISchemaGate share it. See `docs/quarantine-plan-reference.md`.
4. `on_fail_locked` rejects a request whose value differs from the contract (400). A request that omits `on_fail`, or repeats the locked value, is accepted.
5. The request value.
6. The contract value, or the live override when one is saved.
7. Default `quarantine`.

Hosted Kafka and Kinesis consumers use this order. They have no request
override. A drop is counted and is not published to the quarantine topic or
stream. A quarantine decision is published there and is not inserted into
`quarantine_events`.

`atomic=true` with drop still rejects the whole batch. The failures are counted as dropped and nothing is quarantined. An atomic rejection does not advance the quarantine volume counter, because no quarantine rows are stored.

A pin to a deprecated version follows the same mode.

## Response

Ingest and egress responses add:

| Field | Meaning |
|---|---|
| `quarantined` | Failures stored in `quarantine_events`. |
| `dropped` | Failures counted and not stored. |
| `on_fail_mode` | `quarantine` or `drop`. |
| `on_fail_source` | `request`, `contract`, `default`, `auto_trip`, or `plan_cap`. |
| `quarantine_cap` | Present on Free: `limit`, `pending`, `state` (`ok`, `near`, `reached`), and `scope`. |
| `auto_trip` | Present when a cap is enforcing or would have tripped. |

On a dry run, `quarantined` and `dropped` are both 0. Otherwise `failed = quarantined + dropped`.

Failing per-record results include `disposition`: `quarantined` or `dropped`. On egress, `action` is still the block / fail / tag payload decision. `disposition` is only about storage.

`auto_trip` looks like:

```json
{
  "active": false,
  "would_trip": true,
  "enforcing": false,
  "scope": "account"
}
```

`until` is set when an enforcing trip has a cooldown. `scope` is `account` or `contract`.

## Safety cap

The cap counts quarantine inserts per minute and per hour. It does not count records that were already dropped.

It ships watch-only (`enforce: false`). Crossing a threshold records a would-trip event and the `contractgate_quarantine_guard_would_trip_total` metric. The on-fail mode does not change, and failing records are still quarantined.

Set `enforce: true` to switch the winning scope to drop until the cooldown elapses or an owner resets it. A request cannot force quarantine during that trip, including when the contract locks `on_fail`. Each new trip increments `trip_count`.

An owner reset clears the trip immediately. If that same minute or hour is still at the threshold, the next request starts a new trip. Records that were already dropped are not part of the threshold, so a reset during an active trip usually stays clear.

The account cap replaces the contract cap when the account has at least one positive threshold. Reset either scope with:

- `POST /contracts/{id}/quarantine-guard/reset`
- `POST /settings/quarantine-guard/reset`

`GET` and `PATCH /contracts/{id}/fail-policy` read and save the contract setting. `GET` and `PATCH /settings/quarantine-guard` read and save the account cap. Clearing the account cap (JSON `null`) lets the contract cap apply again.

A threshold of 0 is rejected. An omitted threshold is not a limit.

## Drop ledger

`dropped_event_counts` stores one row per minute, contract version, direction (`ingress` or `egress`), source, and reason. The reason is the violation kind and field path, never a value from the record.

Rows are kept for 30 days. `purge_expired_logs` deletes older buckets along with audit and quarantine retention.

`GET /contracts/{id}/drop-counts?hours=24` returns the summed reasons. `hours` defaults to 24 and is capped at 720.

`sample_drops: true` keeps up to 5 samples per contract version, direction, and minute. Each sample is `{ "violations": [{ "kind", "field" }] }`. Record bodies are not stored.

## Metrics

- `contractgate_dropped_total{reason,source}` — `reason` is the violation kind.
- `contractgate_quarantine_guard_would_trip_total{scope}`
- `contractgate_quarantine_guard_trip_total{scope}`
- `contractgate_quarantine_guard_tripped{scope,contract_id}` — 1 while an enforcing trip is the effective mode.

## SDKs and MCP

Python `ingest` and `egress` take `on_fail="quarantine"` or `"drop"`. The npm client takes `onFail`. AISchemaGate `LLMGate`, `aischemagateGuardrail`, and the Python framework adapters take the same option and leave it unset by default, which keeps quarantine.

The MCP tools `validate_events` and `egress_validate` accept `on_fail`. `get_drop_counts` reads the ledger.

## Follow-ups

These surfaces still quarantine failures and do not read `on_fail` yet:

- The Zapier Validate Record action and a `quarantine_guard_tripped` trigger.
- The Kafka Connect single-message transform (`on.fail` is not a config key yet).
- Hosted Kafka and Kinesis consumers.
