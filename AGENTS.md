# AGENTS.md — DataContractGate / AISchemaGate handoff

Operational handoff for any coding agent picking up this repo. `CLAUDE.md` holds the coding rules and the locked contract format;
`PROJECT_STATUS.md` and `docs/STATUS.md` track RFCs. This file covers what those
don't: how production is wired, how to ship, what is in flight, and the traps
that already cost us time. Last updated **2026-10-18**.

Public repo: no secrets, keys, customer data or personal emails in this file or
anywhere in git. Accounts are named by role below.

---

## 1. Products and naming

- **DataContractGate** is the formal product name everywhere user-facing (owner:
  NightMoose, Inc.). Never write bare "ContractGate" in product text — there is
  an unrelated company at contractgate.io. Package identifiers (`contractgate`
  crate/PyPI, `@nightmoose/contractgate-*` npm) stay as they are pending an
  attorney check.
- **AISchemaGate** is the sister brand for LLM-output validation. Same backend,
  accounts, API keys and dashboard; its own marketing site and docs, an alias
  SDK package, and AISchemaGate branding/trimmed nav inside the dashboard.

## 2. Production topology

| Piece | Where | Notes |
|---|---|---|
| API (Rust/Axum, this repo `src/`) | Fly app `contractgate-api`, region iad, shared-cpu, 256 MB, `min_machines_running = 1` | Public host `https://api.datacontractgate.com`. `CANONICAL_API_ORIGIN` must be that host. `KAFKA_ENABLED=false` (Confluent account closed; enable only with a broker and `CONFLUENT_*` secrets). |
| Staging API | Fly app `contractgate-api-staging`, `fly.staging.toml`, scale to zero | `https://contractgate-api-staging.fly.dev`. Shares the production Supabase database — staging writes are real data. `CANONICAL_API_ORIGIN` must be the staging host. No Confluent or AWS secrets. |
| Old API host | `contractgate-api.fly.dev` | **Retired 2026-10-14**: Fly secret `LEGACY_API_HOSTS=contractgate-api.fly.dev` makes every request except `/health` and `/ready` return 308 to the api host (`error::redirect_legacy_hosts`). |
| Dashboard (Next.js 16, `dashboard/`) | Vercel project `contractgate` | `https://app.datacontractgate.com`. Prod env `NEXT_PUBLIC_API_URL=https://api.datacontractgate.com` (inlined at build — see §8). `dashboard/proxy.ts` rewrites `/v1/*` and `X-Api-Key` requests to `CONTRACTGATE_API_ORIGIN` and sets the brand cookie. |
| Database / auth | Supabase project `nmhoehpveqkkpfegkzpn` ("contract gate") | Migrations in `supabase/migrations/` (`001`–`043` in tree). `pg_cron` job `purge-expired-logs` hourly at :17. |
| DataContractGate marketing | repo `nightmoose/datacontractgate_website`, Vercel | Static fragments `sa1/sa2/frag1-6.html`; `scripts/build.mjs` (Vercel buildCommand) inlines them into `index.html` so the served HTML has real content. `.vercelignore` keeps internal `marketing/` docs off the site. |
| AISchemaGate site + docs | repo `nightmoose/aischemagate-www` (private), Vercel | aischemagate.com (landing/pricing) and aischemagate.dev (docs; `proxy.ts` rewrites `/` → `/docs`). Docs are copied from this repo by `DOCS_REF=vX.Y.Z npm run sync-docs` — **manual**, rerun after editing `docs/llm-*.md`. |
| Status app | repo `nightmoose/contractgate-status-ios` | iOS status app. |
| Billing | Stripe (live) | One shared product "Pro — DataContractGate & AISchemaGate", monthly + annual prices. Keys/price IDs live only in Vercel env. |

**Accounts (who owns what).** Fly: the **NightMoose** Fly account owns
`contractgate-api`; the CLI on the dev Mac is shared with other projects and is
often signed into a different (Fullscore) account — `Error: unauthorized` on
`fly … -a contractgate-api` means wrong account (fix: sign the browser into the
NightMoose account, `fly auth login`, check `fly apps list`). GitHub org
`nightmoose`. npm scope `@nightmoose` and PyPI publish via OIDC trusted
publishing (no tokens).

## 3. Plans and limits (single source: `src/plan.rs`)

