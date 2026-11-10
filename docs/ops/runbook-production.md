# DataContractGate — Production Runbook

**Audience:** operators of the hosted gateway (`contractgate-api` on Fly).  
**Last updated:** 2026-11-10  
**Related:** [architecture overview](../architecture-overview.md), incident 2026-07-14 JWT (internal record, available on request)

---

## 1. What runs in production

| Service | Platform | Notes |
|---|---|---|
| `contractgate-server` | Fly.io app `contractgate-api` | Rust binary; HTTP on internal port **3001** |
| Dashboard | Vercel | Talks to Fly API + Supabase Auth/PostgREST |
| Postgres | Supabase | Contracts, audit, quarantine, orgs, API keys |

**Local equivalents:** `make demo` / `docker compose` — not production.

---

## 2. Health checks

| Endpoint | Meaning |
|---|---|
| `GET /health` | Process up (liveness). **Does not** prove JWT or DB. |
| `GET /ready` | DB `SELECT 1` with timeout (readiness). Fly check path. |

```bash
curl -sS https://api.datacontractgate.com/health
curl -sS https://api.datacontractgate.com/ready
```

**Gotcha:** `/health` 200 while JWT path panics → dashboard “CORS”/502. Always check **logs** for panics (see §6).

The pool pings an idle connection before checkout and closes it when the ping errors or exceeds 2 seconds, so a dead socket is replaced instead of wedging every later request. Connections older than `CONTRACTGATE_DB_POOL_MAX_LIFETIME_SECS` (default 30 minutes; acquire timeout stays 8s, idle timeout 60s) are closed even if they stay busy. `fly.toml` probes `GET /ready` every 15s (5s timeout, 30s grace). One failure returns 503 and the service check stops routing to that machine; it does not restart it. Three consecutive failures exit the process. `[[restart]]` `policy = "on-failure"` (10 retries) starts it again. `policy = "always"` is not used: it would fight `auto_stop_machines` if that were turned back on. Production currently sets `auto_stop_machines = 'off'` with `min_machines_running = 2` (see §5).

---

## 3. Deploy

### Preferred: CI after merge to `main`

GitHub Actions `CI` → job **Deploy — Fly.io** (needs all required checks green).  
Uses `FLY_API_TOKEN` secret. Builds Docker image from repo `Dockerfile`.

### Manual deploy (break-glass)

```bash
cd contractgate
git checkout main && git pull
fly deploy -a contractgate-api --remote-only
```

**Rules:**

1. **One** `fly deploy` at a time. Concurrent deploys fight and leave stale releases.
2. Confirm a **new release version**: `fly releases -a contractgate-api | head -5`
3. Confirm machines: `fly status -a contractgate-api`
4. Smoke: `/health`, `/ready`, signed-in dashboard **Contracts** list.

Rust release builds with Kafka/Kinesis features commonly take **10–20 minutes** on remote builders — not hung.

### Rollback

```bash
fly releases -a contractgate-api
# Known-good image (prod v244, released 2026-11-09). Do not run unless rolling back.
fly deploy -a contractgate-api --image registry.fly.io/contractgate-api:1f10464f9bf37515aa3e5538f6fb3852b0ca4e0d
# or use Fly dashboard → Activity → redeploy prior release
```

Prefer rolling forward with a known-good commit when possible.

---

## 4. Configuration (secrets)

Set via `fly secrets set` (never commit).

