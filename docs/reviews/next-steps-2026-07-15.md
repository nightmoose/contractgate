# Next steps — prioritized backlog (2026-07-15)

**Author:** Claude (session synthesis). **For:** Grok to turn into RFCs/worklists;
Sonnet to implement.
**Lens:** dual-sell — advance (A) first customer sale and (B) acquisition/diligence
readiness. Prefer intersection work.

## Just shipped (this session)

RFC-081 quarantine list + replay (dashboard tab now works), RFC-082 pilot report
(JSON/CSV), RFC-083 event metering (usage API + ingest 429 enforcement + widget;
migration 032 applied to prod), RFC-075 auth-on isolation CI lane, hero demo
(`demo/hero/` + `scripts/hero_demo.sh`), data-room package
(`docs/data-room/`, `docs/architecture-overview.md`), dependency-license
inventory (`docs/data-room/dependency-licenses.md`), rsa advisory handled in
`deny.toml`. Metering p99: validated locally; effect is below Docker-for-Mac
noise — justified on first principles (one indexed read).

---

## P0 — correctness / blocks the first-sale motion

### 1. Envelope contracts skip audit **and** quarantine (only meter)
The envelope short-circuit (`src/ingest.rs` ~lines 340–372; `validate_envelope_batch`
is pure) returns after validation + `record_batch_usage` — it writes **no
`audit_log` and no `quarantine_events`**. Impact on the **MRI/Findigs proptech ICP**
(which uses envelope contracts):
- **Pilot report (RFC-082) is empty** for them — the #1 "value delivered" artifact.
- **Quarantine tab / replay (RFC-081) is empty** — blocked events aren't stored,
  can't be inspected or replayed.
- Only the usage counter increments.

This quietly guts the two headline artifacts for the exact ICP we target.
**Decide + do:** route the envelope path through the same audit + quarantine
writes as the per-record path (preferred), or explicitly scope envelope as
"validation-only, no audit" and stop selling the report/replay story on it.
Size: medium (thread envelope results into the audit/quarantine insert path).

### 2. `/contracts/deploy` ignores `x-org-id` in dev-no-auth → 500
`deploy_contract_handler` uses `org_id_from_req`, which (post-RFC-048) does not
trust `x-org-id`, so in dev-no-auth deploy inserts a null `org_id` and 500s
(`null value in column "org_id"`). Breaks the `ORG_ID` path of `hero_demo.sh` and
`perf_metering_smoke.sh`. **Fix:** give deploy the same **dev-gated** `x-org-id`
fallback ingest already has (only when `CONTRACTGATE_DEV_NO_AUTH=1`, so RFC-048's
"never trust x-org-id in prod" holds — keep the `x_org_id_header_not_trusted`
test green). Dev-only ergonomics, prod unaffected. Size: small.

---

## P1 — billing integrity + ops/diligence readiness

### 3. RFC-083 Phase 2 hardening (the follow-ups it deferred)
- **Kafka/Kinesis metering decision.** Streaming ingress neither checks nor
  increments the counter, so a Growth org can bypass the cap and `/usage`
  under-reports. Either **gate streaming to Enterprise** (unlimited → moot) and
  document, or increment the counter on stream paths (429 on a consumer loop is
  impractical). Owner call, then wire it.
- **Reconcile job.** The counter can drift below `audit_log` truth (fire-and-forget
  increment lost on crash) and never re-syncs. Add a periodic reconcile
  (`SET events = GREATEST(events, <audit count>)`, up-only so it never erases
  envelope/stream counts). Cheap insurance; also closes the bootstrap over-count
  edge. Size: small.

### 4. Ops production runbook (`docs/ops/runbook-production.md`, dual-sell #13)
Deploy, env vars, health/ready, rollback, Stripe-webhook-failure recovery,
migration-apply procedure, the 2026-07-14 JWT incident pattern. Pure docs;
data-room staple + "who runs this?" acquirer question. Grok can draft from the
incident doc + `fly.toml` + CLAUDE.md. Size: ~1 day.

### 5. Prod migration-drift scheduled CI (dual-sell #17)
Daily (cron + manual) compare `supabase_migrations.schema_migrations` (prod, via a
read-only `PROD_DATABASE_URL` secret) to `supabase/migrations/*.sql`. Not in the PR
path (fork-secret risk). Prevents the next Stripe-class silent drift. Size: ~1 day
+ a secret/role.

---

## P2 — polish / lower urgency

6. **Ingest path decision (dual-sell #12).** Deprecate legacy `/ingest` with
   `Sunset`/`Deprecation` headers, or backport idempotency/rate-limit/quarantine
   to it. Kills the dual-path (`ingest.rs` vs `v1_ingest.rs`) diligence + test tax.
7. **RFC-075 Phase 2.** Wire the remaining Class-2 isolation tests
   (`soft_delete_hides_from_list`, v1 round-trips, expired-invite, metrics,
   cli_push_pull) into the auth-on lane.
8. **Support SLA doc (dual-sell #15).** Growth best-effort vs Enterprise response
   targets. Trivial; closes a sales objection.
9. **Native-Linux p99 baseline (optional).** One-shot perf run on a Fly machine
   for a real prod-representative metering p99 number for the data room. Low
   priority — Docker-for-Mac can't resolve it.

---

## Suggested order for Grok
P0 #1 (envelope) and #2 (deploy) first — #1 is the biggest first-sale risk, #2 is
a quick win that also unblocks local demos. Then P1 #3–5 (billing integrity + the
two ops/diligence docs). P2 as capacity allows.

## Status (Grok, same day)

| Item | Status |
|------|--------|
| P0 #1 envelope audit + quarantine | **In PR** `nightly-maintenance-2026-07-15-envelope-audit-deploy` — route through same audit/quarantine/forward path as per-record |
| P0 #2 deploy `x-org-id` | **In same PR** — dev-gated fallback matching create_contract |
| P1–P2 | Not started |

*Legal/founder items (patent docket, SOC 2, IP assignment, DPA) are tracked in
`docs/data-room/ip-assignment-checklist.md` — not engineering work.*
