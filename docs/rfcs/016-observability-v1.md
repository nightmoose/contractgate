# RFC-016: Observability v1 (Metrics + Slack Alerts)

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #3                                                        |
| Supersedes    | RFC-009 (broader scope; PD/email/generic webhooks + full rule engine deferred) |

## Summary

Three deliverables, demo-ready:

1. **Prometheus `/metrics`** — request counts, validation latency
   histograms, violation rate, contract counts.
2. **Grafana dashboard JSON** — checked into repo, imports cleanly.
3. **Slack webhook alerts** — one target type. Single rule kind:
   `violation_rate > threshold` over a window. No engine, just a row.

PagerDuty, generic HTTP, email digest, and the multi-kind rule engine
are deferred until users actually ask.

## Goals

1. `/metrics` adds <1ms p99 to no hot-path request.
2. Grafana JSON imports without manual fixups; assumes Prometheus
   datasource named `prometheus`.
3. Slack alerts fire at-most-once per (rule, window) — cooldown enforced.
4. Validation engine p99 budget unaffected (<15ms).

## Non-goals

- PagerDuty / generic webhook targets.
- Email digest.
- Multi-kind rule engine (p99 latency rule, quarantine rate rule,
  breaking-diff rule). Just `violation_rate` in v1.
- Real-time health dashboard `[L]`.
- OpenTelemetry traces.
- Anomaly detection.

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Metrics library | **`metrics` + `metrics-exporter-prometheus` crates.** |
| Q2 | Histogram buckets | **0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 5 (sec).** Tight under 25ms validation budget. |
| Q3 | `/metrics` auth | **Open by default; `METRICS_AUTH_TOKEN` env opts into bearer auth.** |
| Q4 | Slack webhook config | **Per-contract**, stored in `contracts.alert_slack_webhook_url`. Not a separate table in v1. |
| Q5 | Rule storage | **`alert_rules` table, one rule kind only.** Single-row spec keeps schema sane for future expansion. |
| Q6 | Eval cadence | **60s**, configurable via `ALERT_EVAL_INTERVAL_SECS`. |
| Q7 | Cooldown | **300s per rule.** Hard-coded in v1; configurable later. |
| Q8 | Webhook signing | **HMAC-SHA256 over body, header `X-ContractGate-Signature`.** Same shape GitHub uses. |

## Metrics surface

| Metric | Type | Labels |
|---|---|---|
| `contractgate_requests_total` | counter | `route`, `method`, `status` |
| `contractgate_validation_duration_seconds` | histogram | `contract_id`, `outcome` |
| `contractgate_violations_total` | counter | `contract_id`, `kind` |
| `contractgate_quarantined_total` | counter | `contract_id` |
| `contractgate_contracts_active` | gauge | (none, refreshed every 30s) |
| `contractgate_audit_log_rows` | gauge | (none, refreshed every 60s) |

Implementation: axum middleware wraps every route → `requests_total`.
Validation fn wrapped via `metrics::histogram!()`. Existing audit_log
write path emits `violations_total` and `quarantined_total` without
schema change.

## Grafana dashboard

`ops/grafana/contractgate.json`. Panels:

1. Request rate by status.
2. Validation p50 / p95 / p99 (`histogram_quantile`).
3. Violation rate by kind.
4. Quarantine rate.
5. Active contracts (gauge).
6. Top 10 noisiest contracts (`topk` over `violations_total` rate).

Dashboard JSON committed; readme has a screenshot.

## Slack alerts

### Schema

```sql
CREATE TABLE alert_rules (
  id            uuid PRIMARY KEY,
  contract_id   uuid REFERENCES contracts(id) ON DELETE CASCADE,
  name          text NOT NULL,
  kind          text NOT NULL CHECK (kind = 'violation_rate'),  -- only kind in v1
  threshold     float NOT NULL,        -- e.g. 0.05 for 5%
  window_secs   int NOT NULL DEFAULT 300,
  webhook_url   text NOT NULL,         -- Slack incoming webhook
  enabled       bool NOT NULL DEFAULT true,
  last_fired_at timestamptz
);
```

### Eval loop (`src/alert_engine.rs`)

```rust
loop {
    sleep(Duration::from_secs(ALERT_EVAL_INTERVAL_SECS)).await;
    for rule in load_enabled_rules().await? {
        if rule.in_cooldown(300) { continue; }
        let rate = violation_rate_for(rule.contract_id, rule.window_secs).await?;
        if rate > rule.threshold {
            post_slack(&rule, rate).await?;
            mark_fired(rule.id).await?;
        }
    }
}
```

`violation_rate_for` runs:

```sql
SELECT
  count(*) FILTER (WHERE outcome = 'fail')::float
  / nullif(count(*), 0)
FROM audit_log
WHERE contract_id = $1
  AND created_at >= now() - ($2 || ' seconds')::interval;
```

### Slack payload

```json
{ "text": "ContractGate: <contract_name> violation rate <rate> > <threshold> (last <window>)" }
```

No fancy blocks; Slack incoming webhook accepts plain text.

### Dashboard surface

Contract detail page gets an "Alerts" subtab listing rules for this
contract: name, threshold, window, last fired, enable toggle, edit/delete.
Add-rule form: name, threshold, window, Slack webhook URL.

## Test plan

- `tests/metrics.rs` — hit `/metrics`, assert all expected metric names
  present; spike traffic and assert counters move.
- `tests/alert_rules.rs` — seed audit_log to violate threshold, run one
  eval tick, assert Slack POST to mock server.
- `tests/alert_cooldown.rs` — fire rule, immediately re-eval, assert
  no second fire.
- Grafana JSON: `jq` lint in CI to ensure valid JSON + required fields.

## Rollout

1. Sign-off this RFC.
2. `metrics` crate wiring + `/metrics` endpoint + middleware.
3. Load test to confirm <15ms p99 budget held.
4. Grafana JSON committed; README snapshot.
5. `alert_rules` migration.
6. Eval loop + Slack POST + cooldown.
7. Dashboard "Alerts" subtab on contract detail page.
8. `cargo check && cargo test`.
9. Update `MAINTENANCE_LOG.md`.

## Deferred

- PagerDuty + generic webhook targets.
- Email digest.
- Additional rule kinds (p99 latency, quarantine rate, breaking-diff).
- OpenTelemetry traces.
- Anomaly detection.
- Real-time health dashboard.
