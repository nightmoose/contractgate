# RFC-017: Onboarding Stack (Compose + Starter Templates + Demo Seeder)

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Revised       | 2026-04-27 — added demo seeder so RFC-016 metrics + RFC-015 audit search look real on a fresh boot |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #4                                                        |
| Supersedes    | RFC-010 partially (Compose only); RFC-012 partially (3 starters only)  |

## Summary

Three pieces, all pre-customer onboarding:

1. **Docker Compose reference stack** — `docker compose up` boots
   gateway + Postgres + Prometheus + Grafana in five minutes.
2. **Three starter contract templates** — `contracts/starters/*.yaml`
   checked into the repo. Copy-and-modify; no registry, no UI.
3. **Demo seeder binary** — `src/bin/demo-seeder.rs` posts realistic
   events through real `/ingest` so audit_log fills, metrics dashboards
   look populated, and the audit search UI (RFC-015) has rows to filter.
   Compose `--profile demo` runs it after gateway health-check passes.

No Helm chart, no RBAC editor, no air-gap bundle. No template registry,
ratings, submission pipeline, or private namespaces.

## Goals

1. `docker compose up` from a clean clone produces a working gateway,
   Prometheus scraping it, Grafana with the RFC-016 dashboard
   pre-imported, all on `localhost`.
2. The three starter YAMLs cover the most common pilot shapes:
   REST event, Kafka event, dbt model.
3. Each starter is a complete, valid contract that passes
   `contractgate validate` (RFC-014).
4. `docker compose --profile demo up` additionally publishes the three
   starter contracts to the gateway and runs the demo seeder for a
   default 5 minutes at 10 events/sec. After it finishes, audit_log
   has ~3000 rows, metrics dashboards are populated, and audit search
   is non-empty.

## Non-goals

- Bundled Postgres / Kafka via Helm subcharts.
- RBAC role editor UI.
- License-key flow, air-gap bundle.
- Template registry API, dashboard browser tab, ratings, imports counter,
  submission pipeline, private namespaces.
- Continuous traffic generation (seeder is one-shot, not a daemon).
- Replacing the existing `stream_demo` SSE module — that stays for the
  playground; this seeder is a separate code path that writes audit_log.

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Compose seed data | **Empty by default; `--profile demo`** publishes starters + runs the demo seeder. |
| Q2 | Compose Kafka | **Opt-in via `--profile kafka`.** |
| Q3 | Compose Prometheus + Grafana | **Bundled by default.** |
| Q4 | Starter location | **`contracts/starters/{rest_event,kafka_event,dbt_model}.yaml`** in the repo. No DB rows. |
| Q5 | Starter discovery | **Documented in `contracts/starters/README.md`** — copy-and-modify guide. |
| Q6 | Image source | **`ghcr.io/contractgate/{gateway,dashboard}:latest`** — built by existing release workflow. |
| Q7 | Demo seeder location | **`src/bin/demo-seeder.rs`** — separate binary in this crate, alongside existing `src/bin/demo.rs`. Reuses gateway's contract/event types. |
| Q8 | Demo seeder data shape | **Mix of pass / fail / quarantine** — 80/15/5 by default, configurable via flags. Realistic-looking values per starter. |
| Q9 | Demo seeder rate | **10 events/sec for 5 minutes by default** (≈3000 rows). Configurable via `--rate` and `--duration`. |
| Q10 | Demo seeder invocation | **One-shot Compose service** under `profiles: [demo]` that runs to completion and exits. |

## Compose stack

`docker-compose.yml`:

