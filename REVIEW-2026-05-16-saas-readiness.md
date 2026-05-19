# ContractGate SaaS-Readiness Review — 2026-05-16

Audit of backend (Rust/Axum), dashboard (Next.js 15), Supabase migrations, and
auth flow. Goal: turn the current build into a real multi-tenant SaaS.
Ordered by severity. Each item names the file(s), the defect, and the fix.

---

## P0 — Blocking the product from working at all

### P0-1. Dashboard cannot authenticate to backend as the logged-in user

**Files:** `dashboard/lib/api.ts:24,88`, `dashboard/Dockerfile:14,19`,
`src/main.rs:740-785`, `src/api_key_auth.rs`

**What's wrong**
The dashboard sends `x-api-key: ${NEXT_PUBLIC_API_KEY}` on every backend call.
`NEXT_PUBLIC_API_KEY` is a **build-time** env var inlined by `next build`.
There is no per-user binding. Result:

- If `NEXT_PUBLIC_API_KEY` is empty (current state), the header is omitted,
  backend returns 401 → SWR fails silently → contracts list + audit log appear
  empty. **This is the bug Alex is seeing in the screenshot.**
- If it is set, every logged-in user of the dashboard authenticates as the
  same single shared key — the org_id on writes is whatever org that key
  was issued to, regardless of who's actually logged in.

The architecture cannot work for a multi-tenant SaaS as written.

**Fix (one of, pick one — recommend A):**

- **A. Add a Supabase-JWT auth path to the Rust backend.** Accept
  `Authorization: Bearer <supabase-jwt>` in `require_api_key` (rename to
  `require_auth`). Verify the JWT with the Supabase JWKS, extract `sub`
  (user_id), look up the user's primary org membership, and inject a
  `ValidatedKey`-shaped struct into request extensions. The dashboard then
  sends the user's Supabase session JWT — no static API key needed for
  browser traffic. API keys remain for server-to-server (Kafka connectors,
  CLI, SDKs).
- **B. Proxy every backend call through a Next.js route handler.** The route
  handler reads the user's Supabase session, looks up an org-scoped service
  key, and forwards. Keeps backend untouched but doubles latency and adds a
  proxy layer to maintain.
- **C. Auto-issue a session-bound API key on login.** Dashboard issues a
  short-lived key via Supabase RPC right after sign-in and stores it in a
  Secure cookie; sent as `x-api-key`. Simplest to ship but rotates keys
  constantly and pollutes `api_keys` rows.

A is the right answer for a SaaS.

### P0-2. Multi-tenancy leak: `contract_versions`, `contract_name_history`, `quarantine_events` are world-readable

**Files:** `supabase/migrations/002_quarantine_and_p99.sql:38-39`,
`supabase/migrations/003_contract_versioning.sql:206-214`

**What's wrong**
Migrations 002 and 003 created `auth_all` policies (`FOR ALL TO authenticated
USING (TRUE)`). Migration 007 dropped `auth_all` on `contracts`, `audit_log`,
and `forwarded_events` but **never touched** `contract_versions`,
`contract_name_history`, or `quarantine_events`. Any authenticated Supabase
user can SELECT/INSERT/UPDATE/DELETE the YAML, version ladder, and
quarantined raw payloads of every other tenant via the anon REST API.

The dashboard reads `active_contracts_public` directly
(`dashboard/app/contracts/page.tsx:773`), which inherits whatever the
underlying view exposes — so this leak is reachable from the browser.

**Fix**
Add a migration that drops `auth_all` on those three tables and recreates
org-scoped policies using `get_my_org_ids()`, mirroring 008/013. Also audit
every view that reads from these tables for the same exposure.

### P0-3. `api_keys.key_hash` is documented as bcrypt, stored as SHA-256

**Files:** `supabase/migrations/006_accounts_and_api_keys.sql:117,138-140`,
`dashboard/app/account/page.tsx:48-57`, `src/api_key_auth.rs:23,199-200`

**What's wrong**
Schema comments and migration prose claim bcrypt cost 10. The dashboard
generates a SHA-256 base64 digest client-side, and the Rust validator
compares with raw SHA-256. SHA-256 of a 56-char random key is acceptable
security (the entropy is in the key, not the hash) but it is **not** what
the docs say, and a future engineer reading the migration will assume the
column is bcrypt and break verification.