| Plan values | Display | Events/month | Rate limit | Log retention |
|---|---|---|---|---|
| `free`, `aisg_free` | Free / AISchemaGate Developer | 100,000 | 10 req/s, burst 20 | 7 days |
| `growth`, `aisg_pro` | Pro / AISchemaGate Pro | 25,000,000 | 100 req/s, burst 200 | 90 days |
| `enterprise` | Enterprise | unlimited | 100 req/s, burst 200 | 365 days |

Pro is **$199/mo or $166/mo billed annually ($1,992/yr)**, 30-day trial, same
for both brands. `growth` is the legacy value name for Pro (display "Pro").
Retention is enforced by `purge_expired_logs()` (migration 037).

Rate-limit headers: `X-RateLimit-Limit` = burst capacity, `-Remaining` = tokens
left, `-Reset` = when the bucket is full again; added to every authenticated
response by `require_api_key` (`with_rate_limit_headers`). To trip a Free 429
you need >20 requests inside ~1 s; slower traffic refills at 10/s.

## 4. Shipping

**Branches.** Feature work branches off an up-to-date `origin/staging` and
opens a PR into `staging`. Squash-merge that PR to one `WORKING:` commit.
Check `gh pr list` and `git branch -r` before editing the same files, and
never push to someone else's branch.

**Promoting to production.** Open a PR from `staging` into `main` and merge it
with a merge commit. The `main` ruleset allows only merge commits, requires
one approval and signed commits, and blocks force-push and deletion. The owner merges that PR. `deploy-fly` runs only on
`refs/heads/main`.

**CI (`.github/workflows/ci.yml`).** Runs on `main` and `staging`. A change
that is only `docs/**`, `examples/**`, or `**/*.md` skips the heavy jobs.
A markdown file at the repo root (this file) still counts as heavy: the
exclude pattern needs a slash. The `docker` job builds the API image once
(GHA layer cache). On a push it tags `registry.fly.io/contractgate-api:<sha>`
(main) or `registry.fly.io/contractgate-api-staging:<sha>` (staging).
`deploy-fly` ships the prod image (`flyctl deploy --image`).
`deploy-fly-staging` ships `flyctl deploy -c fly.staging.toml --image …`.
Compose smokes reuse the cached image (`SKIP_BUILD=1`). `cancel-in-progress`
is PR-only so every merge to main deploys. Merge → live ≈ 10 min. Confirm a
deploy with `curl https://api.datacontractgate.com/health` → `build` = short
SHA of main. Staging health is `https://contractgate-api-staging.fly.dev/health`.

**Dockerfile dependency layer** stubs every Cargo target and builds with the
same bins/features as the real build (`CARGO_BUILD_ARGS`). If you add a
`[[bin]]` or change features, update the stub list or the cache silently stops
working (it was broken that way until 2026-10-13).

**Releases (`.github/workflows/release.yml`).** All packages move in lockstep:
`Cargo.toml`, `Cargo.lock`, `dashboard/package.json`(+lock), `mcp/package.json`
(+lock) + `mcp/server.json` (2 places), `sdks/python/pyproject.toml` +
`_version.py`, `sdks/typescript/package.json`(+lock) + `SDK_VERSION` in
`src/client.ts`, and the aliases `sdks/aischemagate/{python,typescript}`
(version, `__version__`/`AISCHEMAGATE_VERSION`, their tests, and the minimum
base-SDK version they require). Add CHANGELOG entries. Merge, **then** tag:
`git fetch && git tag vX.Y.Z origin/main && git push origin vX.Y.Z` (tagging
before the merge lands fails the version check — delete and re-tag). Every
publish step skips versions that already exist, so re-running a release is safe.
Transient failures seen: MCP Registry before npm shows the new version, macOS
runners queued until cancelled → `gh run rerun <id> --failed`.
Current version: **0.4.0** (crate, PyPI `contractgate` + `aischemagate`, npm
`@nightmoose/contractgate-sdk`, `@nightmoose/aischemagate`,
`@nightmoose/contractgate-mcp-server`, MCP Registry).

