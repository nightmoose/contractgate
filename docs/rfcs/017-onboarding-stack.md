# RFC-017: Onboarding Stack (Compose + Starter Templates)

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #4                                                        |
| Supersedes    | RFC-010 partially (Compose only); RFC-012 partially (3 starters only)  |

## Summary

Two pieces, both pre-customer onboarding:

1. **Docker Compose reference stack** — `docker compose up` boots
   gateway + Postgres + Prometheus + Grafana in five minutes.
2. **Three starter contract templates** — `contracts/starters/*.yaml`
   checked into the repo. Copy-and-modify; no registry, no UI.

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

## Non-goals

- Bundled Postgres / Kafka via Helm subcharts.
- RBAC role editor UI.
- License-key flow, air-gap bundle.
- Template registry API, dashboard browser tab, ratings, imports counter,
  submission pipeline, private namespaces.
- Auto-import of starters on signup (no signup yet).

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Compose seed data | **Empty by default; `--profile demo` seeds the demo contracts** and runs the existing stream demo. |
| Q2 | Compose Kafka | **Opt-in via `--profile kafka`.** Most pilots don't need it on day 1. |
| Q3 | Compose Prometheus + Grafana | **Bundled by default.** Demos the RFC-016 dashboard out of the box. |
| Q4 | Starter location | **`contracts/starters/{rest_event.yaml, kafka_event.yaml, dbt_model.yaml}`** in the repo. No DB rows. |
| Q5 | Starter discovery | **Documented in `contracts/starters/README.md`** with a one-paragraph copy-and-modify guide. |
| Q6 | Image source | **`ghcr.io/contractgate/gateway:latest`** + the dashboard image. Built by the existing release workflow. |

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

  kafka:
    image: bitnami/kafka:3.7
    profiles: [kafka]
    environment:
      KAFKA_CFG_NODE_ID: 1
      KAFKA_CFG_PROCESS_ROLES: controller,broker

volumes:
  pg_data:
```

`make stack-up` wraps `docker compose up`. `make stack-up-demo` adds
`--profile demo`.

`ops/prometheus/prometheus.yml` is a one-target scrape pointing at
`gateway:8080/metrics` (RFC-016).

`ops/grafana/provisioning/dashboards/contractgate.yaml` provisioning
file points Grafana at `ops/grafana/contractgate.json` (RFC-016).

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

`contracts/starters/README.md` documents the copy-and-modify flow:

```
1. Copy a starter into your repo's contracts/ dir.
2. Rename + edit fields to match your domain.
3. `contractgate validate` (RFC-014).
4. `contractgate push` to publish.
```

## Test plan

- `tests/compose_smoke.sh` — CI lane: `docker compose up -d`, wait for
  `/health`, post a contract, validate an event, assert pass.
- `tests/starters_validate.rs` — parse + compile each starter; assert
  zero errors.
- `tests/starters_demo_event.rs` — pass a representative event through
  each starter; assert success.

## Rollout

1. Sign-off this RFC.
2. `docker-compose.yml` + `ops/prometheus/prometheus.yml` +
   `ops/grafana/provisioning/`. Dogfood on a fresh laptop.
3. Three starter YAMLs + README.
4. `tests/starters_*.rs`.
5. CI compose smoke lane.
6. `cargo check && cargo test`.
7. Update `MAINTENANCE_LOG.md`.

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
