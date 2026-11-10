# Load testing — Show HN spike

[`spike.js`](spike.js) is a [k6](https://k6.io) script that models a "Show HN"
traffic spike against the gateway: a public read mix (health, catalog, public
contracts, a published contract, and a playground-ish drill-in) plus an
optional authenticated `/v1/ingest` scenario.

**It targets staging by default and refuses to run against production**
(`api.datacontractgate.com`) unless `ALLOW_PROD=1`. Load-test staging, not prod.

## Install k6

See <https://grafana.com/docs/k6/latest/set-up/install-k6/>. On macOS:
`brew install k6`.

## Run

Read-only mix against staging (the default `BASE_URL`):

```bash
k6 run ops/loadtest/spike.js
```

Against an explicit staging URL:

```bash
BASE_URL=https://contractgate-api-staging.fly.dev k6 run ops/loadtest/spike.js
```

Add the authenticated bulk-ingest scenario (needs a staging API key and a
contract id):

```bash
BASE_URL=https://contractgate-api-staging.fly.dev \
API_KEY=cg_staging_... \
CONTRACT_ID=<contract-uuid> \
k6 run ops/loadtest/spike.js
```

## Environment variables

| Var | Default | Purpose |
|---|---|---|
| `BASE_URL` | `https://contractgate-api-staging.fly.dev` | Gateway under test. |
| `ALLOW_PROD` | unset | Must be `1` to allow a `BASE_URL` on `api.datacontractgate.com`. |
| `API_KEY` | unset | Staging API key. When set with `CONTRACT_ID`, enables the `ingest` scenario. |
| `CONTRACT_ID` | unset | Contract UUID to post events to (`POST /v1/ingest/{contract_id}`). |
| `PUBLISHED_REF` | unset | Publication ref for `GET /published/{ref}`. Skipped when unset. |

## Load profile

The `browse` scenario ramps VUs on a front-page curve: warm-up → sharp climb →
sustained peak (~300 VUs) → decay, about 15 minutes total. The optional
`ingest` scenario is a much smaller writer population running in parallel.

Both are `ramping-vus`; adjust the `stages` in `spike.js` to match the traffic
you actually expect.

## Thresholds

The run fails (non-zero exit) if:

- `read_errors` rate ≥ 2% (any 5xx on the public read mix), or
- p95 latency exceeds the per-endpoint budget (200 ms for `/health`, 800 ms for
  the public reads, 1500 ms for authed ingest).

Production concurrency already matches `fly.toml` (confirmed live 2026-11-10):
soft 100 / hard 150. Keep that limit: the 2026-11-10 staging p95 missed the
800 ms budget on shared-cpu machines. The pool size is a per-app Fly secret;
the budget is below.

| App | Live shape | Concurrency |
|---|---|---|
| `contractgate-api` | 2× performance-1x / 2048 MB, `iad`, `min_machines_running = 2`, `auto_stop_machines = 'off'` | soft **100** / hard **150** |
| `contractgate-api-staging` | 2× shared-cpu-1x / 256 MB, `min_machines_running = 0`, `auto_stop_machines = 'stop'` | soft **150** / hard **200** |

**2026-11-10 staging** (this script, browse-only, 300 VUs, ~15 min, shared-cpu):
**182.5 RPS**, p95 **1,245 ms**, **0.0043%** errors. k6 exited 99 on the
DB-backed p95 budget; the error-rate budget passed. **2026-11-05 production**
browse (one performance-1x machine, 300 VUs, 15 min, same prod concurrency
100/150): **524.79 RPS**, p95 **74.69 ms**, **0** errors. Use the production
run as the Show HN capacity guide. Staging is not prod-shaped, and it shares
the production database — leave ingest off unless the writes are intentional.

`CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` is set per app via a Fly secret (code
default 10). Supabase Small: Postgres `max_connections` **90**, session-pool
`default_pool_size` **72**. Machines × pool max must stay under 72 with
headroom for staging, the dashboard, and admin. Production is **25 per
machine** (set 2026-11-10 ~1:29 PM America/New_York, previously 70): 2 × 25 =
**50** session clients of the 72-slot pool. Do not add machines or raise the
pool without redoing that math. See
[`docs/ops/spike-day-runbook.md`](../../docs/ops/spike-day-runbook.md).
