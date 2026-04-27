# RFC-009: Observability Quick Wins

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 04 — Observability Quick Wins                                |
| Depends on    | RFC-002 (versioning), RFC-008 (`severity` for alert rule conditions)   |

## Summary

Five items, four small, one medium. Ships self-host parity with managed:

1. **Prometheus `/metrics`** — request counts, validation latency histograms,
   violation rate, contract counts.
2. **Grafana dashboard JSON** — checked into repo, imports cleanly into any
   Grafana ≥9.0.
3. **Webhook alert delivery** — Slack incoming-webhook, PagerDuty Events v2,
   generic HTTP POST.
4. **Email digest** — daily/weekly contract health summaries.
5. **Alerting rule engine** — thresholds (violation rate, schema drift, SLA
   breach) → fires the webhook/email above.

The first four are `[S]`. The rule engine is `[M]` and consumes the others.

## Goals

1. `/metrics` adds <1ms p99 to no hot-path request. Metrics are written via
   the `metrics` crate's recorder, scraped by the exporter on demand.
2. Grafana JSON template (`ops/grafana/contractgate.json`) imports without
   manual variable fixups; assumes a Prometheus datasource named `prometheus`.
3. Webhook delivery has at-least-once semantics with idempotency key in body.
4. Rule engine evaluates rules every 60s on a Tokio interval; rule storage
   in Postgres `alert_rules` table.
5. All alerting paths respect a per-org cooldown so we don't pager-storm.

## Non-goals

- Real-time health dashboard `[L]` — separate effort, post-Chunk 4.
- Impact visualization graph — depends on consumer telemetry not yet collected.
- Anomaly detection on violation streams — needs this chunk's data flowing
  first; future RFC.
- OpenTelemetry traces — metrics-only in v1. Tracing is a separate RFC.
- StatsD / DogStatsD output — Prometheus only.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Metrics library | **`metrics` + `metrics-exporter-prometheus` crates.** Idiomatic, low-overhead, no global state surprises. |
| Q2 | Histogram buckets | **Hand-tuned: 0.001, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 5 (seconds).** Tight under 25ms (the validation budget), wide above. |
| Q3 | `/metrics` auth | **Open by default; `METRICS_AUTH_TOKEN` env opts into bearer-token auth.** Self-host needs scrape; managed flips on auth. |
| Q4 | Webhook reliability | **One async retry queue per webhook target**, exponential backoff (1s, 5s, 25s, drop). At-least-once. |
| Q5 | Webhook secret signing | **HMAC-SHA256 over body, header `X-ContractGate-Signature: sha256=...`** Same shape GitHub uses; familiar. |
| Q6 | Email transport | **SMTP via `lettre`**, configured by `SMTP_*` env. Pluggable provider via trait, but only SMTP impl ships in v1. |
| Q7 | Email digest cadence | **Daily 09:00 UTC + weekly Monday 09:00 UTC**, per-contract opt-in stored on the contract. |
| Q8 | Rule storage | **`alert_rules` table in Postgres** with JSONB condition spec; no Rego. |
| Q9 | Rule evaluation interval | **60s, configurable via `ALERT_EVAL_INTERVAL_SECS` env.** |
| Q10 | Cooldown | **Per-rule, per-target, default 300s.** Configurable on the rule. |

## Current state

- No metrics endpoint. Validation timing is logged but not aggregated.
- No alerting at all today.
- `audit_log.outcome` already carries pass/fail/quarantined — sufficient for
  violation-rate metrics without schema changes.

## Design

### Metrics surface (`/metrics`)

| Metric | Type | Labels | Source |
|---|---|---|---|
| `contractgate_requests_total` | counter | `route`, `method`, `status` | axum middleware |
| `contractgate_validation_duration_seconds` | histogram | `contract_id`, `outcome` | wrap `validate()` |
| `contractgate_violations_total` | counter | `contract_id`, `kind` | violation emit site |
| `contractgate_quarantined_total` | counter | `contract_id` | ingest path |
| `contractgate_contracts_active` | gauge | (none) | refreshed every 30s |
| `contractgate_audit_log_rows` | gauge | (none) | refreshed every 60s |

Implementation: middleware wraps every route, increments counter; validation
fn wrapped in `metrics::histogram!()`. Existing audit_log writes are hooked
without schema change.

### Grafana dashboard (`ops/grafana/contractgate.json`)

Panels (top-to-bottom):
1. **Request rate** — `sum(rate(contractgate_requests_total[1m]))` by status.
2. **Validation p50/p95/p99** — `histogram_quantile(...)` over duration.
3. **Violation rate** — sum of `violations_total` rate, by `kind`.
4. **Quarantine rate** — `quarantined_total` rate.
5. **Active contracts** — gauge.
6. **Top noisy contracts** — `topk(10, rate(violations_total[5m]))` by contract.

