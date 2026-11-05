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

Use the summary to set the real `fly.toml` concurrency `soft_limit` / `hard_limit`
(currently marked PLACEHOLDER) and to confirm the DB pool size
(`CONTRACTGATE_DB_POOL_MAX_CONNECTIONS`) × machine count stays under the
Supabase pooler limit. See [`docs/ops/spike-day-runbook.md`](../../docs/ops/spike-day-runbook.md).