**Fix**
Pick one and make the code+docs match:
- **Keep SHA-256.** Update the migration comments and the `account/page.tsx`
  comment that says "API route handles bcrypt hashing" (it doesn't — there
  is no API route, the insert is direct from the browser). Add a `CHECK`
  constraint on `key_hash` length to catch accidental algorithm swaps.
- **Move to argon2id/bcrypt.** Add a Next.js route handler that hashes
  server-side; have `api_key_auth.rs` switch to `argon2` crate. More work,
  marginal security gain for high-entropy keys.

### P0-4. API keys are inserted directly from the browser

**File:** `dashboard/app/account/page.tsx:163-192`

**What's wrong**
`handleCreateKey` calls `supabase.from("api_keys").insert(...)` directly
from the browser. That works today only because the user is authenticated
to Supabase, but it means:

- The browser is trusted to hash the key correctly (a malicious client
  could insert a hash that matches a *different* known-cleartext key).
- There is no server-side audit trail of key issuance.
- The hash algorithm is locked to whatever the browser implements.

**Fix**
Route key creation through a Next.js API handler. The handler validates
the session, generates the raw key server-side, hashes it server-side,
inserts via the service role, and returns the raw key exactly once. The
client never touches the hashing logic.

---

## P1 — Real SaaS gaps (ship-blockers for paid customers)

### P1-1. Rate limiting is only wired for `POST /v1/ingest/{id}`

**Files:** `src/main.rs:830-1029`, `src/v1_ingest.rs:378`,
`src/rate_limit.rs`

The token-bucket exists and works, but `require_api_key` does not call it.
Only the v1 ingest handler explicitly consumes a token. Every other
protected endpoint (`/contracts`, `/contracts/infer/*`, `/audit`, regular
`/ingest`, `/egress`, `/scorecard`, etc.) has no per-key rate limit.

**Fix**
Move `rate_limiter.check(...)` into `require_api_key` after the key is
validated, before `next.run(request)`. Skip for `/health`, `/metrics`,
`/openapi.json`. Return 429 with `Retry-After` from the bucket's reset_ms.

### P1-2. No request body size limit on most endpoints

**File:** `src/main.rs:1040-1042`

`RequestBodyLimitLayer(10 MB)` is only applied to the v1 sub-router. The
regular `/ingest/{id}`, `/contracts/scaffold`, `/contracts/import`,
`/contracts/infer/csv`, `/contracts/deploy` etc. accept unbounded bodies.
A single 2 GB POST can OOM the server.

**Fix**
Lift the limit layer to the top-level router, with per-route overrides if
some endpoints legitimately need bigger payloads (scaffold from large
samples).

### P1-3. `/playground/validate` is public + unlimited

**File:** `src/main.rs:801`

The playground compiles arbitrary user-supplied YAML and runs validation
with no auth and no rate limit. A loop with pathological regex patterns
(`(a+)+b` on long strings) is a DoS vector even with `regex` crate
linear-time guarantees, because compilation itself is O(2^n) for
adversarial nested-quantifier inputs and the playground runs the full
compile pipeline on each request.

**Fix**
Cap incoming YAML size (e.g. 64 KB), cap incoming event size (16 KB),
add a per-IP rate limit (5 req/s, burst 20). Optionally require a captcha
or auth in production.

### P1-4. No background eviction on the API key cache

**File:** `src/api_key_auth.rs:88-160`

The cache evicts one stale entry per cache miss. Under steady-state
traffic with a fixed key set this is fine, but key churn (test suites,
short-lived CI keys, revoked keys never re-attempted) means the map grows
unbounded. The `HashMap` is keyed by the raw API key string; a
60-second-TTL entry from a long-revoked key stays in memory forever if no
one ever queries that prefix again.

**Fix**
Add a spawned task that scans the map every 5 min and evicts everything
older than TTL. Bound the map size as a belt-and-braces (e.g. LRU at
10 000).

### P1-5. `useOrg` hook violates Rules of Hooks

**File:** `dashboard/lib/org.ts:55-68`

```ts
export function useOrg(): UseOrgResult {
  if (DEMO_MODE) { return { ... }; }   // early return
  const [org, setOrg] = useState(...); // hook called conditionally
```

`DEMO_MODE` is a module-level const so this won't blow up at runtime, but
it will fail `react-hooks/rules-of-hooks` lint and any future change that
makes `DEMO_MODE` reactive (env override, query param) will crash with the
"rendered more hooks than during the previous render" error.