Datasource variable assumes `prometheus` UID. Dashboard JSON committed at
`ops/grafana/contractgate.json` and rendered as a snapshot screenshot in the
README.

### Webhook delivery (`src/alert_webhook.rs`)

```rust
pub enum WebhookTarget {
    Slack { webhook_url: String },
    PagerDuty { routing_key: String },
    Generic { url: String, secret: Option<String> },
}

pub struct WebhookEvent {
    pub idempotency_key: Uuid,
    pub rule_id: Uuid,
    pub fired_at: DateTime<Utc>,
    pub kind: AlertKind,
    pub message: String,
    pub context: serde_json::Value,
}
```

Each target has its own body shape:
- Slack: `{"text": "<message>"}`.
- PagerDuty: full Events v2 envelope (`routing_key`, `event_action: "trigger"`,
  `dedup_key: idempotency_key`, `payload`).
- Generic: the full `WebhookEvent` JSON, signed with HMAC-SHA256 if secret set.

Delivery via a per-target Tokio mpsc with retry queue: 1s, 5s, 25s; drop after
3 attempts and write a row to `alert_delivery_failures` for postmortem.

### Email digest (`src/alert_email.rs`)

- `lettre` SMTP client.
- Templated via `askama` (Jinja-like, compile-time-checked).
- Daily template: yesterday's pass/fail/quarantine counts per contract,
  top 5 violation kinds, link back to dashboard.
- Weekly: same shape, 7-day window, plus a delta-from-prior-week column.
- Opt-in stored on `contracts.alert_email_opt_in` (jsonb: `{daily: bool,
  weekly: bool, recipients: [...]}`).

### Alert rule engine

```sql
CREATE TABLE alert_rules (
  id          uuid PRIMARY KEY,
  contract_id uuid REFERENCES contracts(id),
  name        text NOT NULL,
  condition   jsonb NOT NULL,    -- {kind, threshold, window}
  targets     jsonb NOT NULL,    -- [WebhookTarget | EmailTarget]
  cooldown_seconds int NOT NULL DEFAULT 300,
  enabled     bool NOT NULL DEFAULT true,
  last_fired_at timestamptz
);
```

Condition spec (closed set in v1):

```json
{ "kind": "violation_rate", "threshold": 0.05, "window": "5m" }
{ "kind": "quarantine_rate", "threshold": 0.01, "window": "10m" }
{ "kind": "p99_latency", "threshold_ms": 25, "window": "5m" }
{ "kind": "breaking_diff_published" }   // fires once per publish, no window
```

Evaluation loop (`src/alert_engine.rs`):

```rust
loop {
    sleep(Duration::from_secs(ALERT_EVAL_INTERVAL_SECS)).await;
    for rule in load_enabled_rules().await? {
        if rule.in_cooldown() { continue; }
        if rule.condition_met(metrics_snapshot, audit_db).await? {
            dispatch_to_targets(&rule).await?;
            mark_fired(rule.id).await?;
        }
    }
}
```

`breaking_diff_published` is fired event-driven from the diff handler
(RFC-008), not on the eval loop — it short-circuits cooldown only on first
publish per `(contract_id, version)` pair.

## Test plan

- `tests/metrics.rs` — hit `/metrics`, assert all expected metric names
  present; spike traffic and assert counters move.
- `tests/webhook_delivery.rs` — mock target server, assert idempotency key
  echoed, retry on 500, drop on 4xx.
- `tests/alert_rules.rs` — seed audit_log to violate threshold, run one eval
  tick, assert dispatch happened.
- Email: dry-run test prints rendered HTML to stdout, snapshot via insta.
- Grafana JSON: `jq` lint in CI to ensure valid JSON + required fields.

## Rollout

1. Sign-off this RFC.
2. `metrics` crate wiring + `/metrics` endpoint + middleware. Verify <15ms
   p99 budget unaffected via load test.
3. Grafana JSON committed; README snapshot updated.
4. Webhook target trait + Slack + PD + Generic impls.
5. Email digest (daily first, weekly later in same chunk).
6. `alert_rules` schema migration + eval loop + condition kinds.
7. Dashboard "Alerts" tab — list rules, toggle enabled, view last fire.
8. `cargo check && cargo test`.
9. Update `MAINTENANCE_LOG.md`.

## Deferred

- OpenTelemetry traces.
- Real-time health dashboard.
- Anomaly detection.
- StatsD/Datadog direct push.