**Connect Fivetran and self-host.** `POST /v1/integrations/fivetran/connect`
(`src/fivetran.rs`) uploads `vendor/fivetran-connector/` into the caller's
Fivetran account and creates a paused connection. Fivetran credentials are
not stored. The gateway mints an org-scoped key named `Fivetran connector`.
Paid hosted plans (`growth`, `aisg_pro`, `enterprise`) may connect. So may
any process with `CONTRACTGATE_SELF_HOSTED` set to `1`, `true`, or `TRUE`.
Hosted `free` and `aisg_free` get 403. The flag does not change the stored
plan: `GET /v1/me` (`src/me.rs`) keeps `plan` and sets `paid_features: true`.
The same flag unlocks `GET /v1/contracts/{id}/passed` and payload storage.
Hosted production and staging leave the flag unset. Connect returns 500 and
mints no key when `CANONICAL_API_ORIGIN` is unset or not an https origin.
The dashboard button is on Contracts and follows `paid_features`. Guide:
`dashboard/app/docs/fivetran/page.tsx`. Reference:
`docs/fivetran-connect-reference.md`. `make demo` and the stock
`docker-compose.yml` are the auth-off local demo (`CONTRACTGATE_DEV_NO_AUTH=1`,
`CONTRACTGATE_SELF_HOSTED` left unset). A self-host server that Fivetran can
reach keeps auth on, sets `CONTRACTGATE_SELF_HOSTED=true`, and sets
`CANONICAL_API_ORIGIN` to a public https origin. That setup is on the
Fivetran guide: `dashboard/app/docs/fivetran/page.tsx`.

**Database migrations.** Add `supabase/migrations/NNN_description.sql`
(contiguous numbering — CI enforces it; next number is `044`) and a CI sentinel in
`migrations-check`. Test against a throwaway `postgres:16` with all migrations
applied. **Production migrations need the owner's explicit approval each time**;
apply with the Supabase MCP `apply_migration` (that also records it in
`supabase_migrations.schema_migrations` — raw SQL does not, which is how 038
once went unrecorded). Supabase-specific grants (`authenticated` execute on
`get_my_org_ids()`, table grants) exist in prod but not in plain Postgres; add
them in local test setups.

**Docs.** User-facing behavior changes update the matching `docs/*-reference.md`.
`docs/llm-*.md` feed aischemagate.dev (resync www) and the dashboard
(`dashboard/scripts/sync-llm-docs.mjs`).

## 5. Code map (the parts touched most recently)

- `src/main.rs` — router, `require_api_key` (JWT for the dashboard, `x-api-key`
  for SDKs; rejects `Bearer cg_…` with a hint), middleware order
  (outermost first: security headers → legacy-host redirect → timeout → trace →
  metrics → JSON error bodies → routes).
- `src/error.rs` — `AppError` → `{"error","status"}`; `json_error_bodies`
  makes every error JSON (body-limit 413s, extractor 400/415s, unmatched 404/405);
  `redirect_legacy_hosts`.
- `src/rate_limit.rs` — per-key token bucket, **in memory per machine** (see §8).
- `src/v1_ingest.rs` — `POST /v1/ingest/{id}` (preferred route; returns `index`
  + `quarantine_id`; quarantine ids only when a row is really written — never on
  `dry_run` or atomic rejection; 415 for non-JSON content types) and the
  OpenAPI doc (`/openapi.json`, lists `/v1/ingest` and `/egress`).
- `src/egress.rs` — `POST /egress/{id}`, used by AISchemaGate's LLMGate. 422
  when every record fails, but the body is still a full egress response.
- `src/ingest.rs` — legacy `POST /ingest/{id}`; SDKs ≥ 0.4.0 no longer use it.
- `src/infer*.rs` — contract inference. Must accept its own samples: nulls →
  `null_as_absent: true`; enums only with ≥5 observations and repeats; numbers
  only `min: 0` when non-negative; strings only `min_length: 1`; array items
  inferred like fields. Round-trip tests in `infer.rs`.
- `src/storage/contracts.rs` — `deploy_contract_version` resolves the name
  **within the caller's org and live (non-deleted) rows only**.
- `src/fivetran.rs`, `src/plan.rs` (`gate_paid`, `CONTRACTGATE_SELF_HOSTED`),
  `src/me.rs` (`paid_features`), `src/passed_events.rs` — see §4.
- `src/validation.rs` — `received` echoes over 1 KB are shortened; type-mismatch
  suggestions name real settings (`type: number`, `required: false` +
  `null_as_absent: true`).
- `sdks/python`, `sdks/typescript` — `Client` (`ingest` → `/v1/ingest`, returns
  results for all-failed/atomic 422s) and `LLMGate` (`llm` optional for
  `validate_only`; failed validation → `ok=False`, not an exception; contract
  name or UUID).
