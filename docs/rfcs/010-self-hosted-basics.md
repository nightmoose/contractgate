# RFC-010: Self-Hosted Basics

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 05 — Self-Hosted Basics                                      |
| Depends on    | RFC-001 (org-scoped tenancy) — read for RBAC scope; RFC-009 (`/metrics`) |

## Summary

Make on-prem install boring. Four deliverables:

1. **Docker Compose reference stack** — gateway + Postgres + Kafka +
   Prometheus + Grafana, one `docker compose up`.
2. **Official Helm chart** — `charts/contractgate/`, values cover replicas,
   ingress, secrets, Postgres connection, optional bundled Postgres.
3. **RBAC role editor UI** — extends partial RBAC. Fine-grained perms
   per contract or per namespace.
4. **Air-gapped install bundle + docs** — pinned image tarballs, offline
   Helm chart, license-key flow.

K8s Operator and Terraform provider are deferred to a follow-up RFC. They
need Helm to mature and the tenancy model to stabilize first.

## Goals

1. `docker compose up` from a clean clone produces a working gateway,
   Prometheus scraping it, Grafana with the RFC-009 dashboard pre-imported,
   and Kafka with a demo topic the gateway is wired to forward to.
2. `helm install contractgate ./charts/contractgate` works on any Kubernetes
   ≥1.27 with a default StorageClass. No CRDs.
3. RBAC role editor surfaces the existing permission primitives in a
   human-usable UI; no new permission types added in v1.
4. Air-gap bundle is a single `tar.gz` containing all images, the chart, a
   `load.sh` script, and a step-by-step `INSTALL.md`.

## Non-goals

- Kubernetes Operator with CRDs (`ContractGateInstance`, `ContractPolicy`)
  `[XL]` — separate RFC.
- Terraform provider `[L]` — pairs with Operator; defer together.
- Auto-upgrade across versions in air-gap installs. Document manual upgrade
  procedure; automate later.
- HA Postgres — chart documents bringing your own; bundled Postgres is
  single-node convenience for dev/staging.
- Multi-cluster federation. Out of scope.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Compose seed data | **Empty by default; `docker compose --profile demo up`** seeds the demo contracts and runs the stream demo. |
| Q2 | Helm chart shape | **Single chart with subcharts** for Postgres (Bitnami, optional via `postgres.enabled`) and Kafka (Bitnami, optional via `kafka.enabled`). |
| Q3 | Helm: ingress class | **Configurable; default `nginx`.** Annotations passed through. |
| Q4 | RBAC scope | **Per-contract roles in v1.** Namespace-level deferred until RFC-001 lands org isolation. |
| Q5 | RBAC permission primitives | **Read existing partial RBAC** (`src/api_key_auth.rs` and downstream) — no new primitives in v1. |
| Q6 | Air-gap registries mirrored | **`ghcr.io/contractgate/*` only.** Subcharts (Postgres, Kafka) pulled from Bitnami at build time and re-tagged into our registry. |
| Q7 | License-key validation | **Offline JWS** signed by Anthropic-team key, validated locally; expiry + max-contracts claims. No phone-home. |
| Q8 | Helm + Compose: Prometheus + Grafana | **Bundled by default; opt-out via `metrics.enabled=false`.** |

## Current state

- No Compose file beyond a dev-only scratch script.
- No Helm chart. Existing K8s manifests are demo-only YAML in `ops/k8s/`.
- Partial RBAC: `src/api_key_auth.rs` enforces `x-api-key` and reads role
  from `api_keys.role`. Roles are `admin`, `editor`, `viewer`. UI today
  just lists keys; no role assignment surface.
- No license-key flow at all — managed plane uses billing entitlements.

## Design

### Docker Compose stack (`docker-compose.yml`)

```yaml
services:
  gateway:
    image: ghcr.io/contractgate/gateway:${TAG:-latest}
    ports: ["8080:8080"]
    environment:
      DATABASE_URL: postgres://cg:cg@postgres/contractgate
      METRICS_AUTH_TOKEN: ""    # open in compose
    depends_on: [postgres]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: cg
      POSTGRES_PASSWORD: cg
      POSTGRES_DB: contractgate
    volumes: [pg_data:/var/lib/postgresql/data]

  kafka:
    image: bitnami/kafka:3.7
    profiles: [kafka]   # opt-in
    environment:
      KAFKA_CFG_NODE_ID: 1
      KAFKA_CFG_PROCESS_ROLES: controller,broker
      KAFKA_CFG_LISTENERS: PLAINTEXT://:9092,CONTROLLER://:9093

  prometheus:
    image: prom/prometheus:latest
    profiles: [metrics]
    volumes:
      - ./ops/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml

  grafana:
    image: grafana/grafana:latest
    profiles: [metrics]
    ports: ["3000:3000"]
    volumes:
      - ./ops/grafana:/etc/grafana/provisioning/dashboards

volumes:
  pg_data:
```

