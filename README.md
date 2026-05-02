# ContractGate

**Semantic Contract Enforcement at Ingestion**  
*Patent Pending*

Stop bad data **before** it reaches your warehouse, lakehouse, or ML pipeline.

ContractGate is a high-performance validation gateway that enforces rich semantic data contracts in real time. Built in Rust for extreme speed (<10 µs p99 latency, 86k+ events/sec/core), it goes far beyond JSON Schema or basic type checks with ontology, glossary, patterns, enums, computed metrics, and automatic inference.

![Stream Demo Stats](/screenshots/stream-demo-stats.png)
![Records Tab](/screenshots/stream-demo-records.png)
![Visual Contract Builder](/screenshots/visual-builder.png)

[![Security](https://img.shields.io/badge/security-hardened-brightgreen)](https://github.com/nightmoose/contractgate/security)
[![Dependabot](https://img.shields.io/badge/Dependabot-enabled-brightgreen)](https://github.com/nightmoose/contractgate/security/dependabot)

## ✨ Key Features

- **Semantic YAML Contracts** — entities, types, patterns, enums, required fields, glossary, and computed metrics
- **Real-time Validation** — at ingestion (Kafka, HTTP, batch) with zero database writes in demo mode
- **Visual Contract Builder** — intuitive UI + live YAML preview
- **AI-Powered Inference** — paste sample JSON → instantly generates a rich contract with types, patterns, enums, and required flags
- **Quarantine + Replay** — automatically hold and replay violating events
- **Versioned Contracts** — draft → stable → deprecated with compliance mode
- **High Performance** — Rust + Axum core, side-by-side baseline comparison in every demo
- **Polyglot** — Rust engine, Python SDK, CLI, Kafka Connect, Next.js dashboard
- **Supabase Backend** — auth, contracts, audit logs, quarantine out of the box

**Live Demo** → [app.datacontractgate.com/stream-demo](https://app.datacontractgate.com/stream-demo)  
**Marketing Site** → [datacontractgate.com](https://www.datacontractgate.com)

## Quickstart

### Docker (recommended)
docker compose up
Python SDK

## pip install contractgate  # coming to PyPI this week
from contractgate import validate

## CLI
cargo install --git https://github.com/nightmoose/contractgate contractgate
contractgate validate --contract examples/nested-order.yaml events.json

## Try the Live Stream Demo

Visit app.datacontractgate.com/stream-demo
Choose “nested — e-commerce order”
Hit Start and watch 86k events/sec with real semantic violations in real time

Screenshots
(Upload these to a /screenshots/ folder in the repo)

stream-demo-stats.png — 86k ev/s, 10 µs p99, 0% overhead
stream-demo-records.png — PASS/FAIL events with full JSON + highlighted violations
visual-builder.png — form-based contract editor with live YAML
inference-generator.png — JSON → rich YAML contract in one click

## Architecture

- Core: Rust + Axum (validation engine)
- Dashboard: Next.js 15 + TypeScript + Tailwind
- Storage: Supabase (Postgres + Auth + Storage)
- Connectors: Kafka, HTTP, batch, Python SDK, CLI
- Inference: Schema + pattern detection engine

Full RFCs and punchlists live in /docs/.

## Comparison
See how ContractGate stacks up 
# ContractGate vs. the Competition

| Feature                          | **ContractGate**                          | Great Expectations                  | Soda                              | Monte Carlo                      | dbt (Contracts/Tests)          |
|----------------------------------|-------------------------------------------|-------------------------------------|-----------------------------------|----------------------------------|--------------------------------|
| **Validation Timing**            | **At ingestion** (real-time / streaming) | Post-hoc / batch                    | Mix (mostly batch)                | Observability / anomalies        | During dbt runs (batch)        |
| **Performance**                  | **<10 µs p99, 86k+ ev/s** (Rust)         | Python-based, slower                | Varies                            | Not a validation engine          | Not real-time                  |
| **Semantic Depth**               | **Ontology + glossary + computed metrics** | Basic expectations                  | Checks + rules                    | No contracts                     | Basic schema + tests           |
| **Visual Contract Builder**      | **Yes + live YAML preview**               | Partial (profiler)                  | Limited UI                        | No                               | YAML only                      |
| **Inference from Sample Data**   | **One-click JSON → full contract**        | Profiler (basic)                    | Limited                           | No                               | No                             |
| **Quarantine + Replay**          | **Built-in**                              | No                                  | No                                | No                               | No                             |
| **Versioning & Compliance**      | **Draft → stable → deprecated**           | Manual                              | Basic                             | No                               | Basic                          |
| **Zero DB Writes Demo Mode**     | **Yes**                                   | No                                  | No                                | No                               | No                             |
| **Side-by-Side Perf Comparison** | **Built into every demo**                 | No                                  | No                                | No                               | No                             |
| **License / Openness**           | MIT + patent-pending core                 | Apache 2.0                          | Open core                         | Commercial                       | Open source                    |

**ContractGate is the only tool that validates semantically rich contracts *at ingest time* with sub-millisecond latency and delightful builder + inference UX.**

## Hosted vs Self-Hosted

**ContractGate is open-core (MIT licensed).**

- **Self-host** (Docker / Kubernetes) — great for local dev, experimentation, or air-gapped environments.
- **Use our hosted SaaS** (recommended for production) — managed scaling, enterprise features, SLA, and support.

**Free tier**: 1M events/month forever  
**Enterprise**: Unlimited + VPC/on-prem + SSO + custom SLAs

[Try the hosted demo →](https://app.datacontractgate.com/stream-demo)  
[Talk to sales for production](mailto:datacontractgate@nightmoose.com)

## Private Beta
Free tier: 1M events/month
[Get early access](https://www.datacontractgate.com/) or email alex.suarez@nightmoose.com

## Roadmap

- Full Kafka Connect source/sink
- On-prem / VPC deployment options
- SOC2 + enterprise SSO
- More connectors (Fivetran, Airbyte, etc.)

## Contributing
We love pull requests! See CONTRIBUTING.md and the active punchlist in /docs/punchlist.

## License
MIT — feel free to use, fork, and build on top (commercial use welcome).

Built with ❤️ in Florida by NightMoose
