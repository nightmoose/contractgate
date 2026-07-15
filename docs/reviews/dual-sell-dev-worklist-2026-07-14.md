# Dual-Sell Dev Worklist — Support Customer Sales *and* Asset Sale

**Date:** 2026-07-14  
**Author:** Grok (session review of ContractGate)  
**Repo:** `nightmoose/contractgate`  
**Audience:** Human + Claude (review / implement / challenge priorities)  
**Context:** Work that advances **both** (A) selling ContractGate to customers and (B) selling the company/product as an asset. Prefer intersection work; defer pure feature expansion.

---

## Framing

### Two “sell” paths

| Path | What success looks like |
|---|---|
| **A. Customer sale** | Design partner pilot → paid Growth / Enterprise; clear value (bad events blocked) |
| **B. Asset / acquisition sale** | Diligence-ready: isolation proof, IP clarity, ops handoff, commercial spine, small pipeline/ARR |

### What *not* to lead with

- 80 RFCs, ODCS depth, full feature matrix  
- Net-new surface (SSO/SAML, open-core split, Terraform, K8s operator, Java/Go SDKs, marketplace, RAG profile) unless a **named deal** requires it  

### Product one-liner (for both paths)

> Stop bad events **before** they hit the warehouse — semantic contracts at ingest, with quarantine/replay and sub‑ms validation.

### Guiding principles

1. **Trust before reach** — isolation and abuse resistance before public signup volume.  
2. **Commercial spine before features** — quotas/metering make tiers real for customers and real for acquirers.  
3. **One hero path** — boring, demoable HTTP (or Kafka) story beats breadth.  
4. **Evidence over assertion** — CI with auth **on** beats “we fixed it in code review.”  
5. Protect the validation engine (`validation.rs`) — never regress p99 / panic-free hot path.

---

## Current state (relevant as of 2026-07-14)

### Already strong

- Rust validation engine (compile-once / validate-many, well documented).  
- Broad product surface: HTTP/Kafka/Kinesis, quarantine/replay, scorecard, CLI, Python SDK, dashboard, `make demo`.  
- Process maturity: RFCs, `MAINTENANCE_LOG.md`, coverage ratchet, Dependabot, launch/sale readiness reviews.  
- Multi-tenant hardening RFCs 047–074 largely landed (IDOR, CORS, SSRF, key scope, legacy `API_KEY` gated).  
- Stripe Growth checkout + webhooks exist (`docs/stripe-billing-reference.md`).  
- Plan gating **UI** exists (RFC-045); backend event quotas still soft.  
- Migration `031_security_advisor_fixes.sql` **exists in repo** (apply/verify on prod still operator work).  

### Known gaps that block both sells

- **RFC-075 still Draft** — compose-smoke isolation was a false green under `CONTRACTGATE_DEV_NO_AUTH=1`. Need auth-on isolation lane.  
- **Supabase advisor / multi-tenant residual** — migration 031 written; prod apply + advisor recheck.  
- **Event quotas / metering** — pricing table promises Free 1M / Growth 50M; enforcement + usage UI incomplete.  
- **Ops runbooks / security one-pager / data-room index** — thin relative to product depth.  
- **Dual ingest paths** (`/ingest` vs `/v1/ingest`) — diligence and maintenance tax.  
- **Pilot proof artifacts** — no exportable “blocked N bad events” report pack.

### Anchors already in-repo

| Topic | Anchor |
|---|---|
| Auth-on isolation | RFC-075 (Draft); related 068/073/074 |
| Advisor / RLS fixes | `supabase/migrations/031_security_advisor_fixes.sql`; `docs/punchlist/2026-07-10-sonnet-worklist.md` item 1 |
| Prod migration drift CI | Worklist item 2; `.github/workflows/migration-drift.yml` (if present) |
| Plan tiers (UI) | RFC-045; `docs/plan-gating-reference.md` |
| Metering roadmap theme | RFC-058 Q4 |
| Stripe | `dashboard/app/api/stripe/`; migration 028/029 |
| Sale readiness (May) | `docs/reviews/sale-readiness-review-2026-05-28.md` |
| Launch readiness (May) | `REVIEW-2026-05-22-launch-readiness.md` (many items since closed) |
| July maintenance | `REVIEW-2026-07-09-maintenance-sweep.md` |

---

## Priority worklist

Do **in sequence**. P0 before P1 before P2. Refactors (Kafka/Kinesis DRY, oversized `main.rs`) are good engineering but **low dual-sell ROI** until P0/P1 land.

### P0 — Trust (data room + pilot SOW)

