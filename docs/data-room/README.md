# ContractGate — Data Room Index

**Purpose:** one place to send a prospective customer security reviewer (or later, an acquirer). Every link below is a file in this repo.  
**Last updated:** 2026-07-15

> Product one-liner: stop bad events **before** they hit the warehouse — semantic
> contracts enforced at ingest, with quarantine/replay and sub-ms validation.
> Patent pending.

---

## Product & architecture

- [Architecture overview (one-pager + diagram)](../architecture-overview.md)
- [README](../../README.md) — product surface + positioning
- [RFC status index](../STATUS.md) — shipped vs draft across all RFCs

## Security

- [Security overview](../security-overview.md) — auth, tenant isolation, PII, SSRF/CORS, retention
- [SECURITY.md](../../SECURITY.md) — vulnerability disclosure + SDLC controls
- [Auth reference](../auth-reference.md) — JWT + API-key model, org scoping, CORS
- [Incident postmortem — 2026-07-14 JWT CryptoProvider](../reviews/incident-2026-07-14-jwt-crypto-provider.md)

## Commercial

- [Plan gating reference](../plan-gating-reference.md) — free / growth / enterprise
- [Usage / metering reference](../usage-reference.md) — monthly usage vs limit (**enforcement Phase 2 still open**)
- [Pilot report reference](../pilot-report-reference.md) — exportable “value delivered” report
- [Hero demo walkthrough](../../demo/hero/README.md) — 15-min quarantine → replay path

## IP & licensing

- [LICENSE](../../LICENSE) — MIT
- [NOTICE](../../NOTICE) — copyright / patent notice
- [IP assignment checklist](./ip-assignment-checklist.md) — founder/contractor IP (**owner action**)
- [Third-party Rust dependency licenses](./dependency-licenses.md) — cargo-about inventory

## Ops & reliability

- [Production runbook](../ops/runbook-production.md) — deploy, health, secrets, incidents
- Prior reviews: [sale-readiness 2026-05-28](../reviews/sale-readiness-review-2026-05-28.md)

## Feature references (selected)

- [Quarantine + replay](../quarantine-replay-reference.md)
- [Deploy contract](../deploy-contract-reference.md)
- [Scorecard](../scorecard-reference.md)

---

## Open diligence items (honest gaps)

1. **IP assignment** — founder + contractor assignments (checklist). Not a code artifact.
2. **Patent docket** — capture serial / filing date / counsel on the checklist.
3. **SOC 2** — not started; posture only, no committed date.
4. **RFC-083 Phase 2** — ingest **429** at plan cap + cached counter (hot path; needs p99 smoke). Usage **read** API + dashboard widget are shipped.
5. **cargo-deny `[licenses]` allowlist** — inventory generated; CI license *deny* gate still optional (see dependency-licenses.md).

Owner/legal items (#1–3) are outside the engineering backlog. Phase 2 metering is tracked in RFC-083.
