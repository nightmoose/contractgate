# ContractGate — Public-Launch User Test Plan

**Date:** 2026-05-24
**Targets:** Production
- API: `https://contractgate-api.fly.dev`
- Dashboard: `https://app.datacontractgate.com`

**Purpose:** Verify ContractGate is ready for public launch now that all
launch-readiness findings (RFC-047 through RFC-057) are addressed. This plan
exercises the API, CLI, dashboard, and database/RLS isolation, and includes
explicit regression tests for every P0/P1/P2 fix from the
`REVIEW-2026-05-22-launch-readiness.md` audit.

This plan is the human-readable specification. The exact commands an executor
runs live against production are in the companion file
`HAIKU-TEST-PROMPTS-2026-05-24.md`.

---

## 1. Scope

In scope: API endpoints, validation engine, CLI, dashboard UI, and database /
RLS / org-isolation behavior, on the production deployment.

Out of scope: Kafka and Kinesis ingress (require external broker/stream
infrastructure), load/soak testing, and self-hosted deployment mode.

---

## 2. Environment & preconditions

The executor must be given these values before starting. Keys must be
**DB-backed** (`cg_live_…`) — the legacy env-var key carries no org context and
is rejected on org-scoped routes by design.

| Name | Description | Used by |
|---|---|---|
| `BASE` | `https://contractgate-api.fly.dev` | all API/CLI suites |
| `DASHBOARD` | `https://app.datacontractgate.com` | Suite 8 |
| `KEY_A` | DB-backed API key for **Org A** | Suites 2–7 |
| `KEY_B` | DB-backed API key for a **different Org B** | Suite 2 (cross-org isolation) |
| `JWT` | Supabase session token (optional) | Suite 2 (JWT auth path) |
| `LOGIN_A` | Dashboard email + password for Org A | Suite 8 |
| `CG_BIN` | Path to a pre-built `contractgate` CLI binary | Suite 7 |

`KEY_A` and `KEY_B` are pre-provisioned: two `cg_live_…` DB-backed keys
belonging to two dedicated empty orgs (**QA Test Org A** / **QA Test Org B**)
created only for this run. The exact key values are in
`HAIKU-TEST-PROMPTS-2026-05-24.md`. Because the orgs are isolated, the run
touches no real customer data and can be fully cleaned up by deleting the two
orgs afterward.

Build the CLI binary before Suite 7 (the executor cannot run `cargo`):
`cargo build --release --bin contractgate` → `target/release/contractgate`.

Tooling the executor needs: a shell with `curl`, `python3`, and `jq`; the
Chrome browser extension for Suite 8.

---

## 3. Conventions

**Priority:** P0 = launch blocker, must pass. P1 = high, fix before launch.
P2 = medium, track but not blocking.

**Pass criteria:** a test passes only if the observed HTTP status **and** the
checked response content both match the Expected column. A 5xx on any test is
an automatic fail. Record actual status + a short response excerpt for every
test, pass or fail.

**Cleanup:** test contracts are created with names prefixed `qa_` and may be
left in place or soft-deleted at the end of the run; they do not affect other
tenants.

---

## 4. Test suites

### Suite 1 — Infrastructure & Health

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 1.1 | `GET /health` | 200; JSON `status:"ok"`. RFC-053: endpoint now runs a real DB probe, so a 200 means the DB pool is live. | P0 |
| 1.2 | `GET /ready` | 200 when DB reachable (readiness probe, `SELECT 1`). | P0 |
| 1.3 | `GET /metrics` | 200; Prometheus text format; contains an HTTP request metric. | P1 |
| 1.4 | `GET /openapi.json` | 200; valid JSON with an `openapi` field. | P1 |
| 1.5 | `GET /catalog` (public, no auth) | 200; JSON array. Confirms a public route answers without a key. | P1 |

### Suite 2 — Authentication & security regressions