`make stack-up` wraps `docker compose --profile demo --profile metrics up`.

### Helm chart (`charts/contractgate/`)

```
charts/contractgate/
├── Chart.yaml                      # 0.1.0
├── values.yaml                     # documented defaults
├── values.schema.json              # JSON Schema for `helm lint`
├── templates/
│   ├── deployment-gateway.yaml
│   ├── deployment-dashboard.yaml
│   ├── service-gateway.yaml
│   ├── service-dashboard.yaml
│   ├── ingress.yaml
│   ├── secret-api-key.yaml
│   ├── configmap-env.yaml
│   ├── servicemonitor.yaml         # if values.metrics.servicemonitor.enabled
│   └── _helpers.tpl
├── charts/                         # subcharts pulled in via Chart.yaml dependencies
│   ├── postgresql/                 # bitnami/postgresql, conditional
│   └── kafka/                      # bitnami/kafka, conditional
└── README.md                       # values reference + quickstart
```

Key `values.yaml` keys:

```yaml
image:
  gateway: ghcr.io/contractgate/gateway:0.1.0
  dashboard: ghcr.io/contractgate/dashboard:0.1.0

replicaCount:
  gateway: 2
  dashboard: 1

ingress:
  enabled: true
  className: nginx
  hosts: ["contractgate.example.com"]

postgres:
  enabled: true            # bundled subchart
  externalUrl: ""          # set if enabled=false

kafka:
  enabled: false

metrics:
  enabled: true
  servicemonitor: { enabled: false }

license:
  key: ""                  # required for self-host beyond N contracts
```

### RBAC role editor UI

New dashboard route `dashboard/app/(authed)/admin/access/page.tsx`.

Surfaces:
- **Roles tab** — three built-ins (`admin`, `editor`, `viewer`) with a
  matrix view of (resource × action). Read-only in v1; custom role authoring
  is `[L]` and deferred.
- **Assignments tab** — list of API keys; for each, edit role + (in v2)
  per-contract overrides. v1 ships the role dropdown only.

Backend: extend `PUT /api-keys/:id` to accept `role`. Already mostly there;
add a `role` validator and a server-side role-set whitelist.

### Air-gap bundle layout

```
contractgate-airgap-0.1.0.tar.gz
├── INSTALL.md
├── images/
│   ├── gateway.tar
│   ├── dashboard.tar
│   ├── postgres.tar         # if bundling
│   └── kafka.tar            # if bundling
├── chart/
│   └── contractgate-0.1.0.tgz
├── grafana/
│   └── contractgate.json
└── load.sh                  # `docker load < images/*.tar` + `helm install`
```

`load.sh` re-tags pulled images into the user's local registry, then templates
out a `values-override.yaml` pointing the chart at that registry.

License key shape (offline-validatable):

```
JWS payload:
{
  "license_id": "uuid",
  "issued_to": "Acme Corp",
  "issued_at": "2026-04-27T...",
  "expires_at": "2027-04-27T...",
  "max_contracts": 100,
  "features": ["airgap"]
}
```

Validated at gateway startup; mounts in `LICENSE_KEY` env or
`/etc/contractgate/license.jws`.

## Test plan

- `tests/compose_smoke.sh` — CI lane that runs `docker compose up -d`,
  waits for `/health`, posts a contract, validates an event, asserts pass.
- `helm lint` + `helm template` in CI for every PR touching `charts/`.
- `tests/rbac_role_assignment.spec.ts` — Playwright: assign editor role,
  attempt admin-only action, assert 403.
- Air-gap: manual smoke on a network-disabled VM before each release.

## Rollout

1. Sign-off this RFC. Re-read RFC-001 for tenancy constraints.
2. `docker-compose.yml` + `ops/prometheus/prometheus.yml` + Grafana provisioning.
   Dogfood on a fresh laptop.
3. Helm chart skeleton + gateway/dashboard deployments + service + ingress.
4. Subchart wiring (Postgres optional, Kafka optional).
5. `helm lint` CI job.
6. RBAC editor UI — Roles tab first (read-only matrix), then Assignments tab.
7. License-key parser + startup validator.
8. Air-gap bundle assembly script in `ops/airgap/build.sh` + INSTALL.md.
9. `cargo check && cargo test`; dashboard build; chart lint.
10. Update `MAINTENANCE_LOG.md`.

## Deferred

- K8s Operator + CRDs.
- Terraform provider.
- Custom RBAC role authoring.
- Per-namespace permissions (gated on RFC-001 tenancy impl).
- Phone-home telemetry.
