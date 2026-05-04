# ContractGate

**Semantic Contract Enforcement at Ingestion** — *Patent Pending*

Stop bad data **before** it reaches your warehouse, lakehouse, or ML pipeline.

ContractGate is a high-performance validation gateway that enforces rich semantic data contracts in real time. Built in Rust for extreme speed (<10 µs p99 latency, 86k+ events/sec/core), it goes far beyond JSON Schema or basic type checks — ontology, glossary, patterns, enums, computed metrics, and automatic inference.

[![Security](https://img.shields.io/badge/security-hardened-brightgreen)](https://github.com/nightmoose/contractgate/security)
[![Dependabot](https://img.shields.io/badge/Dependabot-enabled-brightgreen)](https://github.com/nightmoose/contractgate/security/dependabot)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

**Live demo** → [app.datacontractgate.com/stream-demo](https://app.datacontractgate.com/stream-demo)

---

## Try it in 10 minutes (Self-Hosted Free)

No Supabase account. No API keys. No sign-up.

```bash
git clone https://github.com/nightmoose/contractgate.git
cd contractgate
make demo
```

Opens at **http://localhost:3000** — dashboard pre-loaded with three starter
contracts and a live event stream. First build ~3 min, subsequent runs instant
(Docker layer cache).

```bash
make demo-reset   # wipe + reseed (clean slate)
make demo-down    # stop and remove volumes
make demo-logs    # follow all service logs
```

> **Self-Hosted Free** runs the real gateway binary — not a sandbox.
> Unlimited local events, full YAML editor, playground, audit log.
> [Feature comparison below ↓](#self-hosted-free-vs-cloud)

---

## Key Features

- **Semantic YAML Contracts** — entities, types, patterns, enums, required fields, glossary, and computed metrics
- **Real-time Validation** — at ingestion (Kafka, HTTP, batch) with <10 µs p99 latency
- **Visual Contract Builder** — intuitive UI + live YAML preview *(Cloud)*
- **AI-Powered Inference** — paste sample JSON → instantly generates a full contract *(Cloud)*
- **Quarantine + Replay** — automatically hold and replay violating events *(Cloud)*
- **Versioned Contracts** — draft → stable → deprecated with compliance mode *(Cloud)*
- **High Performance** — Rust + Axum core, 86k+ events/sec/core
- **Polyglot** — Rust engine, Python SDK, CLI, Kafka Connect, Next.js dashboard

---

## Self-Hosted Free vs Cloud

| Capability | Self-Hosted Free | Cloud (Free / Growth / Enterprise) |
|---|---|---|
| Validation engine | ✅ Unlimited (local) | ✅ 1M / 50M / Unlimited events/month |
| Starter contracts | ✅ 3 | ✅ 3 / Unlimited / Unlimited |
| Playground | ✅ | ✅ |
| Audit log | ✅ Local Postgres | ✅ 7 / 90 / Custom days retention |
| Live event stream | ✅ | ✅ |
| Auth & API keys | ❌ | ✅ |
| Multi-tenancy | ❌ Single org | ✅ |
| Visual contract builder | ❌ | ✅ Growth+ |
| AI inference (JSON → YAML) | ❌ | ✅ Growth+ |
| Quarantine + replay | ❌ | ✅ Growth+ |
| PII transform rules | ❌ | ✅ Growth+ |
| Semantic versioning UI | ❌ | ✅ Growth+ |
| GitHub sync | ❌ | ✅ Growth+ |
| Team invites & roles | ❌ | ✅ Growth+ |
| SSO / SAML | ❌ | ✅ Enterprise |
| Managed hosting + SLA | ❌ Self-hosted | ✅ |

[→ Full pricing](https://app.datacontractgate.com/pricing)
[→ Talk to sales](mailto:sales@contractgate.io)

---

## Other Quickstarts

**Docker (gateway + Postgres only):**
```bash
docker compose up
```

**Python SDK:**
```bash
pip install contractgate
```
```python
from contractgate import Client
client = Client(api_url="http://localhost:8080", api_key="cg_demo_key")
client.validate(contract_id="...", event={"user_id": "u1", "event_type": "click"})
```

**CLI:**
```bash
cargo install --git https://github.com/nightmoose/contractgate contractgate
contractgate validate --contract examples/nested-order.yaml events.json
```

---

## Architecture

| Layer | Tech |
|---|---|
| Validation engine | Rust + Axum (<10 µs p99) |
| Dashboard | Next.js 15 + TypeScript + Tailwind |
| Storage | Supabase (Postgres + Auth) |
| Connectors | Kafka Connect, HTTP, batch, Python SDK, CLI |
| Observability | Prometheus + Grafana |

Full RFCs and design docs live in [`docs/rfcs/`](docs/rfcs/).

---

## Comparison

| Feature | **ContractGate** | Great Expectations | Soda | Monte Carlo | dbt |
|---|---|---|---|---|---|
| Validation timing | **At ingestion** | Post-hoc / batch | Mix (mostly batch) | Observability | During dbt runs |
| Performance | **<10 µs p99, 86k+ ev/s** (Rust) | Python-based | Varies | Not a validator | Not real-time |
| Semantic depth | **Ontology + glossary + metrics** | Basic expectations | Checks + rules | No contracts | Basic schema |
| Visual builder | **Yes + live YAML** | Partial | Limited | No | YAML only |
| Inference from sample data | **One-click JSON → YAML** | Profiler (basic) | Limited | No | No |
| Quarantine + replay | **Built-in** | No | No | No | No |
| Versioning | **Draft → stable → deprecated** | Manual | Basic | No | Basic |
| Self-hosted OSS | **Yes (`make demo`)** | Yes | Open core | Commercial | Yes |
| License | **MIT** | Apache 2.0 | Open core | Commercial | Apache 2.0 |

---

## Contributing

PRs welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md) and the active punchlist in [`docs/`](docs/).

Active RFCs: [`docs/rfcs/`](docs/rfcs/)

---

## License

MIT — see [`LICENSE`](LICENSE). Free to use, fork, and build on top (commercial use welcome).

The semantic contract enforcement algorithm is the subject of a pending patent.
The patent covers the *invention* — your right to use, modify, and distribute the
*code* under the MIT license is unaffected. See [`NOTICE`](NOTICE) for details.

---

Built with ❤️ in Florida by [NightMoose](https://nightmoose.com)