| # | Work | Customer value | Acquirer value | Size | Notes / acceptance |
|---|---|---|---|---|---|
| **1** | **Apply + verify migration 031** on staging → prod | Safe multi-tenant cloud | Closes advisor ERRORs in diligence | Ops (file ready) | Re-run Supabase advisors; confirm SECURITY DEFINER views / baseline policy / function grants clear. **Do not invent a second migration if 031 already covers it.** |
| **2** | **Ship RFC-075: auth-on isolation test lane** | Proof tenant A ≠ B | CI evidence, not narrative | 1–2 days | Auth **on** (`CONTRACTGATE_DEV_NO_AUTH` unset/false). Include no-key → 401 sanity so the lane cannot silently lie. |
| **3** | **`docs/security-overview.md`** | Sales attach / questionnaire | Data-room staple | ~0.5 day | Cover: auth model, RLS vs service-role, key scopes (`allowed_contract_ids`), retention, SSRF/CORS, vuln reporting. Link RFCs + tests. |
| **4** | **Auth-on compose smoke** | Demo stack matches prod threat model | Removes false-green diligence flag | ~1 day | Wire or replace disabled isolation step from RFC-073 learnings. |

**P0 exit criteria:** CI proves isolation with auth on; advisors clean on prod; security one-pager exists and is emailable without edits.

---

### P1 — Commercial spine

| # | Work | Customer value | Acquirer value | Size | Notes / acceptance |
|---|---|---|---|---|---|
| **5** | **Per-org monthly event counter** | Fair limits; usage story | Metering = monetization machinery | 2–3 days | Rollup from audit/ingest; org-scoped; define month boundary (UTC calendar month is fine v1). |
| **6** | **Enforce Free / Growth caps on ingest** | Upgrade moment; free-rider control | Tiers are not marketing | 1–2 days after #5 | Align with README: Free 1M, Growth 50M, Enterprise unlimited. Return **429** + clear JSON body. |
| **7** | **Dashboard usage widget** | Self-serve upgrade trigger | Productized billing signal | ~1 day | “Events this period / plan limit” + link to pricing/upgrade. |
| **8** | **Stripe path hardening + ops notes** | Fewer silent “paid still free” failures | Revenue reliability | ~1 day | `stripe_failed_events` visibility; short runbook section. |

**P1 commercial exit criteria:** Test org can hit Free cap → 429 → Checkout upgrade → usage continues under Growth, without founder SQL.

---

### P1 — Pilot / demo reliability

| # | Work | Customer value | Acquirer value | Size | Notes / acceptance |
|---|---|---|---|---|---|
| **9** | **Hero-path fixture pack** | 15‑min demo that never fails | Live product walkthrough | ~1 day | Sample bad events + contract + expected violations + quarantine → replay. Prefer **HTTP** first. |
| **10** | **`make demo` / compose-demo stay green** | Design partners succeed offline | Onboarding quality signal | Ongoing | Pin a stable pilot seed if needed. |
| **11** | **Exportable pilot report** | Success metric for their boss | “Value delivered” artifact | 1–2 days | CSV/JSON: violations + pass rate for a contract over a window (thin v1 OK). |
| **12** | **Ingest path decision** | Cleaner integration story | Less dual-path risk | 1 day decide + implement | Deprecate legacy `/ingest` with sunset header **or** backport idempotency/rate-limit/quarantine_id. Document choice. |

---

### P2 — Handoff / enterprise packaging

| # | Work | Customer value | Acquirer value | Size | Notes / acceptance |
|---|---|---|---|---|---|
| **13** | **`docs/ops/runbook-production.md`** | Enterprise “who runs this?” | Acquisition handoff | ~1 day | Deploy, env vars, health/ready, rollback, Stripe webhook failure, migration apply. |
| **14** | **Architecture one-pager** | Solutions / security questionnaires | Data room | ~0.5 day | Diagram: producer → key/org → contract → validate → audit/quarantine. |
| **15** | **Support SLA markdown** | Closes support objection | Commercial maturity | ~0.5 day | Growth best-effort vs Enterprise response targets. No tooling required. |
| **16** | **Data-room index** `docs/data-room/README.md` | Instant “send materials” | Same | ~0.5 day | Links: security, IP (`LICENSE`/`NOTICE`), STATUS, runbook, demo, pricing. |
| **17** | **Prod migration-drift scheduled CI** | Prevents next Stripe-class outage | Ops maturity | ~1 day + secrets | Compare prod ledger to migration files (read-only DB role). See July 10 worklist item 2. |

---

## Explicitly out of scope (until pulled by a deal)

Do **not** prioritize these for dual-sell ROI right now:

- SSO / SAML (RFC-062)  
- Full open-core / enterprise feature-flag split (RFC-059–061)  
- Terraform provider, Kubernetes Operator  
- Java / Go SDKs  
- Contract marketplace / templates network effects  
- Visual builder nested-object polish (RFC-080) unless a pilot needs it  
- RAG ingestion profile (RFC-077)  
- Reddit bot (RFC-027)  
- Broad new inference formats  
- Kafka/Kinesis shared-core refactor (good debt paydown; schedule after P0/P1)

