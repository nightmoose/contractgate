# RFC-016: Observability v1 (Metrics Only)

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Revised       | 2026-04-27 — scope trimmed: alerts deferred until first pilot has real traffic |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #3                                                        |
| Supersedes    | RFC-009 (broader scope; PD/email/generic webhooks + full rule engine deferred) |

## Summary

Two deliverables:

1. **Prometheus `/metrics`** — request counts, validation latency
   histograms, violation rate, contract counts.
2. **Grafana dashboard JSON** — checked into repo, imports cleanly.

That's it. No alerts, no rule engine, no `alert_rules` table, no Slack /
PagerDuty / generic webhook / email targets. Reason: pre-customer there
is no live event stream; alerts would never fire on test data, and the
rule engine plumbing (~3 days) returns zero current value. Demo data
flow is handled by the seeder in RFC-017.

Alerts return as a follow-up RFC when the first pilot's traffic exists.

## Goals

1. `/metrics` adds <1ms p99 to no hot-path request.
2. Grafana JSON imports without manual fixups; assumes Prometheus
   datasource named `prometheus`.
3. Validation engine p99 budget unaffected (<15ms).
4. Dashboard panels show real-looking data when the RFC-017 demo seeder
   is running.

## Non-goals

- Slack / PagerDuty / generic webhook alert delivery.
- Email digest.
- Rule engine, `alert_rules` table, eval loop, cooldown logic.
- Real-time health dashboard `[L]`.
- OpenTelemetry traces.
- Anomaly detection.
- Per-contract metrics auth (open by default, env-var token opt-in).

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Metrics library | **`metrics` + `metrics-exporter-prometheus` crates.** |
| Q2 | Histogram buckets | **0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 5 (sec).** Tight under 25ms validation budget. |
| Q3 | `/metrics` auth | **Open by default; `METRICS_AUTH_TOKEN` env opts into bearer auth.** |
| Q4 | Endpoint mounting | **Public route** (no `x-api-key` requirement) so Prometheus scraping works without per-org credentials. Token-gated when `METRICS_AUTH_TOKEN` set. |
| Q5 | Refresh of gauges | **Background Tokio task on 30s/60s tickers** for `contracts_active` and `audit_log_rows`. |

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

Provisioning files at `ops/grafana/provisioning/dashboards/contractgate.yaml`
so RFC-017's Compose stack auto-imports the dashboard on boot.

## Test plan

- `tests/metrics.rs` — hit `/metrics`, assert all expected metric names
  present; spike traffic and assert counters move.
- Load test to confirm <15ms p99 validation budget held with metrics
  middleware in place.
- Grafana JSON: `jq` lint in CI to ensure valid JSON + required fields.

## Rollout

1. Sign-off this RFC.
2. `metrics` crate wiring + middleware + validation histogram instrumentation.
3. `/metrics` endpoint mounted on public router.
4. Background gauge-refresh tasks.
5. Load test → confirm p99 budget held.
6. Grafana JSON + provisioning files in `ops/grafana/`.
7. README snapshot of dashboard (rendered with RFC-017 seeder running).
8. `cargo check && cargo test`.
9. Update `MAINTENANCE_LOG.md`.

## Deferred

- Slack / PagerDuty / generic webhook alert targets.
- Email digest.
- Rule engine + `alert_rules` table + eval loop + cooldown.
- Additional rule kinds (p99 latency, quarantine rate, breaking-diff).
- OpenTelemetry traces.
- Anomaly detection.
- Real-time health dashboard.

Alerts return as a follow-up RFC once first pilot has continuous traffic
to design real thresholds against.