Each test here maps to a specific launch-blocker fix. These are the highest-priority tests in the plan.

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 2.1 | `GET /contracts` with **no** API key | 401 Unauthorized. | P0 |
| 2.2 | `GET /contracts` with a **malformed** key (`cg_live_bogus`) | 401 Unauthorized. | P0 |
| 2.3 | **Cross-org IDOR (RFC-047):** create a contract with `KEY_A`, then `GET /contracts/{id}` with `KEY_B` | 404 Not Found (never 403, never 200 — UUID existence is not revealed). Same id with `KEY_A` → 200. | P0 |
| 2.4 | **Cross-org write (RFC-047):** `PATCH` and `DELETE /contracts/{id}` of Org A's contract using `KEY_B` | 404 Not Found; contract still intact when re-read with `KEY_A`. | P0 |
| 2.5 | **`x-org-id` ignored (RFC-048):** repeat 2.3's `GET /contracts/{id}` with `KEY_A` plus a forged `x-org-id: <random uuid>` header | 200 — header has no effect; org still resolved from the key. | P0 |
| 2.6 | **CORS disallowed origin (RFC-050):** `OPTIONS /contracts` with `Origin: https://evil.example.com` | No `Access-Control-Allow-Origin` header echoing that origin. | P0 |
| 2.7 | **CORS allowed origin (RFC-050):** `OPTIONS /contracts` with `Origin: https://app.datacontractgate.com` | `Access-Control-Allow-Origin` present for that origin; `GET, POST, PATCH, DELETE, OPTIONS` allowed. Public route `/health` returns `*`. | P0 |
| 2.8 | **SSRF block (RFC-049):** `POST /contracts/infer/url` with `{"url":"http://169.254.169.254/latest/meta-data/"}` | Rejected (4xx); no metadata content returned. A URL that 302-redirects to a private IP is also rejected (redirects are re-checked). | P0 |
| 2.9 | **JWT auth path (optional):** `GET /contracts` with `Authorization: Bearer <JWT>` | 200; org resolved from the token's primary org membership. | P1 |

### Suite 3 — Contract lifecycle (API)

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 3.1 | `POST /contracts` with valid contract YAML | 201/200; response has `id` and `name`. | P0 |
| 3.2 | `GET /contracts` | 200; list contains the contract from 3.1. | P0 |
| 3.3 | `GET /contracts/{id}` | 200; returns the contract identity. | P0 |
| 3.4 | `PATCH /contracts/{id}` (e.g. change description) | 200; change reflected on re-read. | P1 |
| 3.5 | `GET /contracts/{id}/versions` | 200; at least one version returned. | P1 |
| 3.6 | `POST /contracts/deploy` (RFC-028 atomic deploy) | 200/201; version lands as stable, prior stable deprecated (exercises the promote/deprecate path). | P1 |
| 3.7 | `POST /contracts` with **malformed** YAML | 400 Bad Request with a parse error message; no 5xx. | P1 |
| 3.8 | `DELETE /contracts/{id}` (soft delete) | 204 (or 200); contract no longer returned by `GET /contracts/{id}` (404). | P1 |

### Suite 4 — Validation engine & ingestion

Uses a fresh `qa_` contract with the locked semantic-contract format
(`user_id` string+pattern, `event_type` enum, `timestamp` integer,
`amount` number+min). The contract is created via `POST /contracts/deploy` so
it has a stable version — ingesting against a draft-only contract returns 409.

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 4.1 | Ingest a valid `click` event | 200; `passed:1, failed:0`. | P0 |
| 4.2 | Ingest event missing required `user_id` | 200; `failed:1` with a "required" violation. | P0 |
| 4.3 | Ingest event with `event_type` outside the enum | 200; `failed:1` with an enum violation. | P0 |
| 4.4 | Ingest event with `user_id` breaking the regex pattern | 200; `failed:1` with a pattern violation. | P0 |
| 4.5 | Ingest `purchase` event with negative `amount` | 200; `failed:1` (min constraint). | P0 |
| 4.6 | Ingest a mixed batch (2 valid, 2 invalid) | 200; `passed:2, failed:2`. | P0 |
| 4.7 | Ingest with `?dry_run=true` | 200; `dry_run:true`; no rows written (stats unchanged). | P1 |
| 4.8 | `GET /ingest/{id}/stats` | 200; `total_events` reflects the non-dry-run ingests above. | P1 |
| 4.9 | `date` field type (RFC-044): contract with a `date` field, ingest a valid `YYYY-MM-DD` value and an invalid one | valid → `passed`, invalid → `failed`. | P1 |
| 4.10 | Latency: 30 sequential ingests, then read `/ingest/{id}/stats` | All 200; wall-clock stable, no outlier > 2s; `p99_validation_us` < 15000 (the <15 ms server-side engine budget, reported by the stats endpoint). | P1 |

### Suite 5 — Inference & playground

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 5.1 | `POST /playground/validate` with YAML + a valid event | 200; `passed:true`. Confirms playground is auth-gated (RFC-042) and works. | P1 |
| 5.2 | `POST /playground/validate` with YAML + an invalid event | 200; `passed:false` with violations. | P1 |
| 5.3 | `POST /contracts/infer` with 2+ JSON samples | 200; response has `yaml_content`, `field_count`, `sample_count`. | P1 |
| 5.4 | `POST /contracts/infer/csv` with `csv_content` | 200; `yaml_content` produced from the CSV header/rows. | P1 |
| 5.5 | `POST /contracts/infer` with an empty `samples` array | 400 Bad Request ("at least one sample is required"); no 5xx. | P2 |

