# Chunk 4 — Observability Quick Wins

**Theme:** Five small items. Ship together. Self-host parity with managed.
**Why now:** Each is independent and small. Bundling avoids context-switch cost.

## Items

- [ ] Prometheus `/metrics` endpoint `[S]` — request counts, p50/p95/p99 validation latency, violation rate, contract counts.
- [ ] Grafana dashboard JSON template `[S]` — checked into repo, imports cleanly.
- [ ] Webhook alert delivery `[S]` — Slack incoming-webhook, PagerDuty events v2, generic HTTP POST.
- [ ] Email digest for contract health `[S]` — daily/weekly summary per org.
- [ ] Alerting rule engine `[M]` — thresholds (violation rate, schema drift, SLA breach) → fires the webhook/email above.

## Deferred to Chunk 5 / later

- Real-time health dashboard `[L]` — full UI build, separate effort.
- Impact visualization graph `[L]` — depends on consumer/producer telemetry not yet collected.
- Anomaly detection on violation streams `[L]` — needs Chunk 4 data flowing first.

## Surface to reuse

- Existing audit logging for violation counts.
- Validation engine timing (must remain <15ms p99 — measure, don't block).

## Open questions for the conversation

1. Metrics library: `metrics` crate + `metrics-exporter-prometheus`, or hand-rolled? Former is idiomatic.
2. Email transport: SMTP env config, or pluggable provider (SES, SendGrid, Postmark)?
3. Rule engine config: YAML in DB, Rego, or simple per-contract knobs? Start simple.
4. Default alert thresholds — what's a sensible violation-rate page-the-team number?
5. RFC required for the rule engine; the four `[S]` items are probably skip-RFC.

## Suggested first step

Land `/metrics` + Grafana JSON in one PR. Validate p99 budget unaffected. Then webhooks → email → rule engine.