**Fix**
Move the demo branch to inside the `useEffect` or wrap the entire body in
the demo check (return a fixed object after the hook calls, not before).

### P1-6. SSRF gap: `infer_url` does not block redirects to private IPs

**File:** `src/infer_url.rs:115-119`

`reqwest::Client::builder()` is built without `.redirect(Policy::none())`.
The SSRF check pins the *initial* DNS to a public IP, but reqwest follows
up-to-10 redirects by default and each redirect does a fresh DNS lookup.
A public attacker-controlled URL can 302 to `http://169.254.169.254/...`
on the second hop.

**Fix**
Either disable redirects (`.redirect(redirect::Policy::none())`) or
implement a custom `redirect::Policy` that re-runs `check_ssrf` on each
Location header before allowing the hop.

### P1-7. Audit honesty regression in error path

**File:** `src/ingest.rs` (per project memory `feedback_audit_honesty.md`)

Worth re-verifying: the memory entry says "contract_version in audit_log
must reflect the version that actually matched, never a default". Spot
check that the wholesale-quarantine path on `DeprecatedVersionPinned`
still writes the actually-pinned version, not the latest_stable fallback.

### P1-8. CORS is `allow_origin: Any` + `allow_headers: Any`

**File:** `src/main.rs:792-795`

```rust
let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any);
```

Combined with cookie-based dashboard auth (if P0-1 is fixed via the
JWT-in-cookie route), this opens CSRF. Even with header-based auth, `Any`
origin with `x-api-key` allowed means any malicious site can replay a
user's cached API key from JS.

**Fix**
Whitelist the dashboard origin(s) explicitly. Read from `DASHBOARD_ORIGIN`
env var (comma-separated). For unauth public endpoints (`/health`,
`/metrics`, `/public-contracts`, `/catalog`) keep a separate permissive
CORS layer.

### P1-9. No CSRF protection on the Next.js API routes that mutate

**Files:** `dashboard/app/api/org/members/route.ts`,
`dashboard/app/api/github/config/route.ts`, `dashboard/app/api/invites/accept/route.ts`

PUT/POST/DELETE handlers rely entirely on the Supabase session cookie.
SameSite=Lax helps most cases but `POST` from a form is still allowed
cross-site under Lax. Add either a same-origin check (`Origin`/`Referer`)
or a CSRF token round-trip.

### P1-10. Service-role key handling for v1 ingest writes

**File:** `src/main.rs:1099-1111`

`DATABASE_URL` is used by sqlx to connect as whatever role the URL
specifies. If it's the service role, RLS is bypassed entirely on every
query — fine for the trusted Rust backend, but it means the backend has
ambient authority and a bug in any handler that takes `org_id` from a
client header (see `org_id_from_req`) becomes a tenant break.

**Fix**
Document the trust boundary clearly in `main.rs`. Add a runtime check at
startup that warns if the DB URL is using the service role with RLS off
on key tables (sanity guard). Replace the `x-org-id` fallback header
trust with a stricter rule: when no `ValidatedKey` is present and
`api_key` is non-empty, reject — don't fall back to the header.

---

## P2 — Tech debt + correctness

### P2-1. `org_id_from_req` trusts `x-org-id` header in legacy mode

**File:** `src/main.rs:280-290`

The fallback path lets any client claim any `org_id` simply by setting a
header, as long as they have *any* valid env-var key. This was designed
for a single-tenant dev setup but the code is still in the prod build.

**Fix**
Drop the header fallback once P0-1 lands (the JWT path has authoritative
org_id from the membership table). Until then, gate the fallback on a
`DEV_MODE=true` env var.

### P2-2. Cache key on raw API key string keeps secrets in heap forever

**File:** `src/api_key_auth.rs:76,131-137`

`HashMap<String, CachedEntry>` holds the raw keys as String. A memory
dump or core file leaks every active key. The `Drop` for `String` zeroes
nothing.

**Fix**
Key the cache by SHA-256 of the raw key instead of the raw key itself.
Still O(1) lookup, no plaintext on the heap. Or use the `zeroize` crate
to wipe on eviction.

### P2-3. Cache miss-but-error: failed validations are cached too

**File:** `src/api_key_auth.rs:120-138`

