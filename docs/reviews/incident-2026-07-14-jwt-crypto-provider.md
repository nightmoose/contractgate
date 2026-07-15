# Incident: Prod JWT CryptoProvider panic (2026-07-14)

**Status:** Resolved (Fly **v126**, manual deploy)  
**Severity:** P0 — dashboard unusable for signed-in users (contracts list dead)  
**Duration:** ~hours of intermittent/full outage until v126  
**Code fix:** PR [#141](https://github.com/nightmoose/contractgate/pull/141)  
**Also related:** PR [#140](https://github.com/nightmoose/contractgate/pull/140) CORS allowlist (secondary)

This note exists so the next agent (Claude/Grok/human) does **not** re-debug
CORS when the real bug is a JWT panic, and does **not** thrash Fly deploys.

---

## User-visible symptoms

On `https://app.datacontractgate.com` (contracts page):

- Red UI: `Failed to load contracts: Failed to fetch`
- Chrome console:
  - `Access to fetch ... blocked by CORS policy: No 'Access-Control-Allow-Origin'`
  - `GET https://contractgate-api.fly.dev/contracts net::ERR_FAILED 502 (Bad Gateway)`
  - Sometimes a bare `500` on an extension/resource

**Misleading signal:** CORS.  
**Actual signal:** **502** from Fly while machines crash-loop.

---

## Root cause

`jsonwebtoken` **9 → 10** was on `main` with only:

```toml
jsonwebtoken = { version = "10", features = ["use_pem"] }
```

jsonwebtoken **v10 requires** an explicit crypto backend feature:

- `rust_crypto` (pure Rust — preferred here), **or**
- `aws_lc_rs`

Without it, the first real Bearer JWT verification panics:

```text
thread 'tokio-rt-worker' panicked at jsonwebtoken-10.4.0/src/crypto/mod.rs:125:42:
Could not automatically determine the process-level CryptoProvider from
jsonwebtoken crate features.
Call CryptoProvider::install_default() before this point ...
```

With `panic = "abort"` in release profile → **SIGABRT** → Fly restarts machine
→ after max restart count (10) machine **stopped** → edge returns **502**
**without** CORS headers → Chrome reports **CORS blocked**.

Unauthenticated `/health` and often `/ready` still returned 200 when a machine
was briefly up, so “API is fine” probes lied.

Dashboard path: Supabase session → `Authorization: Bearer <jwt>` →
`require_api_key` → `jwt_auth::verify_supabase_jwt` → **boom**.

---

## Fix (code)

```toml
# Cargo.toml — DO NOT remove rust_crypto on Dependabot bumps
jsonwebtoken = { version = "10", features = ["use_pem", "rust_crypto"] }
```

Secondary (same day, not the crash root cause): protected CORS
`allow_headers` includes `Accept` + `x-api-key` (dashboard fallback when no
session). See `docs/auth-reference.md`.

---

## Fix (prod)

| Item | Detail |
|---|---|
| Merged | PR #141 → `main` (`98d1fef` merge commit family) |
| Live | Fly app `contractgate-api` **release v126** (manual deploy by Alex) |
| Broken | v125 and earlier images still panic on JWT |

Merging to GitHub **does not** fix prod until Fly shows a **new release number**
and machines run that image. CI `Deploy — Fly.io` waits for full smoke matrix
(10–20+ min) and was not the path that restored service.

Rust image builds with `kafka-ingress,scaffold,kinesis-ingress` commonly take
**10–20 minutes** on Fly/Depot — that is normal, not hung.

---

## How to diagnose next time (copy-paste)

```bash
# 1. Health can lie
curl -sS https://contractgate-api.fly.dev/health
curl -sS https://contractgate-api.fly.dev/ready

# 2. Real signal: Fly logs (look for CryptoProvider / SIGABRT)
fly logs -a contractgate-api

# 3. Release number
fly releases -a contractgate-api | head -5
fly status -a contractgate-api

# 4. After a real deploy: Bearer should 401 cleanly, not kill the process
curl -sS -D - -o /dev/null \
  -H "Origin: https://app.datacontractgate.com" \
  -H "Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.e30.x" \
  https://contractgate-api.fly.dev/contracts
```

If logs show `CryptoProvider` + crash-loop → **not** a CORS allowlist bug.
Do not start by editing `CorsLayer`.

---

## What went wrong operationally (so we don’t repeat it)

1. Initial misdirection to CORS (browser console) instead of Fly panic logs.
2. Multiple concurrent `fly deploy` processes from agent sessions fought each
   other and never cleanly advanced the release.
3. Declaring “fixed” at merge time without confirming **Fly version ≥ v126**.
4. Leftover `MAINTENANCE_LOG.md` conflict markers on `main` from RFC-081 merge
   (cleaned in #140/#141) — caused PR “conflicted” noise.

**Rule:** one deploy at a time; wait for new release in Activity; then verify
dashboard + logs.

---

## Related files

| File | Why |
|---|---|
| `Cargo.toml` | `jsonwebtoken` features — protect `rust_crypto` |
| `src/jwt_auth.rs` | JWT verify path |
| `src/main.rs` | `require_api_key`, protected CORS |
| `docs/auth-reference.md` | CORS + 502-vs-CORS note |
| `MAINTENANCE_LOG.md` | Same-day run entry |
| `fly.toml` | App name, auto_stop, 256 MB VM |

---

## Checklist before closing a similar incident

- [ ] Fly release version advanced past the bad one  
- [ ] No `CryptoProvider` / SIGABRT in `fly logs` for 5+ minutes of signed-in traffic  
- [ ] Dashboard contracts page loads after hard refresh  
- [ ] `MAINTENANCE_LOG.md` updated with resolution  
- [ ] Dependabot cannot silently drop `rust_crypto` (comment + PR review)

---

*Written 2026-07-15 after v126 restored production. For Claude: read this before
“fixing CORS” on contractgate-api 502s from the dashboard.*