---

## Suggested 2-week engineering sprint

### Week 1 — Trust

1. Verify/apply **migration 031** + advisor recheck (Alex applies prod).  
2. Implement **RFC-075** auth-on isolation lane + no-key → 401 gate.  
3. Write **`docs/security-overview.md`** (link from `SECURITY.md` / README).  
4. Flip Supabase **leaked password protection** (Auth dashboard toggle — not SQL).

### Week 2 — Commercial + pilot

5. Event metering rollup + ingest cap enforcement.  
6. Usage widget on dashboard.  
7. Hero-path fixture + thin pilot export.  
8. Ops runbook + data-room index (docs; parallelizable).

### Sprint exit criteria

- [ ] Isolation proven in CI with auth on  
- [ ] Free tier can hard-stop on event count with clear 429  
- [ ] Security page exists and is shareable  
- [ ] 15‑minute hero demo runs without manual heroics  

That package supports **“try this for two weeks”** and **“here’s the diligence folder.”**

---

## Process constraints (ContractGate house rules)

When implementing, follow `CLAUDE.md`:

- Branch: `nightly-maintenance-$(date +%Y-%m-%d)-<rfc-slug>` from **`origin/main`** (fetch first; local main has been stale before).  
- `cargo check && cargo test && cargo clippy --all-targets -- -D warnings` before declaring done.  
- Validation engine stays **&lt;15 ms p99**; no panics on request path.  
- Org RLS via `public.get_my_org_ids()` only — no inline `org_memberships` subqueries (PG 42P17 recursion).  
- **Do not apply migrations to prod Supabase from agent sessions** — write files; Alex applies.  
- New migration file ⇒ bump `EXPECTED_MIGRATION_COUNT` + sentinel in `.github/workflows/ci.yml`.  
- User-facing changes ⇒ update or add `docs/*-reference.md`; append `MAINTENANCE_LOG.md`.  
- One issue/PR at a time unless directed otherwise.

---

## Recommended first implementation PR

**RFC-075 auth-on isolation test lane**

- Lowest prod risk (CI/tests only if 031 already merged to main).  
- Unblocks honest claims for both customer pilots and acquirer diligence.  
- Unblocks safe re-enablement of compose isolation assertions.

If 031 is not yet applied to prod, treat **apply 031** as a parallel operator checklist item, not a code invent.

---

## Questions for Claude (reviewer)

Please challenge or improve this plan on:

