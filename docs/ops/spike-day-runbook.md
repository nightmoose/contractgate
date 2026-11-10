# Spike-day runbook — Show HN / launch traffic

**Audience:** operator on call for a planned traffic spike on the hosted gateway
(`contractgate-api` on Fly).
**Scope:** launch-day actions plus the 2026-11-10 pre-flight record. For everyday
ops see [production runbook](runbook-production.md).
**Event:** Show HN, Thursday Nov 12, 2026, 8:00 AM America/New_York.
**Comms owner:** Alex Suarez
**Deploy owner:** Alex Suarez · **DB owner:** Alex Suarez

> Commands here are **documented, not run by this doc**. Nothing in this repo
> executes `fly`. Run them yourself, after the pre-flight. The 2026-11-10
> pre-flight changed no live Fly or Supabase settings.

---

## 1. Pre-flight (the day before)

Checked Tuesday Nov 10, 2026, 12:52–13:07 America/New_York.

- [x] Production `fly.toml` / live config is always-on: `min_machines_running = 2`,
      `auto_stop_machines = 'off'`, `[[vm]]` `performance-1x` / `2048 MB`,
      region `iad` (2 machines). Confirmed live on 2026-11-10.
- [x] Ran `ops/loadtest/spike.js` against **staging** on 2026-11-10
      (browse-only Show HN profile, 300 VUs, ~15 min). Results in §2.
      Production concurrency remains `soft_limit = 100` / `hard_limit = 150`
      (already in `fly.toml` and live). Do not raise it from this staging run.
      Staging (`contractgate-api-staging`) is 2× shared-cpu-1x / 256 MB,
      `min_machines_running = 0`, `auto_stop_machines = 'stop'`, soft **150** /
      hard **200**.
- [x] `DATABASE_URL` on both `contractgate-api` and `contractgate-api-staging`
      uses the Supabase **session pooler** host
      `aws-1-us-east-2.pooler.supabase.com:5432`, not the direct host
      (`db.*.supabase.co`) and not the transaction pooler (port 6543).
      Staging startup logs had no "uses the Supabase direct host" warning.
      Staging shares the production database — do not point an authenticated
      ingest load test at it.
- [ ] `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` × machine count is
      **under the Supabase session-pooler / Postgres limit**. This is **not**
      true for production today. See §3. Do not scale machines or raise the
      pool until this is fixed.
- [x] Known-good image ref recorded for rollback (§6).
- [x] `/health` and `/ready` green on prod after the staging spike; current
      prod release is clean (v244, two machines, 2/2 checks passing).

---

## 2. Measured results

Concurrency is already set. These runs are the record; they are not a prompt
to edit `fly.toml`.

| Run | Shape | Load | RPS | p95 | Errors |
|---|---|---|---:|---:|---|
| 2026-11-10 staging | shared-cpu-1x / 256 MB, pool 5; second machine auto-started mid-climb | 300 VUs, browse-only, ~15 min | 182.52 | 1,245 ms | 0.0043% (7 / 164,428) |
| 2026-11-05 production | one performance-1x, concurrency 100/150, session `default_pool_size` 72 | 300 VUs, browse-only, 15 min | 524.79 | 74.69 ms | 0 (p50 67.06 ms, p99 87.1 ms) |
| 2026-11-04 staging | 2 machines, pool 5; this run sized prod concurrency to 100/150 | 100 VUs, browse-only | ~82 | 530 ms | 0 (p99 588 ms, no 429s) |

**2026-11-10 staging** (`https://contractgate-api-staging.fly.dev`, k6 v0.54.0,
`PUBLISHED_REF=3a1ef2a7b410f13e93ccbfd6`). k6 exit **99** because DB-backed p95
exceeded the 800 ms budget. The `read_errors` budget (under 2%) **passed**.
The watchdog did not abort. Production stayed green the whole time.

- Overall: p50 778 ms, p95 **1,245 ms**, p99 1,546 ms. 164,428 requests,
  41,107 iterations, 182.52 RPS. Checks 164,421 / 164,428.
- `/health` p95 58 ms (budget 200 ms, pass). `/catalog` p95 1,252 ms,
  `/public-contracts` p95 1,260 ms, `/published/{ref}` p95 1,268 ms (budget
  800 ms, fail). `/public-contracts/{id}` was not exercised
  (`--discard-response-bodies` skipped the drill-in, about 25% fewer DB reads
  than a full browse). `/v1/ingest` was not registered.
- The 7 failures were status 0 / 60s timeouts (1 `/health`, 4 `/catalog`,
  1 `/public-contracts`, 1 `/published`). No HTTP 429, no `EMAXCONNSESSION`,
  no "too many connections", no `pool_acquire` timeouts. Staging `/ready` at
  peak: `pool.idle = 0`, `pool.size = 5`. Postgres during the run: 23–30
  backends, 10–16 Supavisor.

Staging is not prod-shaped (shared CPU, 256 MB, pool 5, scale-to-zero). The
**2026-11-05 production browse result** is the capacity guide for Show HN.
Do not raise production concurrency from the slower staging p95, and do not
drop production back to one machine: the 2026-11-05 pass was a single machine
and is not a reason to give up the second.

---

## 3. Pool math

`CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` is set **per app** via a Fly secret.
The code default is 10 (`src/request_limits.rs`). The `fly.toml` comment that
says "today: 10 × 2 = 20" describes that default, not the live secret.

