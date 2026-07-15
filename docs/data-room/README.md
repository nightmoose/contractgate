# ContractGate — Data Room Index

**Purpose:** one place to send a prospective acquirer or an enterprise security
reviewer. Every link below is a file in this repo. Last updated 2026-07-15.

> Product one-liner: stop bad events **before** they hit the warehouse — semantic
> contracts enforced at ingest, with quarantine/replay and sub-ms validation.
> Patent pending.

## Product & architecture
- [Architecture overview (one-pager + diagram)](../architecture-overview.md)
- [README](../../README.md) — product surface + positioning
- [RFC status index](../STATUS.md) — shipped vs draft across all RFCs

## Security
- [Security overview](../security-overview.md) — auth, tenant isolation, PII,
  SSRF/CORS, Supabase posture, retention, vuln reporting
- [SECURITY.md](../../SECURITY.md) — vulnerability disclosure policy + SDLC
  controls (secret scanning, CodeQL, cargo-audit/deny, signed commits)
- [Auth reference](../auth-reference.md) — JWT + API-key model, org scoping, CORS

## Commercial
- [Plan gating reference](../plan-gating-reference.md) — tiers (free/growth/enterprise)
- [Usage / metering reference](../usage-reference.md) — per-org monthly usage vs limit
  (**read API + widget shipped; ingest 429 enforcement still Phase 2**)
- [Pilot report reference](../pilot-report-reference.md) — the "value delivered" export

## IP & licensing
- [LICENSE](../../LICENSE) — MIT
- [NOTICE](../../NOTICE) — copyright
- [IP assignment checklist](./ip-assignment-checklist.md) — founder/contractor IP,
  patent docket, trademarks (**owner action — see open items**)
- [Third-party Rust dependency licenses](./dependency-licenses.md) — cargo-about
  inventory (regenerate: see file header)

## Ops & reliability
- [Production runbook](../ops/runbook-production.md) — deploy, health, secrets, incidents
- [Pilot / hero demo](../../demo/hero/README.md) — 15-min quarantine→replay walkthrough
- [Incident postmortem — 2026-07-14 JWT CryptoProvider](../reviews/incident-2026-07-14-jwt-crypto-provider.md) — P0 root cause + fix + prevention
- Prior readiness reviews: [sale-readiness 2026-05-28](../reviews/sale-readiness-review-2026-05-28.md)

## Feature references (selected)
- [Quarantine + replay](../quarantine-replay-reference.md)
- [Deploy contract](../deploy-contract-reference.md)
- [Scorecard](../scorecard-reference.md)

---

## Open diligence items (honest gaps)

These are known and tracked, not hidden:

1. **IP assignment** — confirm founder + any contractor IP is assigned to the
   entity (see checklist). #1 acquirer check; not a code artifact.
2. **Patent docket** — "Patent Pending" is asserted; capture serial no. / filing
   date / counsel in the checklist.
3. **SOC 2** — not started; treat as "posture + readiness," not a committed date.
4. **RFC-083 Phase 2** — ingest **429** at plan cap + cached counter (hot path;
   needs live p99 smoke). Usage **read** API + dashboard widget are shipped.
5. **cargo-deny `[licenses]` CI gate** — inventory exists (`dependency-licenses.md` +
   `about.toml`); optional next step is wiring license allowlist into CI deny.

Owner/legal items (#1–3) are outside the engineering backlog. Phase 2 metering
is tracked in RFC-083.