| Variable | Required | Purpose |
|---|---|---|
| `DATABASE_URL` | Yes | Supabase Postgres (service role for gateway) |
| `CANONICAL_API_ORIGIN` | Yes | Public API origin. Production must set `https://api.datacontractgate.com`. Staging must set `https://contractgate-api-staging.fly.dev`. Connect Fivetran returns 500 and mints no key if this is unset. |
| `DASHBOARD_ORIGIN` | Yes (prod) | CORS allowlist (comma-separated). Prod carries `https://app.datacontractgate.com,https://aischemagate.com,https://aischemagate.dev` (RFC-096: shared backend serves both DataContractGate and AISchemaGate). |
| `PORT` | No | Default 3001 (matches `fly.toml`) |
| `RUST_LOG` | No | e.g. `contractgate=info,tower_http=warn` |
| `SUPABASE_URL` | If pooler breaks JWKS | Explicit project URL for JWKS fetch |
| `USAGE_RECONCILE_INTERVAL_SECS` | No | RFC-083 counter reconcile period (default `21600` = 6h, min 300) |
| `CONTRACTGATE_DB_POOL_MAX_LIFETIME_SECS` | No | Recycle Postgres connections after this many seconds (default `1800`). `0` or non-numeric is ignored. |
| `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` | No | Max pooled Postgres connections per machine. Code default `10`; `0` or non-numeric is ignored. The live value is a Fly secret **per app** (production **70**, staging **5**), not that default. See the pool budget below. |
| `CONTRACTGATE_DEV_NO_AUTH` | **Never in prod** | Disables auth when `1` |

Dashboard (Vercel) separately needs Supabase anon/service keys, Stripe, `NEXT_PUBLIC_API_URL`, etc.

```bash
fly secrets list -a contractgate-api
```

**Pool budget (checked 2026-11-10).** Supabase is on the Small compute tier:
Postgres `max_connections` is **90** (3 reserved) and the session-pooler
`default_pool_size` is **72**. Both apps use session mode on port 5432, so
each pooled connection is a backend. Machines ×
`CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` must stay under **72** with headroom
for Supabase internals and the other app. Production at 2 × 70 = **140** is
over that cap. Target **20 per production machine** (2 × 20 = 40), which
leaves room for staging at 2 × 5. Do not add machines or raise the pool until
the production secret is lowered. Detail:
[spike-day runbook](spike-day-runbook.md).

---

## 5. Logs & machines

```bash
fly logs -a contractgate-api
fly status -a contractgate-api
fly machines list -a contractgate-api
```

**Machine size (production `fly.toml`, confirmed live 2026-11-10):** 2× performance-1x, **2048 MB**, region `iad`. `auto_stop_machines = 'off'`, `auto_start_machines = true`, `min_machines_running = 2`. HTTP concurrency is requests, soft **100** / hard **150**. Machines stay up; the first Show HN burst does not pay a cold start.

