# ContractGate — Production Runbook

**Audience:** operators of the hosted gateway (`contractgate-api` on Fly).  
**Last updated:** 2026-07-15  
**Related:** [architecture overview](../architecture-overview.md), [incident 2026-07-14 JWT](../reviews/incident-2026-07-14-jwt-crypto-provider.md)

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
curl -sS https://contractgate-api.fly.dev/health
curl -sS https://contractgate-api.fly.dev/ready
```

**Gotcha:** `/health` 200 while JWT path panics → dashboard “CORS”/502. Always check **logs** for panics (see §6).

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
fly deploy -a contractgate-api --image <previous-image-ref>
# or use Fly dashboard → Activity → redeploy prior release
```

Prefer rolling forward with a known-good commit when possible.

---

## 4. Configuration (secrets)

Set via `fly secrets set` (never commit).

| Variable | Required | Purpose |
|---|---|---|
| `DATABASE_URL` | Yes | Supabase Postgres (service role for gateway) |
| `DASHBOARD_ORIGIN` | Yes (prod) | CORS allowlist, e.g. `https://app.datacontractgate.com` |
| `PORT` | No | Default 3001 (matches `fly.toml`) |
| `RUST_LOG` | No | e.g. `contractgate=info,tower_http=warn` |
| `SUPABASE_URL` | If pooler breaks JWKS | Explicit project URL for JWKS fetch |
| `USAGE_RECONCILE_INTERVAL_SECS` | No | RFC-083 counter reconcile period (default `21600` = 6h, min 300) |
| `CONTRACTGATE_DEV_NO_AUTH` | **Never in prod** | Disables auth when `1` |

Dashboard (Vercel) separately needs Supabase anon/service keys, Stripe, `NEXT_PUBLIC_API_URL`, etc.

```bash
fly secrets list -a contractgate-api
```

---

## 5. Logs & machines

```bash
fly logs -a contractgate-api
fly status -a contractgate-api
fly machines list -a contractgate-api
```

**Machine size (fly.toml):** shared-cpu-1x, **256 MB** — watch OOM under heavy stream + Kafka consumers.

**Auto stop:** `auto_stop_machines = 'stop'` with `min_machines_running = 1` — cold starts possible if min drops.

After max restart count, machines stay **stopped** → edge **502** with **no CORS headers** → browsers report CORS. Fix the crash, then start/redeploy.

---

## 6. Common incidents

### A. Dashboard contracts fail with “CORS” + 502

1. `fly logs` — look for `panic`, `CryptoProvider`, `SIGABRT`
2. `fly status` — machines stopped?
3. Do **not** start by changing CORS. Fix crash or bring machines up.
4. See [JWT CryptoProvider incident](../reviews/incident-2026-07-14-jwt-crypto-provider.md)

### B. JWT auth broken (“JWKS not loaded”)

1. Check logs at startup for JWKS fetch
2. Set `SUPABASE_URL=https://<project>.supabase.co` if `DATABASE_URL` is a pooler host
3. Redeploy / restart after secret change

### C. `/ready` 503 / degraded

1. Supabase outage or wrong `DATABASE_URL`
2. Pool exhausted (check idle/size in `/ready` JSON when partially up)
3. Network from Fly region to Supabase

### D. Stripe “paid but still free”

1. Dashboard webhook logs + table `stripe_failed_events`
2. Ensure checkout stamps `metadata.orgId`
3. See [stripe-billing-reference](../stripe-billing-reference.md)

### E. Migrations drift

1. Repo files: `supabase/migrations/*.sql`
2. Prod ledger: `supabase_migrations.schema_migrations`
3. Apply missing files **manually** (Alex); agents do not apply to prod
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
3. Document in `MAINTENANCE_LOG.md`
4. Alex applies to prod Supabase **before** (or with) the binary that depends on it

**RFC-083 / migration 032:** `org_monthly_usage` must exist for plan caps to
enforce. Metering **fails open** if the table is missing (ingest continues,
unmetered), but apply 032 first so Free/Growth limits actually work.

**Drift detection:** scheduled workflow
[`.github/workflows/migration-drift.yml`](../../.github/workflows/migration-drift.yml)
(daily + manual) compares repo files to prod `supabase_migrations.schema_migrations`
via read-only secret `PROD_DATABASE_URL`. Not on the PR path.

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
- Stream demo: `app.datacontractgate.com/stream-demo`
- Self-host: `make demo`

---

## 10. Do not

- Run multiple concurrent `fly deploy`s
- Set `CONTRACTGATE_DEV_NO_AUTH` in production
- Trust “CORS misconfigured” without reading Fly logs on 502
- Apply prod migrations from agent sessions without human approval