### Suite 6 — Audit, stats & catalog

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 6.1 | `GET /audit?contract_id={id}` | 200; entries exist for the Suite 4 ingests. | P1 |
| 6.2 | **Audit honesty:** in 6.1's entries, `contract_version` equals the version that actually matched each event — never a default/placeholder. | Version field is the real matched version. | P0 |
| 6.3 | `GET /stats` | 200; org-scoped totals returned. | P1 |
| 6.4 | `GET /public-contracts` and `GET /public-contracts/{id}` | 200; curated open-data contracts list and fetch with no auth. | P1 |
| 6.5 | Audit is org-scoped: `GET /audit` with `KEY_B` does not return Org A's contract entries. | No Org A rows visible to Org B. | P0 |

### Suite 7 — CLI

Requires the pre-built `contractgate` binary (`CG_BIN`). `validate`, `infer`,
and `scaffold` are offline; `deploy-contract` and `push`/`pull` need the
gateway and a key.

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 7.1 | `contractgate --version` and `--help` | Prints version and the subcommand list (deploy-contract, push, pull, validate, scaffold, enforce, infer). | P1 |
| 7.2 | `contractgate validate <good.yaml>` | Exit 0; reports the contract compiles. | P1 |
| 7.3 | `contractgate validate <bad.yaml>` | Non-zero exit; parse/compile error printed. | P1 |
| 7.4 | `curl … \| contractgate infer --from-stdin --name qa_users` | Exit 0; emits draft contract YAML. | P1 |
| 7.5 | `contractgate scaffold --from-file <samples.json> --name qa_events` | Exit 0; draft contract written. | P2 |
| 7.6 | `contractgate deploy-contract <good.yaml> --dry-run` (with `CONTRACTGATE_API_KEY=KEY_A`) | Exit 0; dry-run reports what would deploy, writes nothing. | P1 |
| 7.7 | `contractgate deploy-contract` / `push` with **no** key | Exit 11; "API key required" message. | P1 |

### Suite 8 — Dashboard UI

Run with the Chrome browser extension against `DASHBOARD`. Requires `LOGIN_A`.

| ID | Test | Expected result | Pri |
|---|---|---|---|
| 8.1 | Load `/auth/login`, sign in with `LOGIN_A` | Lands on an authenticated page; no console errors. | P0 |
| 8.2 | Open `/contracts` | Contract list renders; search/filter controls work (RFC-028). | P1 |
| 8.3 | Open `/playground`, paste a contract + event, run validation | Pass/fail result shown in the UI. | P1 |
| 8.4 | Open `/catalog` | Public catalog renders; a contract can be opened. | P1 |
| 8.5 | Open `/account` → API Keys; issue a new key | Key shown exactly once; issuance goes through the server route (RFC-056) — no raw key generated client-side. | P0 |
| 8.6 | In `/account`, revoke the key from 8.5 | Key marked revoked; reusing it on the API → 401. | P1 |
| 8.7 | Open `/audit` and `/scorecard` | Pages load and render data without errors. | P2 |
| 8.8 | Plan gating (RFC-045): as a Free-tier user, open a Growth-only feature (e.g. Visual Builder / From CSV / GitHub sync) | Upsell card shown instead of the feature; no crash. | P2 |
| 8.9 | CORS in practice: dashboard API calls succeed from `app.datacontractgate.com` | Network tab shows authenticated calls returning data, no CORS errors. | P1 |

---

## 5. Reporting & exit criteria

For each test, the executor records: test ID, Pass/Fail, observed HTTP status
(or UI outcome), and a one-line response excerpt. The run ends with a summary
table and counts.

**Launch gate:**
- All **P0** tests must pass. Any P0 failure blocks launch.
- P1 failures are triaged individually; more than two open P1 failures blocks launch.
- P2 failures are logged as follow-ups.

**Special attention — regression checks that must pass:**
2.3 / 2.4 (RFC-047 IDOR), 2.5 (RFC-048 `x-org-id`), 2.6 / 2.7 (RFC-050 CORS),
2.8 (RFC-049 SSRF), 1.1 (RFC-053 health probe), 6.2 (audit honesty),
8.5 (RFC-056 server-side key issuance). These are the fixes the launch
readiness review was built around; a regression in any of them is a P0.