**Staging** is the small shape: `contractgate-api-staging` is 2× shared-cpu-1x / **256 MB**, `auto_stop_machines = 'stop'`, `min_machines_running = 0` (cold start after idle), concurrency soft **150** / hard **200**. See [Staging](#staging-2026-10-14).

After max restart count, machines stay **stopped** → edge **502** with **no CORS headers** → browsers report CORS. Fix the crash, then start/redeploy.

---

## 6. Common incidents

### A. Dashboard contracts fail with “CORS” + 502

1. `fly logs` — look for `panic`, `CryptoProvider`, `SIGABRT`
2. `fly status` — machines stopped?
3. Do **not** start by changing CORS. Fix crash or bring machines up.
4. See JWT CryptoProvider incident (internal record, available on request)

### B. JWT auth broken (“JWKS not loaded”)

1. Check logs at startup for JWKS fetch
2. Set `SUPABASE_URL=https://<project>.supabase.co` if `DATABASE_URL` is a pooler host
3. Redeploy / restart after secret change

### C. `/ready` 503 / degraded

1. Supabase outage or wrong `DATABASE_URL`
2. Pool exhausted (check idle/size in `/ready` JSON when partially up). A pool that stays failed exits after three probes and Fly restarts the machine; a single 503 does not.
3. Network from Fly region to Supabase

### D. Stripe “paid but still free”

1. Dashboard webhook logs + table `stripe_failed_events`
2. Ensure checkout stamps `metadata.orgId`
3. See [stripe-billing-reference](../stripe-billing-reference.md)

### E. Migrations drift

1. Repo files: `supabase/migrations/*.sql`
2. Prod ledger: `supabase_migrations.schema_migrations`
3. Apply missing files **manually** (product owner); agents do not apply to prod
4. CI migration count must match when adding files

---

## 7. Database migrations

```bash
# Local / CI only — production is operator-applied
# Never run destructive migrate against prod from a laptop without a plan
```

When adding a migration:

1. Add `supabase/migrations/NNN_*.sql`
2. Bump `EXPECTED_MIGRATION_COUNT` + sentinel in `.github/workflows/ci.yml`
3. Note the migration in the PR description
4. The product owner applies to prod Supabase **before** (or with) the binary that depends on it

**RFC-083 / migration 032:** `org_monthly_usage` must exist for plan caps to
enforce. Metering **fails open** if the table is missing (ingest continues,
unmetered), but apply 032 first so Free/Growth limits actually work.

**Drift detection:** operator-owned — compare `supabase/migrations/*.sql` to
prod `supabase_migrations.schema_migrations` when applying (no scheduled CI;
deliberately not automated).

One-shot counter repair (up-only vs audit):

```bash
# Against a DATABASE_URL that can write org_monthly_usage
cargo run --bin contractgate-server -- usage-reconcile
```

---

## 8. On-call checklist (minimal)

| Check | Command / place |
|---|---|
| API alive | `curl …/health` + `…/ready` |
| Machines | `fly status` |
| Errors | `fly logs` (panic/5xx) |
| Dashboard | Hard-refresh contracts list signed-in |
| DB | Supabase dashboard status |

**Escalation:** founder / NightMoose — product is pre-public-scale; no 24/7 rotation yet.
Customer-facing targets: [Support SLA](../support-sla.md).

---

## 9. Related demos

- Hero pilot path: [`demo/hero/README.md`](../../demo/hero/README.md) + `scripts/hero_demo.sh`
- Local throughput demo: [`demo/README.md`](../../demo/README.md) (`docker compose -f docker-compose.kafka-demo.yml up -d` + `cargo run --features demo --bin demo`, dashboard on `localhost:8088`)
- Self-host: `make demo`

---

## 10. Do not

- Run multiple concurrent `fly deploy`s
- Set `CONTRACTGATE_DEV_NO_AUTH` in production
- Trust “CORS misconfigured” without reading Fly logs on 502
- Apply prod migrations from agent sessions without human approval

## Staging (2026-10-14)

- Branch **`staging`** (created from `main`). The dev team may merge PRs into
  `staging` freely; **only the product owner merges to `main`** (production).
- Production must set `CANONICAL_API_ORIGIN=https://api.datacontractgate.com`. Staging must set `CANONICAL_API_ORIGIN=https://contractgate-api-staging.fly.dev`.
- Fly app `contractgate-api-staging` via `fly.staging.toml`: `auto_stop_machines = 'stop'`,
  `min_machines_running = 0`, `auto_start_machines = true`, shared-cpu-1x / 256 MB
  in `iad`, concurrency soft 150 / hard 200, `KAFKA_ENABLED=false`. Created 2026-10-14.
  Live pre-flight on 2026-11-10 saw two machines of that size; min stays 0, so
  they stop when idle.
- No Stripe live keys, Confluent or AWS secrets on staging.
- CI: a push to `staging` builds the image, pushes it to
  `registry.fly.io/contractgate-api-staging:<sha>` (token `FLY_API_TOKEN_STAGING`)
  and `deploy-fly-staging` runs `flyctl deploy -c fly.staging.toml --image …`.
  Prod registry push and `deploy-fly` are guarded to `refs/heads/main`.

## Kafka / Confluent ingress — disabled (2026-10-14)

The Confluent Cloud account is closed. `KAFKA_ENABLED` defaults to **false**
(`fly.toml` sets it explicitly): boot does not restore Kafka consumers and
`POST /contracts/{id}/kafka-ingress/enable` returns 400 pointing at `/v1/ingest`.
Existing `kafka_ingress` rows are left as they are. To re-enable: provision a
broker, set the `CONFLUENT_*` secrets, set `KAFKA_ENABLED=true`, redeploy.
The `kafka-ingress` cargo feature stays compiled in (no code removal).
