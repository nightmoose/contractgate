# Health & Readiness Probes — Reference

DataContractGate exposes two probe endpoints on the same port as the API.
Both are public (no auth required).

---

## `GET /health` — liveness probe

Returns `200` whenever the process is running.  **Does not touch the
database.**

```json
{
  "status": "ok",
  "version": "0.1.0",
  "service": "contractgate"
}
```

**Use as:** Fly.io liveness check, Kubernetes `livenessProbe`, or
docker-compose dependency — anything that should only restart the pod
when the process itself has died.  A database outage must **not** cause
the orchestrator to kill a live pod, so this probe intentionally has no
DB dependency.

---

## `GET /ready` — readiness probe (RFC-053)

Runs `SELECT 1` against the connection pool with a 2-second timeout.

### 200 — ready

```json
{
  "status": "ready",
  "db":     "ok",
  "version": "0.1.0",
  "pool": { "size": 5, "idle": 3 }
}
```

### 503 — degraded

```json
{
  "status": "degraded",
  "db":     "error"
}
```

Returned when the pool cannot serve `SELECT 1` within 2 seconds (pool
exhausted, Supabase project paused, network partition).

**Use as:** platform health check for traffic routing.  Fly.io and
docker-compose probe `/ready` so that traffic is drained from an instance
whose DB pool is broken.  One failed probe does not restart the machine.
Three consecutive failures exit the DataContractGate process; Fly's
`on-failure` restart policy starts it again.

---

## Platform configuration

### Fly.io (`fly.toml`)

```toml
[[http_service.checks]]
  grace_period = '30s'
  interval     = '15s'
  timeout      = '5s'
  method       = 'GET'
  path         = '/ready'

[checks]
  [checks.ready]
    port         = 3001
    type         = 'http'
    interval     = '15s'
    timeout      = '5s'
    grace_period = '30s'
    method       = 'GET'
    path         = '/ready'

[[restart]]
  policy  = 'on-failure'
  retries = 10
```

The service check stops routing while `/ready` is non-2xx. The top-level
check is the same path for `fly checks list`. `on-failure` restarts a
non-zero exit (including the three-failure exit) and still allows autostop.

### docker-compose

```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8080/ready"]
  interval: 5s
  timeout: 3s
  retries: 12
  start_period: 10s
```

---

## JWKS refresh (RFC-052)

The Supabase JWKS key set is refreshed automatically every **10 minutes**
in a background task.  A failed fetch logs a warning and keeps the previous
key set — the gateway never becomes unauthenticated because of a transient
network blip.

If a Bearer token arrives with an unknown `kid` (e.g. during a key
rotation), the gateway triggers an **out-of-band refresh** (debounced to at
most once per 60 seconds) and retries the verification once.  This makes
key rotation near-instant rather than waiting for the next 10-minute tick.

| Env var | Effect |
|---|---|
| `SUPABASE_URL` | Explicit JWKS base URL override (recommended on Fly) |
| `DATABASE_URL` | JWKS URL is derived from the host when `SUPABASE_URL` is unset |

If neither variable produces a valid URL, JWT auth is disabled and only
`API_KEY` / DB-backed API keys are accepted.

## Retired host

When `LEGACY_API_HOSTS` is set (prod: `contractgate-api.fly.dev`), every
request whose `Host` is a legacy name — including `/health` and `/ready` —
gets a 308 to `https://api.datacontractgate.com` (R-S3). Fly's own checks
(`[checks.ready]` in `fly.toml`) call the machine's internal address, whose
`Host` is not a legacy name, so they keep getting 200.
