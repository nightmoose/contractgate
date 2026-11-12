# Support SLA (hosted DataContractGate)

**Audience:** sales, CS, diligence  
**Status:** Pre-scale — targets for Free, Pro and Enterprise  
**Last updated:** 2026-11-11  

Legal contracts (DPA, MSA) supersede this page when signed. These are **product
targets** for the hosted multi-tenant service, not a 24/7 NOC commitment at
current scale.

**Targets, not guarantees.** The uptime and response figures below for Free and
Pro are goals we work to meet. They are not contractual guarantees, and Free
and Pro carry **no service credits**. Service credits apply only where a signed
Enterprise MSA provides them.

Pro is the plan formerly called **Growth** (the plan value in code is still
`growth`).

---

## Tiers

| | Free | Pro | Enterprise |
|---|---|---|---|
| **Channel** | Docs + community / email best-effort | Email (ticket) | Named channel + email; optional shared Slack |
| **Initial response** | Best-effort | **1 business day** | **4 business hours** (US Eastern, Mon–Fri) |
| **Severity-1 (prod down)** | Best-effort | Same business day | **1 hour** during business hours; after-hours best-effort until on-call is staffed |
| **Severity-2 (degraded)** | Best-effort | 1 business day | 4 business hours |
| **Severity-3 (how-to)** | Docs first | 2 business days | 1 business day |
| **Uptime target** | Best-effort | 99.5% monthly (hosted API), target only, no credits | Custom (MSA); default 99.9% with credits |
| **Dedicated deployment** | — | — | Available |
| **SSO / SAML** | — | — | Available |
| **Custom DPA / BAA path** | Standard DPA | Standard DPA | Negotiated |

**Business hours:** Monday–Friday, 09:00–18:00 US Eastern, excluding US federal
holidays, unless the MSA says otherwise.

---

## Severity guide

| Severity | Examples |
|---|---|
| **1 — Outage** | Hosted API `/health` or `/ready` fail for all tenants; auth total failure; data loss risk |
| **2 — Degraded** | Elevated 5xx, single-region impact, webhook lag, one feature broken with workaround |
| **3 — Inquiry** | Config, contract design, integration questions, feature requests |

Report Severity-1 via the channel in your order form / MSA, and
`status` / status page when published. Include org id, request ids, and
approximate start time.

---

## Fair use and capacity

Hosted Free and Pro run on **shared multi-tenant infrastructure**. "Unlimited"
contracts and versions on Pro mean there is no fixed cap, not unlimited use of
shared capacity: they are subject to fair use, alongside the plan's monthly
event limit and the API rate limits as documented (see
[Plans](aischemagate-plans-reference.md)) or communicated to you.

- NightMoose may **rate-limit, throttle, queue, or temporarily restrict** an org
  whose usage degrades service for other customers or exceeds what is
  reasonable for its plan. Signals include request rate, bursts, payload size,
  contract or version count, and storage.
- We will try to **contact the account first**, except where immediate action
  is needed to protect the service or other customers.
- Sustained high-volume or high-concurrency workloads may be asked to move to
  **Enterprise or a dedicated deployment**.
- Time an org spends throttled or rate-limited because of fair-use enforcement,
  or because it exceeded its plan limits, is **excluded from uptime
  measurement**.

---

## What we measure

- **API availability:** successful responses from Fly-hosted `contractgate-api`
  (excludes customer-side network, Supabase Auth outages we do not control,
  scheduled maintenance announced ≥24h ahead, and requests throttled or
  rate-limited under fair use or plan limits).
- **Support clock** starts when a ticket is received in the official channel with
  enough detail to triage (org, environment, repro).

---

## Self-hosted

Self-hosted deployments are **outside** this SLA unless a separate support
agreement is signed. Community / docs support only by default.

---

## Related

- [Production runbook](ops/runbook-production.md) — how we operate the service  
- [Plan gating](plan-gating-reference.md) — feature tiers  
- [Data room](data-room/README.md) — diligence package  