```yaml
services:
  gateway:
    image: ghcr.io/contractgate/gateway:${TAG:-latest}
    ports: ["8080:8080"]
    environment:
      DATABASE_URL: postgres://cg:cg@postgres/contractgate
      METRICS_AUTH_TOKEN: ""
    depends_on: [postgres]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 5s
      retries: 12

  dashboard:
    image: ghcr.io/contractgate/dashboard:${TAG:-latest}
    ports: ["3000:3000"]
    environment:
      NEXT_PUBLIC_API_BASE: http://localhost:8080
    depends_on: [gateway]

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: cg
      POSTGRES_PASSWORD: cg
      POSTGRES_DB: contractgate
    volumes: [pg_data:/var/lib/postgresql/data]

  prometheus:
    image: prom/prometheus:latest
    volumes:
      - ./ops/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml

  grafana:
    image: grafana/grafana:latest
    ports: ["3001:3000"]
    volumes:
      - ./ops/grafana:/etc/grafana/provisioning/dashboards

  demo-seeder:
    image: ghcr.io/contractgate/gateway:${TAG:-latest}
    profiles: [demo]
    command: ["/usr/local/bin/demo-seeder", "--rate", "10", "--duration", "300s"]
    environment:
      GATEWAY_URL: http://gateway:8080
      CONTRACTGATE_API_KEY: ${CONTRACTGATE_API_KEY:-cg_demo_key}
    depends_on:
      gateway: { condition: service_healthy }

  kafka:
    image: bitnami/kafka:3.7
    profiles: [kafka]
    environment:
      KAFKA_CFG_NODE_ID: 1
      KAFKA_CFG_PROCESS_ROLES: controller,broker

volumes:
  pg_data:
```

`make stack-up` wraps `docker compose up`. `make stack-up-demo`
wraps `docker compose --profile demo up`.

`ops/prometheus/prometheus.yml` scrapes `gateway:8080/metrics` (RFC-016).
`ops/grafana/provisioning/dashboards/contractgate.yaml` provisions the
RFC-016 dashboard.

## Demo seeder (`src/bin/demo-seeder.rs`)

Small Rust binary, ships in the same release image as the gateway.

### Flags

```
demo-seeder
  --gateway-url <url>     [default: env GATEWAY_URL or http://localhost:8080]
  --api-key <key>         [default: env CONTRACTGATE_API_KEY]
  --rate <events/sec>     [default: 10]
  --duration <duration>   [default: 5m]   accepts 30s, 5m, 1h
  --pass-pct <0..1>       [default: 0.80]
  --fail-pct <0..1>       [default: 0.15]
  --quarantine-pct <0..1> [default: 0.05]
  --contracts <list>      [default: rest_event,kafka_event,dbt_model]
```

### Behavior

1. On startup, ensure each contract in `--contracts` is published to
   the gateway. POST `/contracts` if missing. (Reuses CLI logic from
   RFC-014's `push` if convenient, but a small inline call is fine —
   no shared crate yet.)
2. Loop until `--duration` elapses:
   - Pick a random contract from the list.
   - Roll dice against pass/fail/quarantine percentages.
   - Generate a payload that matches the chosen outcome:
     - **Pass** — all fields valid per the starter's contract.
     - **Fail** — one field deliberately violates a constraint
       (out-of-range int, bad enum, pattern mismatch).
     - **Quarantine** — payload that triggers the quarantine path
       (uses an existing quarantine-eligible failure mode; check
       `src/ingest.rs` for current rules).
   - POST `/ingest` with the event.
   - Sleep `1 / rate` seconds.
3. Print summary on exit: events sent, pass/fail/quarantine counts,
   p99 of round-trip request time.

### Realistic-looking values

Starter-aware payload synthesis:

- `rest_event` — realistic `method`, `path` (e.g. `/api/users/:id`),
  `status` (mostly 200, some 4xx/5xx), `latency_ms` (gamma-distributed).
- `kafka_event` — `topic` from a small list, monotonic `offset`,
  `producer_id` from a small pool.
- `dbt_model` — UUIDs for `id`, monotonic `created_at`/`updated_at`,
  realistic `source_system` enum mix.

Code structure:

```
src/bin/demo-seeder.rs              # main, flag parsing
src/demo_seed/
├── mod.rs
├── synth.rs                        # per-contract payload generators
├── outcome.rs                      # pass / fail / quarantine dice
└── client.rs                       # tiny ingest poster
```