- `dashboard/` — `components/BrandProvider.tsx` + `lib/brand.ts` (brand cookie
  `cg_product`, sticky `?product=aischemagate`), `components/Sidebar.tsx`
  (`AISG_HIDDEN`), `components/help/FirstRun.tsx` (brand-aware empty states),
  `app/pricing/{page,AisgPricing}.tsx`, Stripe routes under `app/api/stripe/`.

## 6. Current state (2026-10-18)

Production is `main`. Feature work lands on `staging` first; the owner promotes
it with a merge commit. The latest promotion on main is PR #312: Connect
Fivetran, the self-host paid-feature flag, and the dashboard gate on
`paid_features`. Migrations in the tree run through 043. Collaboration rows
are keyed by contract id (042). Package version is still **0.4.0**.
`KAFKA_ENABLED` is false.

Test reports live outside the repo on the owner's machine
(`~/Desktop/ContractGate-E2E/`).

## 7. Open work

**Next / launch-adjacent**
- Rate limiting is per machine (§8): move buckets to a shared store, or
  divide per-machine limits by machine count.
- SDK 0.4.1: add `passed` as an alias of `ok` on `LLMGateResult` (Py + TS);
  bundle with the next fixes. `LLMGateResult` still has `ok` only.
- B5: `LLMGate.completion(auto_repair=True)` against a real OpenAI/Anthropic
  key — never tested end to end.

**Polish round (after the announcement)**
- Marketing site talks only about Kafka; add AISchemaGate.
- www and app send only HSTS: add CSP, `X-Content-Type-Options`,
  `X-Frame-Options` (the API already sends them).
- www missing `robots.txt`/`sitemap.xml`.
- 3 orgs have two live contracts with the same name; clean up, then add a
  partial unique index on `(org_id, name) where deleted_at is null`.
- Pinned versions on `/v1/ingest` use `?version=` (the documented pin). `{id}@version` also works.
- `@nightmoose/aischemagate` exports map omits `package.json`.
- Kafka/Kinesis wording still appears in a few DataContractGate-only places
  (fine for DCG, hidden for AISG).

**Backlog**
- Zapier connector 1.1.1 (DataContractGate wording) once 1.1.0 is approved.
- YAML library migration (RUSTSEC-2024-0449 is ignored in `deny.toml` until then).
- AISchemaGate near-term TODO in `PROJECT_STATUS.md` (drift alerting,
  SOC2/HIPAA/BAA, local Rust core, metered overage, contract-count limit
  enforcement — the "3 contracts" Free limit is shown but not enforced).
- Attorney: "protected from third-party IP claims" line on the DCG pricing FAQ;
  package renames; AISchemaGate name.
- Stripe tidy-up: archive old $299/$2,988/$99 prices, check statement descriptor.

## 8. Traps we already hit

- **`NEXT_PUBLIC_*` is baked in at build time.** Changing it in Vercel does
  nothing until a fresh build that starts *after* the change (redeploy with
  "Use existing Build Cache" off). Verify by grepping the served JS chunks.
- **In-memory rate limiter** (`DashMap` on `AppState`) is per machine. Extra
  machines multiply the advertised limit. The key cache (60 s TTL) is per
  machine too. `fly.toml` sets `min_machines_running = 1`.
- **Test mocks must match the server.** SDK bugs shipped because mocks returned
  200 for failed batches and contract rows with fields the API doesn't send.
  Mock the real status codes and shapes (422 on all-failed, `ContractSummary`).
- **Unscoped lookups by name** (`WHERE name = $1`) leak across orgs and match
  soft-deleted rows. Always add `org_id` and `deleted_at IS NULL`.
- **RLS recursion**: a policy on table A that queries B, whose policy queries A,
  fails every authenticated read with 42P17 (migration 039). Put cross-table
  lookups in `SECURITY DEFINER` helpers (`get_my_org_ids`,
  `get_my_org_contract_names`).
- **Don't hide failures**: `|| true` hid a broken Docker cache for months;
  `continue-on-error` once hid a broken PyPI publisher.
- **Alias npm package** must install the base SDK from the registry, not a
  local `file:` lockfile link (release job uses `--no-package-lock`).
- **PyPI trusted publishing**: two pending publishers with identical claims
  collide — the alias uses its own GitHub environment `pypi-aischemagate`.
- **Fly CLI account** — see §2. `fly secrets set` restarts the machines.
- **Docker cleanup**: remove test containers by name; `docker container prune`
  deletes every stopped container on the machine.
- Production DB queries return customer data: count and hash, don't print
  emails or payloads.