The comment says this prevents stampedes, but caching `Err(())` for 60s
means: revoke a key, the user retries with the same key once, the failure
is cached, and even *if you re-issue them a new key with the same value*
(unlikely but possible during testing) they're locked out for a minute.
More realistically: a transient DB blip during validation gets cached as
a hard failure.

**Fix**
Cache hits only, or cache misses with a much shorter TTL (e.g. 1s) and
distinguish "key doesn't exist" from "DB error". Errors should never be
cached.

### P2-4. The contract cache poison policy is wrong for SaaS

**File:** `src/main.rs:139-149`

`.expect("contract cache RwLock poisoned")` is the right call for a
single-tenant local tool. For a multi-tenant SaaS gateway, panicking the
whole process because one mutex got poisoned takes down every customer.

**Fix**
Catch the `PoisonError`, log it, recover the inner value via
`into_inner()`, and continue. A poisoned RwLock just means *some* thread
panicked while writing — the data is still readable.

### P2-5. `tokio::spawn` for `last_used_at` update is fire-and-forget without back-pressure

**File:** `src/api_key_auth.rs:144-151`

Under load, every validated request spawns a detached task that does an
UPDATE. The pool can saturate, the spawn keeps happening, and you queue
unbounded tasks holding `db.clone()`.

**Fix**
Either batch the updates (write to a `mpsc::Sender<Uuid>` consumed by one
worker that flushes every 5s with `UPDATE ... WHERE id = ANY($1)`), or
use a Postgres `UNLOGGED TABLE` for the timestamps. The latter is
operationally simpler.

### P2-6. `delete_contract_handler` deletes without checking org membership

**File:** `src/main.rs:354-361`

`delete_contract` is called with no `org_id` filter. RLS on the DB will
catch it *if* the backend connects as `authenticated` — but the backend
uses the service role, so RLS is off and any user with a valid API key
can delete any contract by guessing UUIDs.

**Fix**
Add `org_id_from_req` to the handler and pass to a new
`delete_contract(pool, id, org_id)` that includes `AND org_id = $2` in
the WHERE clause. Apply the same pattern to every handler that mutates
a resource: `patch_contract_handler`, `delete_version_handler`,
`patch_version_handler`, all `*-ingress` handlers, etc.

### P2-7. `dashboard/Dockerfile` bakes secrets into the image

**File:** `dashboard/Dockerfile:14-19`

`NEXT_PUBLIC_API_KEY` ends up in `.env.production` inside the image
layer and in the bundled JS served to every browser. Anyone with
`docker pull` can extract it; every dashboard user already has it in
their devtools.

**Fix**
After P0-1 lands this env var goes away. Until then, document that this
key must be revocable and rotated per deploy.

### P2-8. Wide-open `service_role` policies are duplicates

**File:** `supabase/migrations/001_initial_schema.sql:116+`

Many tables have explicit `service_all` policies that grant the service
role full access. The service role bypasses RLS unconditionally in
Supabase — these policies are no-ops. Not a bug, just noise; remove for
readability.

### P2-9. `cache_read`/`cache_write` panic with `.expect` on mutex poison

See P2-4. Same anti-pattern repeated.

### P2-10. SWR errors are swallowed in dashboard

**Files:** `dashboard/app/contracts/page.tsx:1784-1787`,
`dashboard/app/audit/page.tsx:348-375`

`useSWR<ContractSummary[]>(org ? "contracts" : null, listContracts)` —
the `isLoading` flag is read but `error` is not displayed anywhere.
That's why the 401 looks like "loaded successfully with zero rows"
instead of "auth failed, try logging in again".

**Fix**
Surface `error` from every useSWR call. A single `<ErrorBanner>` at the
top of each page that reads from the most-likely-to-fail call is enough.

### P2-11. Frontend env var `NEXT_PUBLIC_API_URL` defaults disagree

**Files:** `dashboard/lib/api.ts:23`,
`dashboard/app/contracts/page.tsx:121`

`api.ts` defaults to `http://localhost:8080`, `contracts/page.tsx` to
`http://localhost:3001`. The server's `PORT` default in `main.rs:1140`
is `3001`. So api.ts's default is wrong. In production this is masked
by the env var being set, but in local dev the contracts list works and
the in-modal "ingest URL" panel is wrong (or vice versa).

**Fix**
Single source of truth — export a `BASE` constant from `lib/api.ts` and
import it everywhere. Default to `:3001` to match `main.rs`.

### P2-12. `useSWR` cache key collisions