| | Per machine | Machines | Potential session clients |
|---|---:|---:|---:|
| Production live | **70** | 2 | **140** |
| Staging live | **5** | 2 (when both are up) | **10** |
| Combined live | | | **150** |
| Target production | **20** | 2 | **40** |

Supabase Small compute, checked 2026-11-10:

| Limit | Value |
|---|---|
| Postgres `max_connections` | **90** (3 superuser-reserved; 87 usable) |
| Session pooler `default_pool_size` | **72** (this project; the Small catalog default is 15) |
| Pooler client cap | 400 (front door, mainly transaction mode — not the binding limit) |

The apps use **session mode on port 5432** (one Postgres backend per client).
The binding caps are `default_pool_size` **72** and `max_connections` **90**.
Machines × pool max must stay **under 72 with headroom** for Supabase
internals and the other app.

**Verdict:** production at 2 × 70 = 140 can exhaust the 72-slot session pool
(`EMAXCONNSESSION`). One full production machine (70) already nearly fills 72.
Target **20 per production machine** (2 × 20 = 40), which leaves room for a
staging pool of 2 × 5 = 10. Do not `fly scale count` above 2, and do not raise
the pool, until the production secret is lowered. The 2026-11-10 browse test
itself stayed at 23–30 backends and did not hit the pooler.

If no pool change is approved before Show HN, keep 2× performance-1x / 2048 MB,
always-on, concurrency 100/150, and watch Supavisor / `EMAXCONNSESSION` /
`/ready` from the opening bell. First kill switch if the pooler saturates:
lower `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` and redeploy. Do not add machines.

---

## 4. Scale commands (documented — run as needed)

```bash
# machines × CONTRACTGATE_DB_POOL_MAX_CONNECTIONS must stay under the
# session pool (default_pool_size 72) with headroom. Unsafe until the
# production secret drops from 70 to the target of 20 (2 × 20 = 40).
# Do not scale count above 2 for Show HN before that recheck.
fly scale count 4 -a contractgate-api

# Resize the VM if CPU/memory is the ceiling. Production is already
# performance-1x / 2048 MB, auto_stop off, min_machines_running = 2.
fly scale vm performance-2x --memory 2048 -a contractgate-api

# Lower the per-machine pool toward the target of 20. Documented only;
# needs the DB owner's go-ahead. Then re-check §3.
fly secrets set CONTRACTGATE_DB_POOL_MAX_CONNECTIONS=20 -a contractgate-api

fly status -a contractgate-api        # machines, regions, health
```

---

## 5. What to watch

| Signal | Where | Healthy |
|---|---|---|
| p95 / p99 latency, 5xx rate | Grafana (`ops/grafana`) / `/metrics` | p99 well under 15 ms engine; HTTP p95 stable |
| **429s** | Logs / metrics | Rising 429s = rate limits biting; expected under abuse, not normal load |
| **Supabase connections** | Supabase dashboard → Database → Pooler | Under `default_pool_size` 72; no `EMAXCONNSESSION` or "too many connections" |
| `/ready` failures, 503s | `fly logs`, Fly checks | No machine cycling on readiness |
| **Quarantine** volume | Dashboard / `/quarantine` | Spikes mean bad-data flood, not an outage — triage after |
| Pool waits | Logs: `pool_acquire`, "ping timed out" | Rare; sustained waits = pool too small or pooler saturated |

```bash
fly logs -a contractgate-api | grep -E 'timed out|pool_acquire|too many connections|EMAXCONNSESSION|429'
```

---

## 6. Kill switches (fastest safe lever first)

- **Shed load:** lower `fly.toml` concurrency `soft_limit` (or scale machines
  only after §3 still fits) so Fly queues/sheds instead of overwhelming Postgres.
- **Rate limits:** the authenticated surface already rate-limits per key; abuse
  shows as 429s — that is the gateway protecting itself, leave it on.
- **Pooler saturation:** lower `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` and
  redeploy. Live production is 70 per machine; the target is 20. Adding
  machines makes this worse.
- Kafka/Confluent ingress is already disabled (`KAFKA_ENABLED=false`) — no action.

---

## 7. Rollback

Known-good production image, release **v244**, released 2026-11-09. Both
production machines were on this image at the 2026-11-10 pre-flight, with
2/2 checks passing.

`registry.fly.io/contractgate-api:1f10464f9bf37515aa3e5538f6fb3852b0ca4e0d`

```bash
fly releases -a contractgate-api
# Do not run unless rolling back.
fly deploy -a contractgate-api --image registry.fly.io/contractgate-api:1f10464f9bf37515aa3e5538f6fb3852b0ca4e0d
```

Prefer rolling **forward** with a known-good commit when possible. To undo
capacity changes, revert `fly.toml` (`min_machines_running`, `vm`,
concurrency) and `fly scale count` / `vm` to the pre-event values: 2×
performance-1x / 2048 MB in `iad`, `min_machines_running = 2`,
`auto_stop_machines = 'off'`, soft 100 / hard 150.

---

## 8. After

- [ ] Restore steady-state scale (`fly scale count`, VM size) if you bumped
      them: 2× performance-1x / 2048 MB in `iad`, `min_machines_running = 2`,
      `auto_stop_machines = 'off'`.
- [ ] Concurrency is already recorded (production soft 100 / hard 150; staging
      soft 150 / hard 200) from the 2026-11-04, 2026-11-05, and 2026-11-10 runs
      in §2. Add any new spike next to those rows. Do not raise production
      concurrency from a slower staging run.
- [ ] Triage quarantined events; note anything worth a follow-up.
