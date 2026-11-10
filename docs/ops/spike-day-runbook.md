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
> pre-flight changed no live Fly or Supabase settings. Later that afternoon
> the DB owner set the production pool secret to 25 per machine (§4).

---

## 1. Pre-flight (the day before)

Checked Tuesday Nov 10, 2026, 12:52–13:07 America/New_York.

- [x] Production `fly.toml` / live config is always-on: `min_machines_running = 2`,
      `auto_stop_machines = 'off'`, `[[vm]]` `performance-1x` / `2048 MB`,
      region `iad` (2 machines). Confirmed live on 2026-11-10.
- [x] Ran `ops/loadtest/spike.js` against **staging** on 2026-11-10
      (browse-only Show HN profile, 300 VUs, ~15 min). Results in §3.
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
- [x] `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` × machine count is
      **under the Supabase session-pooler / Postgres limit**. Set
      2026-11-10 ~1:29 PM America/New_York (DB owner's decision): production
      **25** per machine (was 70). 2 × 25 = **50** session clients against
      the 72-slot session pool. See §4. Do not raise the pool or add
      production machines without redoing that math.
- [x] Known-good image ref recorded for rollback (§8).
- [x] `/health` and `/ready` green on prod after the staging spike; current
      prod release is clean (v244, two machines, 2/2 checks passing).

---

## 2. Launch morning (before posting)

Show HN is Thursday Nov 12, 2026, 8:00 AM America/New_York. Do this **before**
the post goes up.

Staging shares the production database and the same 72-slot session pool.
Stop it so it holds **no** pooler slots during the spike. Production stays at
2 machines (pool 25 each, 50 of 72).

```bash
# Preferred. Count 0 means a stray request cannot auto-start a machine
# (staging has auto_start_machines = true).
fly scale count 0 -a contractgate-api-staging

# Or stop each machine and leave the count in place. A later request can
# auto-start them back into the pool, so prefer scale count 0 for the spike.
fly machines list -a contractgate-api-staging
fly machine stop <machine-id> -a contractgate-api-staging
```

Confirm `fly status -a contractgate-api-staging` shows no started machines,
and production `/health` and `/ready` are still green (`contractgate-api`,
both machines, checks passing).

Bring staging back after the spike (steady state is 2× shared-cpu-1x / 256 MB,
`min_machines_running = 0`, so idle machines can auto-stop again):

```bash
fly scale count 2 -a contractgate-api-staging
# or, if you stopped machines instead of scaling the count:
fly machine start <machine-id> -a contractgate-api-staging
```

---

## 3. Measured results

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

## 4. Pool math

`CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` is set **per app** via a Fly secret.
The code default is 10 (`src/request_limits.rs`). The `fly.toml` comment that
says "today: 10 × 2 = 20" describes that default, not the live secret.

On 2026-11-10 ~1:29 PM America/New_York the DB owner set production to **25**
per machine (previously **70**). Staging remains **5** per machine.

| | Per machine | Machines | Potential session clients |
|---|---:|---:|---:|
| Production live | **25** | 2 | **50** |
| Staging live | **5** | 2 (when both are up) | **10** |
| Combined, if staging is up | | | **60** |
| Session pool (`default_pool_size`) | | | **72** |

50 of 72 leaves headroom for staging (up to 10), the dashboard, and admin
clients. With staging stopped for launch (§2), production's ceiling is 50
and the rest of the 72 slots stay free for those other clients.

Supabase Small compute, checked 2026-11-10:

| Limit | Value |
|---|---|
| Postgres `max_connections` | **90** (3 superuser-reserved; 87 usable) |
| Session pooler `default_pool_size` | **72** (this project; the Small catalog default is 15) |
| Pooler client cap | 400 (front door, mainly transaction mode — not the binding limit) |

The apps use **session mode on port 5432** (one Postgres backend per client).
The binding caps are `default_pool_size` **72** and `max_connections` **90**.
Machines × pool max must stay **under 72 with headroom** for staging, the
dashboard, and admin.

**Verdict:** 2 × 25 = 50 stays under the 72-slot session pool. Do not
`fly scale count` above 2 on production, and do not raise the pool, without
redoing this math (3 × 25 = 75 already exceeds 72). The previous secret of
70 per machine (2 × 70 = 140) did not fit. Roll that secret back only to
undo the reduction (§8); it is not a response to pooler pressure. Stop
staging before the post (§2) so its 2 × 5 does not share the 72 slots
during the spike. The 2026-11-10 browse test (run earlier, staging pool 5)
stayed at 23–30 backends and did not hit the pooler.

Keep 2× performance-1x / 2048 MB, always-on, concurrency 100/150. Watch
Supavisor / `EMAXCONNSESSION` / `/ready` from the opening bell. First kill
switch if the pooler saturates: lower `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS`
below 25 and redeploy. Do not add machines.

---

## 5. Scale commands (documented — run as needed)

```bash
# machines × CONTRACTGATE_DB_POOL_MAX_CONNECTIONS must stay under the
# session pool (default_pool_size 72) with headroom. Production is 25
# per machine: 2 × 25 = 50. 3 × 25 = 75 exceeds 72. Do not scale
# production above 2 without redoing §4. Stop staging first (§2).
# Syntax only — 4 × 25 = 100 does not fit:
# fly scale count 4 -a contractgate-api

# Resize the VM if CPU/memory is the ceiling. Production is already
# performance-1x / 2048 MB, auto_stop off, min_machines_running = 2.
fly scale vm performance-2x --memory 2048 -a contractgate-api

fly status -a contractgate-api        # machines, regions, health
```

---

## 6. What to watch

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

## 7. Kill switches (fastest safe lever first)

- **Shed load:** lower `fly.toml` concurrency `soft_limit` (or scale machines
  only after §4 still fits) so Fly queues/sheds instead of overwhelming Postgres.
- **Rate limits:** the authenticated surface already rate-limits per key; abuse
  shows as 429s — that is the gateway protecting itself, leave it on.
- **Pooler saturation:** lower `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` below 25
  and redeploy. Adding machines makes this worse. Rolling the secret back to
  70 (§8) restores the old over-cap setting; it does not relieve pressure.
- Kafka/Confluent ingress is already disabled (`KAFKA_ENABLED=false`) — no action.

---

## 8. Rollback

Known-good production image, release **v244**, released 2026-11-09. Both
production machines were on this image at the 2026-11-10 pre-flight, with
2/2 checks passing.

`registry.fly.io/contractgate-api:1f10464f9bf37515aa3e5538f6fb3852b0ca4e0d`

```bash
fly releases -a contractgate-api
# Do not run unless rolling back the image.
fly deploy -a contractgate-api --image registry.fly.io/contractgate-api:1f10464f9bf37515aa3e5538f6fb3852b0ca4e0d

# Pool-secret rollback. Production was 70 per machine before 2026-11-10
# ~1:29 PM America/New_York. 2 × 70 = 140 session clients against a 72-slot
# pool. Use only to undo the reduction to 25, not to relieve pooler pressure.
fly secrets set CONTRACTGATE_DB_POOL_MAX_CONNECTIONS=70 -a contractgate-api
```

Prefer rolling **forward** with a known-good commit when possible. To undo
capacity changes, revert `fly.toml` (`min_machines_running`, `vm`,
concurrency) and `fly scale count` / `vm` to the pre-event values: 2×
performance-1x / 2048 MB in `iad`, `min_machines_running = 2`,
`auto_stop_machines = 'off'`, soft 100 / hard 150.

---

## 9. After

- [ ] Restore steady-state scale (`fly scale count`, VM size) if you bumped
      them: 2× performance-1x / 2048 MB in `iad`, `min_machines_running = 2`,
      `auto_stop_machines = 'off'`. Leave the production pool at 25 per machine.
- [ ] Bring staging back if you stopped it for the spike:
      `fly scale count 2 -a contractgate-api-staging`, or `fly machine start`
      for each machine you stopped (§2).
- [ ] Concurrency is already recorded (production soft 100 / hard 150; staging
      soft 150 / hard 200) from the 2026-11-04, 2026-11-05, and 2026-11-10 runs
      in §3. Add any new spike next to those rows. Do not raise production
      concurrency from a slower staging run.
- [ ] Triage quarantined events; note anything worth a follow-up.