## Starter templates

`contracts/starters/rest_event.yaml`:

```yaml
version: "1.0"
name: rest_event
description: "Generic REST event — request_id, method, path, status, latency."
ontology:
  entities:
    - { name: request_id,    type: string,  required: true,
        pattern: "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" }
    - { name: method,        type: string,  required: true,
        enum: ["GET","POST","PUT","PATCH","DELETE"] }
    - { name: path,          type: string,  required: true }
    - { name: status,        type: integer, required: true, min: 100, max: 599 }
    - { name: latency_ms,    type: integer, required: true, min: 0 }
    - { name: timestamp,     type: integer, required: true }
```

`contracts/starters/kafka_event.yaml`:

```yaml
version: "1.0"
name: kafka_event
description: "Generic Kafka event with partition key + payload metadata."
ontology:
  entities:
    - { name: topic,         type: string,  required: true }
    - { name: partition,     type: integer, required: true, min: 0 }
    - { name: offset,        type: integer, required: true, min: 0 }
    - { name: key,           type: string,  required: false }
    - { name: producer_id,   type: string,  required: true,
        pattern: "^[a-zA-Z0-9_-]+$" }
    - { name: timestamp,     type: integer, required: true }
```

`contracts/starters/dbt_model.yaml`:

```yaml
version: "1.0"
name: dbt_model_row
description: "Row-level contract for a dbt model — id, audit timestamps, soft-delete."
ontology:
  entities:
    - { name: id,            type: string,  required: true }
    - { name: created_at,    type: integer, required: true }
    - { name: updated_at,    type: integer, required: true }
    - { name: deleted_at,    type: integer, required: false }
    - { name: source_system, type: string,  required: true,
        enum: ["postgres","mysql","snowflake","bigquery"] }
```

`contracts/starters/README.md`:

```
1. Copy a starter into your repo's contracts/ dir.
2. Rename + edit fields to match your domain.
3. `contractgate validate` (RFC-014).
4. `contractgate push` to publish.
```

## Test plan

- `tests/compose_smoke.sh` — CI lane: `docker compose up -d`, wait for
  `/health`, post a contract, validate an event, assert pass.
- `tests/compose_demo_smoke.sh` — `docker compose --profile demo up`,
  wait for seeder to exit, assert audit_log row count > 1000 and
  `/metrics` reports non-zero violation counts.
- `tests/starters_validate.rs` — parse + compile each starter; assert
  zero errors.
- `tests/starters_demo_event.rs` — pass a representative event through
  each starter; assert success.
- `tests/demo_seeder_outcomes.rs` — run seeder for 30s at 50/sec
  against a test gateway, assert pass/fail/quarantine ratios within
  ±2pp of configured.

## Rollout

1. Sign-off this RFC.
2. Three starter YAMLs + `contracts/starters/README.md`.
3. `tests/starters_*.rs`.
4. `docker-compose.yml` + `ops/prometheus/prometheus.yml` +
   `ops/grafana/provisioning/`. Dogfood on a fresh laptop (without
   demo profile yet).
5. `src/bin/demo-seeder.rs` + `src/demo_seed/` module.
6. Compose `demo-seeder` service under `profiles: [demo]`.
7. CI smoke lanes (both default and demo profiles).
8. README snapshots: dashboard with seeded data, audit search with rows.
9. `cargo check && cargo test`.
10. Update `MAINTENANCE_LOG.md`.

## Deferred

- Helm chart.
- RBAC role editor UI.
- Air-gapped install bundle.
- License-key flow.
- Template registry API.
- Templates dashboard tab.
- Ratings + imports counter.
- Submission pipeline.
- Private namespaces.
- More starters beyond the initial three.
- Continuous traffic mode for the seeder (currently one-shot).
- Seeder integration with `stream_demo` SSE — kept as separate paths.
