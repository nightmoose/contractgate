# Spike-day runbook — Show HN / launch traffic

**Audience:** operator on call for a planned traffic spike on the hosted gateway
(`contractgate-api` on Fly).
**Scope:** one page. For everyday ops see [production runbook](runbook-production.md).
**Comms owner:** _<name/handle — fill in before the event>_
**Deploy owner:** _<name/handle>_ · **DB owner:** _<name/handle>_

> Commands here are **documented, not run by this doc**. Nothing in this repo
> executes `fly`. Run them yourself, after the pre-flight.

---

## 1. Pre-flight (the day before)

- [ ] `fly.toml` is always-on: `min_machines_running = 2`,
      `auto_stop_machines = 'off'`, `[[vm]]` `performance-1x` / `2048 MB`.
- [ ] Ran `ops/loadtest/spike.js` against **staging** and set the concurrency
      `soft_limit` / `hard_limit` from the result (they are marked PLACEHOLDER).
- [ ] `DATABASE_URL` uses the Supabase **pooler** host
      (`*.pooler.supabase.com`), not the direct host (`db.*.supabase.co`).
      Confirm there is no "uses the Supabase direct host" warning in startup logs.
- [ ] `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` (default 10) × machine count is
      **under the Supabase pooler connection limit**. Default plan: 10 × 2 = 20.
      Check the pooler limit in the Supabase dashboard before raising either.
- [ ] Known-good image ref recorded for rollback (`fly releases -a contractgate-api`).
- [ ] `/health` and `/ready` green; recent deploy is clean.

---

## 2. Scale commands (documented — run as needed)

```bash
# Add machines for the surge (keep pool size × count under the pooler limit).
fly scale count 4 -a contractgate-api

# Resize the VM if CPU/memory is the ceiling.
fly scale vm performance-2x --memory 2048 -a contractgate-api

# Raise the DB pool per machine (then re-check the pooler math).
fly secrets set CONTRACTGATE_DB_POOL_MAX_CONNECTIONS=15 -a contractgate-api

fly status -a contractgate-api        # machines, regions, health
```

---

## 3. What to watch

| Signal | Where | Healthy |
|---|---|---|
| p95 / p99 latency, 5xx rate | Grafana (`ops/grafana`) / `/metrics` | p99 well under 15 ms engine; HTTP p95 stable |
| **429s** | Logs / metrics | Rising 429s = rate limits biting; expected under abuse, not normal load |
| **Supabase connections** | Supabase dashboard → Database → Pooler | Below the pooler limit; no "too many connections" |
| `/ready` failures, 503s | `fly logs`, Fly checks | No machine cycling on readiness |
| **Quarantine** volume | Dashboard / `/quarantine` | Spikes mean bad-data flood, not an outage — triage after |
| Pool waits | Logs: `pool_acquire`, "ping timed out" | Rare; sustained waits = pool too small or pooler saturated |

```bash
fly logs -a contractgate-api | grep -E 'timed out|pool_acquire|too many connections|429'
```

---

## 4. Kill switches (fastest safe lever first)

- **Shed load:** lower `fly.toml` concurrency `soft_limit` (or scale machines)
  so Fly queues/sheds instead of overwhelming Postgres.
- **Rate limits:** the authenticated surface already rate-limits per key; abuse
  shows as 429s — that is the gateway protecting itself, leave it on.
- **Pooler saturation:** lower `CONTRACTGATE_DB_POOL_MAX_CONNECTIONS` and
  redeploy if total connections approach the Supabase limit (fewer, not more).
- Kafka/Confluent ingress is already disabled (`KAFKA_ENABLED=false`) — no action.

---

## 5. Rollback

```bash
fly releases -a contractgate-api
fly deploy -a contractgate-api --image <previous-good-image-ref>
```

Prefer rolling **forward** with a known-good commit when possible. To undo
capacity changes, revert `fly.toml` (`min_machines_running`, `vm`,
concurrency) and `fly scale count`/`vm` to the pre-event values.

---

## 6. After

- [ ] Restore steady-state scale (`fly scale count`, VM size) if you bumped them.
- [ ] Fold the real numbers back into `fly.toml` concurrency limits (remove the
      PLACEHOLDER note) and this runbook.
- [ ] Triage quarantined events; note anything worth a follow-up.