**File:** `dashboard/app/audit/page.tsx:366`

`useSWR<AuditEntry[]>(org ? ["audit", contractFilter, page] : null, ...)`
— the SWR key includes `contractFilter` and `page` but `org` only
gates the request, not the cache key. If a user switches orgs (when
multi-org lands), they'll see the previous org's cached audit entries
flash before refetch. Fold `org.org_id` into every multi-org-sensitive
SWR key.

### P2-13. Dead/legacy schema columns

**File:** `supabase/migrations/001_initial_schema.sql:32-40`

`contracts.version`, `contracts.active`, `contracts.yaml_content` were
deprecated by the versioning RFC-002 migration but the columns are
still present. They get out of sync with `contract_versions`. Either
drop them or document them as "deprecated, do not write".

### P2-14. CLAUDE.md mentions a code-review-graph MCP that isn't installed

The project instructions in CLAUDE.md describe a `code-review-graph` MCP
with tools like `detect_changes`, `query_graph`, `get_impact_radius`.
Those tools are not in the current MCP registry. Either drop the
section, or install the MCP and verify the tools exist.

### P2-15. 38 RFCs and no consolidated "current state" doc

`docs/rfcs/` has 38 numbered RFCs (some duplicated numbers — two 001s).
A new contributor cannot quickly tell which ones are shipped vs.
proposed vs. abandoned. The `MAINTENANCE_LOG.md` helps, but a single
`docs/STATUS.md` table (RFC#, title, status, shipped-in-nightly) would
prevent the next person from re-implementing already-shipped features.

### P2-16. Inconsistent migration style — case + quoting

`001_initial_schema.sql` is UPPERCASE SQL. `006`+ are lowercase. `019`
is mixed. Postgres doesn't care, but human readers do. Add a `sqlfluff`
config to CI and pick a style.

### P2-17. `last_used_at` updates fail silently

**File:** `src/api_key_auth.rs:146-149`

The `let _ = sqlx::query("UPDATE ...")` discards the result. If the
column gets renamed or the row vanishes, every key validation silently
no-ops the update and the "Last used" column in the dashboard goes
permanently stale, masking a real bug.

**Fix**
At least `tracing::warn!` on error.

### P2-18. Dashboard runs all reads through the Rust backend, except when it doesn't

Mixed pattern: `account/page.tsx` reads `api_keys` via Supabase REST
directly; `contracts/page.tsx` reads `active_contracts_public` via
Supabase REST; everything else goes through the Rust API. The two
paths have different auth models (Supabase JWT vs. backend API key)
which is why the bug in P0-1 shows up as "I can see api_keys but not
contracts." Pick one path per table and document.

---

## P3 — Polish + nice-to-have

- The "Press Esc to close" footer in the New Contract wizard is fine,
  but the modal swallows global Cmd+S — add a Save shortcut.
- `transformed_event: null` is impossible per the docstring but the
  type says `unknown`. Tighten to `Record<string, unknown> | unknown[]`.
- `inferUrl` should surface the upstream HTTP status as `502 Bad
  Gateway` when the upstream is reachable but returns 5xx, not as a
  400 (looks like the client's fault).
- The `/health` endpoint returns hardcoded `"status": "ok"` even when
  the DB is unreachable. Make it run a `SELECT 1` first.
- `Cargo.toml` has both `metrics-exporter-prometheus` with a custom
  http-listener AND the `/metrics` Axum route. One of these is
  redundant.
- `examples.ts` `EXAMPLE_YAML` should be the *exact* YAML from
  `CLAUDE.md` so docs and dashboard agree.

---

## Summary order of operations

1. **Ship P0-1 first** — nothing else matters if users can't log in
   and see their own data. This unblocks Alex's current bug.
2. **Then P0-2 and P0-4** — close the multi-tenancy leaks before any
   second customer touches the system.
3. **P0-3 is a code/doc mismatch** — fix in the same PR as P0-4.
4. **Roll P1-1 through P1-3 together** — rate limit + body limit +
   playground hardening are the abuse-prevention bundle.
5. **P1-6 (SSRF redirect) is small but important** — ship it standalone.
6. **P2 items** can be batched as the next 2-3 nightly maintenance runs.

When handing to Sonnet, paste one P-item per turn — don't try to do
multiple at once. Each one needs its own RFC under `docs/rfcs/` per the
project's RFC-first rule.