1. **Ordering** — Should metering (#5–7) jump ahead of RFC-075 if public signup is off and all pilots are self-hosted?  
2. **Quota design** — Count only `passed` events, all ingest attempts, or billable validated events? Idempotent retries?  
3. **Hero path** — HTTP-only vs Kafka-first for ICP (proptech / Findigs-style vs streaming platform)?  
4. **Legacy `/ingest`** — Deprecate vs backport: which is safer given existing SDK/docs traffic?  
5. **Scope creep** — Any P0 item that is actually P2 for a private design-partner-only GTM?  
6. **Missing diligence artifacts** — IP assignment, patent docket status, SOC2 timeline — anything engineering should stub in-repo now?

---

## Claude reviewer response (2026-07-14)

Reviewed against the live repo and prod Supabase. Facts check out (031 exists,
RFC-075 Draft, dual `ingest.rs`/`v1_ingest.rs`, metering unbuilt). Plan structure
endorsed. Deltas below.

### Corrections to state / process

- **Migration 031 is already applied to prod** (`20260711124002`), and advisors
  were re-checked 2026-07-14: **zero ERROR-level items remain.** P0 #1 collapses
  to "already done." No migration 032 is warranted — 031 already covers the
  fixable surface. Every residual advisor item is intentional or a toggle:
  - `get_my_org_ids()` SECURITY DEFINER executable by `authenticated` (WARN) —
    **by design**; org RLS policies call it as the querying user. Revoking it
    breaks tenancy (PG 42P17 class). Do **not** "fix" this lint.
  - `early_access` INSERT `WITH CHECK (true)` (WARN) — public waitlist form,
    intentional. Optional: scope to `anon` role only. Not a dual-sell blocker.
  - 4× `rls_enabled_no_policy` (INFO) — service-role-only tables; 031 documented
    intent via `COMMENT ON TABLE`. INFO lint still fires but is expected.
  - `auth_leaked_password_protection` (WARN) — **Auth dashboard toggle, not SQL.**
    Only genuinely-open item; needs Alex to flip it (Auth → Password settings).
- **Stale house rule:** the doc's "do not apply migrations to prod from agent
  sessions — Alex applies" changed 2026-07-10. Claude now applies prod migrations
  via the Supabase MCP, with the migration-drift workflow as the guard. Correct
  sequence for a *new* migration is still file-committed-first → then apply, to
  avoid the drift class we fought on 2026-07-09. Git operations remain Alex-only.

### Ordering change (if goal is first sale)

Metering (#5–7, ~5 days) is mostly a path-B "tiers are real" story. You don't need
quota enforcement to bill a design partner. **Pull #9 hero-path fixture and #11
pilot export up next to RFC-075; let metering slip behind them.**

### Demote / fold

- **#4 (auth-on compose smoke) is not P0.** ~80% overlap with #2 (RFC-075 auth-on
  lane). Once 075 proves isolation in CI, re-enabling the compose assertion is
  polish → P2. Don't let it gate the trust exit.
- **#8 (Stripe hardening)** is P2 for an invoiced-pilot GTM. Don't build self-serve
  billing reliability before a self-serve customer exists.

### Missing artifact Grok skipped

- **Third-party dependency license inventory** (`cargo-about`). Acquirers scan a
  Rust dep tree for GPL/AGPL contamination — cheap, engineering-ownable, real
  red-flag if absent. Add to the data room next to `LICENSE`/`NOTICE`.

### Answers to the 6 questions

1. **Ordering** — No, metering does not jump ahead of 075. But 031-apply (now done)
   was the true lowest-effort/highest-diligence-ROI item; 075 is next.
2. **Quota design** — Count *billable validated events* (accepted into the
   validation path), dedup idempotent retries via existing `idempotency.rs` key.
   Don't count pre-validation rejects (malformed/401).
3. **Hero path** — HTTP-first, unambiguously. ICP is request/response. Kafka is a
   diligence checkbox and a worse live demo.
4. **Legacy `/ingest`** — Deprecate with `Sunset`/`Deprecation` headers; do **not**
   backport. Backporting doubles the surface the plan is trying to shrink.
5. **Scope creep for private GTM** — #4 and #8 (above) are effectively P2.
6. **Missing diligence artifacts** — IP assignment (founder/contractor → entity;
   #1 non-code check), patent docket one-liner (serial/filing/counsel for the
   "Patent Pending" claim), dependency license inventory (above). Skip a SOC2
   *timeline* you can't commit to — write a "security posture / SOC2-readiness"
   note instead.

### Newly discovered P0 (2026-07-14): quarantine→replay HTTP gap

While building the hero-path fixture (#9), found the dashboard Quarantine tab is
wired to backend routes that don't exist: no `GET /quarantine` list endpoint at
all, and `POST /quarantine/replay` (frontend `{event_ids, version, contract_id}`)
never matched the backend's `POST /contracts/{id}/quarantine/replay`
(`{ids, target_version}`). The `ReplayResponse` shape also differs. Net: the
quarantine/replay feature — a headline for both sells — is unreachable over HTTP,
and the demo's replay step has no supported path. Spec'd as
[RFC-081](../rfcs/081-quarantine-list-and-replay-reconciliation.md) with a
Sonnet-ready worklist at
[`docs/punchlist/2026-07-14-rfc081-quarantine-endpoints.md`](../punchlist/2026-07-14-rfc081-quarantine-endpoints.md).
This blocks #9/#11; do it before the hero-path demo.

### Recommended first implementation (revised)

031-apply is done. Highest-value item completable without cargo/git risk is
**P0 #3 `docs/security-overview.md`** — serves both sells (sales questionnaire
attach + data-room staple), self-verifiable. Claude is drafting it now. RFC-075
is next but needs a branch + `cargo test` (Alex commits/runs).

---

## Related files to read before implementing

- `CLAUDE.md`  
- `docs/STATUS.md`  
- `docs/rfcs/075-auth-on-isolation-test-lane.md` (if present; else STATUS notes)  
- `docs/rfcs/045-plan-gating.md` / `docs/plan-gating-reference.md`  
- `docs/rfcs/058-twelve-month-roadmap.md`  
- `docs/stripe-billing-reference.md`  
- `docs/punchlist/2026-07-10-sonnet-worklist.md`  
- `docs/reviews/sale-readiness-review-2026-05-28.md`  
- `REVIEW-2026-07-09-maintenance-sweep.md`  
- `supabase/migrations/031_security_advisor_fixes.sql`  
- `.github/workflows/ci.yml` (compose-smoke, migrations-check, dashboard jobs)

---

## Changelog

| Date | Note |
|---|---|
| 2026-07-14 | Initial dual-sell worklist from product + sale/launch readiness synthesis. |

---

*End of document — intended for share-out to Claude (or any implementer) without chat context.*
