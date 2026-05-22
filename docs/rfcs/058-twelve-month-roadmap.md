# RFC-058 — ContractGate 12-Month Product Roadmap (2026 H2 – 2027 H1)

**Status:** Draft  
**Date:** 2026-05-22  
**Branch:** n/a — planning document  
**Addresses:** REVIEW-2026-05-22-launch-readiness §roadmap

---

## Purpose

A sequenced 12-month plan from the public launch onward. It folds in the
launch blockers from the 2026-05-22 review, the deferred items from
`docs/punchlist/`, and the natural product progression of a contract-
enforcement gateway. Each quarter has a theme, a short list of deliverables,
and an exit criterion. Dates are quarters, not commitments — re-plan each
quarter against real pilot signal.

Guiding principles: (1) ship isolation and abuse-resistance before reach;
(2) let real usage design anything that needs scale signal; (3) keep the
validation engine the fast, correct patent core — never regress it.

---

## Q3 2026 (Jul–Sep) — Launch Hardening & Go-Live

**Theme:** earn the right to be public. Close every tenant-isolation and
abuse hole, then launch.

- **Launch blockers** — RFC-047 (backend org-scoping), RFC-048 (drop
  `x-org-id` trust), RFC-049 (SSRF redirect), RFC-050 (CORS allowlist).
- **Hardening bundle** — RFC-051 (key-cache), RFC-052 (JWKS refresh),
  RFC-053 (`/ready` probe), RFC-054 (lock-poison recovery), RFC-055 (CI
  toolchain).
- **Issuance & docs** — RFC-056 (server-side key issuance), RFC-057
  (documentation completeness).
- **External pen-test** of the multi-tenant surface before the announcement.
- **Public launch.**

**Exit criterion:** an independent security review finds no cross-tenant
read/write path; CI is green for the right reasons; all shipped features have
reference docs.

---

## Q4 2026 (Oct–Dec) — Managed Plane: Metering & Billing

**Theme:** make the SaaS economically real now that tenants are isolated.

- **Usage metering service** — count API calls, contracts stored, connector
  events per org; hourly rollup table. (`docs/punchlist/08`.)
- **Plan-tier enforcement** — wire RFC-045 plan gating to live quotas
  (events/month per Free/Growth/Enterprise from the README pricing table).
- **Billing integration** — Stripe; self-serve upgrade/downgrade.
- **Self-serve org signup + onboarding** — email verification, first-contract
  wizard, sample data.
- **Quota & rate-limit dashboards** — per-org usage visible to the customer
  and to operators.
- **Alerting v1** — the deferred RFC-009 stack (Slack, email digest, generic
  webhook) now that pilots produce continuous traffic to design thresholds
  against.

**Exit criterion:** a customer can sign up, hit a plan limit, upgrade, and be
billed — with no manual operator step.

---

## Q1 2027 (Jan–Mar) — Reach: SDKs & Ecosystem

**Theme:** lower integration cost; meet teams in their stack.

- **SDK rollout** — Java and Go clients (TypeScript and Python already ship);
  freeze the wire shape first.
- **Terraform provider** — manage contracts, policies, and connectors as IaC.
- **Pre-commit hook framework + GitLab/Bitbucket CI templates** — extend the
  GitHub-Actions story to the rest of the market.
- **Templates / marketplace v1** — a browsable registry of starter contracts
  beyond the three bundled ones; ratings and submission once there is a
  network to support them.
- **Impact estimator** — `/contracts/{id}/impact` and the materialized
  producer-count view, now that audit volume makes the numbers meaningful.

**Exit criterion:** a new team can adopt ContractGate in their own language
and CI without hand-written HTTP calls.

---

## Q2 2027 (Apr–Jun) — Enterprise & Scale

**Theme:** the features that close larger deals and prove durability.

- **SSO / SAML** — the README already lists it as an Enterprise capability.
- **RBAC role editor** — beyond the current collaborator roles; org-level
  admin granularity.
- **Kubernetes Operator + CRDs** (`ContractGateInstance`, `ContractPolicy`)
  for on-prem / air-gapped deployments.
- **Anomaly detection & real-time health dashboard** — built on the metering
  data now flowing.
- **Performance program** — publish a reproducible, audited benchmark for the
  validation engine; defend the p99 budget under multi-tenant load with a
  continuous load test in CI.
- **Audit-log retention tiers** — wire the 7/90/custom-day retention from the
  pricing table to real lifecycle jobs.

**Exit criterion:** ContractGate can be sold to a security-conscious
enterprise — SSO, RBAC, self-hostable, with a published performance number.

---

## Continuous (every quarter)

- Nightly maintenance runs — tech-debt, dependency bumps, `cargo audit`.
- Keep the validation engine within its p99 budget; no feature regresses it.
- One RFC per non-trivial change; `docs/STATUS.md` kept current.
- Re-plan the next two quarters against pilot and launch signal.

---

## Explicitly deferred beyond this window

- Database-per-tenant isolation — row-level `org_id` scoping is sufficient
  until a customer's compliance posture forces the change.
- LLM-backed migration / diff prose — rule-based output ships the demos; the
  LLM layer is a feature add, not a foundation.
- A full rule-engine for alerting — start with fixed conditions; generalise
  only when customers ask.

---

## Open questions for the planning conversation

1. Metering granularity — per-event rows, hourly rollup, or both?
2. Billing — Stripe-managed plans only, or also usage-based overages at launch?
3. Signup — email-only first, or SSO from day one for design-partner orgs?
4. Operator vs Terraform — do enterprise pilots want both, and in which order?
5. Does the patent-pending status constrain how the engine benchmark is
   published?
