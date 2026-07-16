# ContractGate — product knowledge for the Slack bot

Curated facts only. Prefer these over guessing. If something is not here, say so
honestly rather than inventing RFCs or pricing.

## One-liner

ContractGate stops bad data events **before** they hit the warehouse: semantic
contracts enforced **at ingest**, with **quarantine + replay** and an exportable
**pilot report**.

## How it differs from dbt tests / Great Expectations / warehouse DQ

| | ContractGate | dbt tests / warehouse DQ |
|---|---|---|
| **When** | At ingest (HTTP, stream, partner API) | After load / in the warehouse |
| **Failure** | Event held in quarantine, not forwarded | Row already landed; fix is reactive |
| **Artifact** | Pilot report (pass/quarantine rates, top violations) | Test failures in CI or warehouse jobs |
| **Partner APIs** | Envelope/batch contracts (e.g. MRI-style `{ data: [...] }`) | Often never touch dbt |

Use both: warehouse tests catch model bugs; ContractGate catches bad producers
and partner payloads **before** they pollute downstream.

## Why Rust (not Python) for the validation engine

- **Hot path:** every event is validated in the gateway. Rust (Axum) is chosen for
  predictable low latency and a panic-free validation path.
- **Compile once, validate many:** contracts are compiled (regexes, field index,
  etc.) and reused; measured validation p99 is well under the 15 ms product target
  (on the order of tens of microseconds in local/prod measurements — always say
  "sub-millisecond / well under 15 ms p99 target" unless citing a specific bench).
- **Python still matters:** there is a **Python SDK** and CLI/tooling ecosystem;
  TypeScript dashboard on Vercel. Rust is the **engine**, not the only language
  in the product.
- **Not** "we hate Python" — it's the right tool for a high-throughput, always-on
  gate in front of Kafka/Kinesis/HTTP.

## Architecture (short)

- **Gateway:** Rust/Axum on Fly.io (US `iad`)
- **Dashboard:** Next.js on Vercel (contracts UI, quarantine, usage, keys, billing)
- **DB:** Supabase Postgres (contracts, versions, `audit_log`, `quarantine_events`,
  hashed API keys, orgs, usage counters)
- **Billing:** Stripe Checkout + webhooks for Growth self-serve
- **Auth:** API keys (server-to-server) + Supabase JWT (dashboard); org isolation
  enforced on the data plane

## Core loop (the product story)

1. **Contract** — versioned YAML (semantic fields, types, enums, optional envelope)
2. **Ingest** — `POST /ingest/...` or `/v1/ingest`, Kafka, or Kinesis
3. **Pass** → audit + forward downstream  
   **Fail** → **quarantine** (held with payload + violations; not silently dropped)
4. **Fix** producer or promote a new contract version
5. **Replay** quarantined events against a target version → backlog drains
6. **Pilot report** — per-contract totals (passed/quarantined), pass rate, top violations
   (the "value delivered" artifact for pilots)

Deploying a new stable version can be blocked while quarantine is pending (safety:
handle the backlog deliberately).

## Envelope contracts

Partner APIs often wrap records: `{ success, data: [...], pagination }`.
Contracts can declare an `envelope` stanza (`records_path`, optional wrapper
checks). Used for MRI/Findigs-style integrations. Envelope traffic is billable
and goes through audit/quarantine like normal HTTP ingest.

## Plans & metering (be accurate)

| Plan | Monthly event limit | Rough price (hosted) |
|------|---------------------|----------------------|
| Free | 1,000,000 | $0 |
| Growth | 50,000,000 | $299/mo (self-serve Stripe) |
| Enterprise | Unlimited | Custom |

- `GET /usage` — current month used/limit/remaining
- Over Free/Growth cap → ingest **429** `plan_limit_exceeded` (HTTP metered paths)
- `dry_run` is not billable
- **Kafka/Kinesis not metered in v1** — prefer Enterprise for production streaming;
  Free/Growth stream usage may under-report on `/usage`
- Self-hosted / no-org is unmetered

UI plan gating also limits features (e.g. Free: limited contracts; Growth+: replay,
Kafka/Kinesis tabs, builder, etc.; Enterprise: SSO, custom SLA, dedicated, etc.).
If unsure of a UI gate detail, say "check Pricing / the dashboard PlanGate" rather
than inventing.

## Integrations & surfaces

- HTTP batch ingest and v1 ingest (idempotency/rate-limit on v1 path)
- Kafka ingress, Kinesis ingress
- Dashboard: playground, quarantine list + replay, pilot report, API keys, billing
- CLI / GitOps, Python SDK, contract inference from samples (JSON/CSV/etc.)
- ODCS-oriented import/export story (contracts as code; runtime enforcement is us)

## Security / tenancy (high level)

- Multi-tenant hosted: org-scoped contracts and data
- API keys scoped; wrong-org looks like 404 (no existence leak)
- PII transforms can run before durable write (mask/hash before quarantine/audit)
- US residency today (Fly iad + Supabase us-east-2)

## What we are not

- Not a warehouse transformation layer (not dbt, not Spark)
- Not "only schema JSON Schema syntax" — semantic contracts + runtime enforcement
- Not a general chatbot — this Slack bot answers ContractGate product questions

## Advisory / getting started

- People can say **"I'm interested"** for a short intake (name/company, email, stack,
  pain, self-serve vs guided)
- Hero demo story: bad events quarantine → fix contract/version → replay clean
- Site/app: datacontractgate.com / app.datacontractgate.com (use current public URLs
  if known; don't invent email addresses)

## Answering style for tough technical questions

- Prefer: short true statement from this knowledge → offer to go deeper
- Example (Rust): "The validation gateway is Rust (Axum) for a compile-once,
  validate-many hot path with sub-ms latency targets. Python is still used for
  the SDK and tooling—the dashboard is TypeScript. Want how quarantine/replay
  fits that path?"
- Do **not** say "not documented" if the fact is in this knowledge file.
