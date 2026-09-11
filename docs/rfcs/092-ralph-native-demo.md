# RFC-092: Ralph-native demo stack (Kafi / Driftless / ContractGate)

| Field      | Value |
|------------|--------|
| Status     | **Accepted — implementing** |
| Author     | ContractGate team |
| Created    | 2026-09-11 |
| Branch     | `nightly-maintenance-2026-09-11-rfc092-ralph-demo` |
| Target     | Working compose demo for design-partner walkthrough (week of 2026-09-15) |
| Depends on | Existing Kafka demo Redpanda compose; Python SDK validator; no engine change required |

---

## Problem

A design partner (Ralph M. Debusmann — author of **Kafi Streams**, **kash.py**, and the **driftless** PoC) offered to try ContractGate. His public work already forms a coherent pipeline:

```
CDC-style producers → Kafka → Kafi Streams joins → LanceDB → MCP (agent memory)
```

ContractGate’s value in that story is the missing middle: **semantic enforcement at ingest** so bad events never poison joins or agent memory. We do not yet have a runnable demo that speaks *his* stack (his topic shapes, his libs, his “zero-drift” punchline). A generic speed demo or hosted signup link is weaker than a fork he can run that wires **his** babies together with the gate.

---

## Goal

Ship `demo/ralph/` — a self-contained, one-command (or short README) stack that:

1. Produces Debezium-style shoe CDC (orders / customers / products) — from his driftless datagen.
2. Validates **after-images** with ContractGate contracts (pass → clean topics, fail → quarantine).
3. Runs his Kafi Streams join → LanceDB → MCP path **only on clean topics**.
4. Makes the punchline obvious: inject a bad event → quarantine → MCP/agent memory stays clean.
5. Optionally uses kafi/kash-style inspection (`cat` / topic list) so the demo feels native to him.

**Success for next week:** Ralph can clone/run, see one bad event quarantined, and query MCP against pristine joined data — without us asking him to redesign his PoC.

---

## Non-goals

- No ContractGate engine / API / SaaS changes required for v1 of this demo.
- Not a replacement for `make demo` or the Kafka speed demo (`demo/` cargo binary).
- Not a reseller / partnership legal framework.
- Not Confluent Cloud kafka-ingress (RFC-025) — this is **local Redpanda**.
- Not vendoring all of Kafi — depend on PyPI `kafi` + a thin adapted driftless layer.
- No derogatory comparisons to Schema Registry / other tools in demo copy.

---

## Architecture

```
┌─────────────────────┐
│  producers.py       │  (adapted from xdgrulez/driftless)
│  + inject_bad.py    │
└─────────┬───────────┘
          │  Debezium envelopes
          ▼
   orders.raw / customers.raw / products.raw     (Redpanda)
          │
          ▼
┌─────────────────────┐
│  gate_bridge.py     │  unwrap after → ContractGate validate
│  (Python SDK local  │  re-wrap → produce
│   and/or HTTP gw)   │
└─────────┬───────────┘
          │
     ┌────┴────┐
     ▼         ▼
  orders /   orders.quarantine
  customers  customers.quarantine
  products   products.quarantine
     │
     ▼
┌─────────────────────┐
│  app.py (Driftless) │  Kafi Streams joins → embed → LanceDB
│  + MCP server       │  consumes CLEAN topics only
└─────────┬───────────┘
          ▼
   client.py / MCP query   → agent memory without drift
```

### Topic map

| Role | Topics |
|------|--------|
| Raw (ungated) | `orders.raw`, `customers.raw`, `products.raw` |
| Clean (Driftless reads) | `orders`, `customers`, `products` |
| Quarantine | `orders.quarantine`, `customers.quarantine`, `products.quarantine` |

Clean topic names stay **identical to upstream driftless** so his Streams code needs only bootstrap/config changes (or a one-line topic rename in our vendored copy).

### What we validate

Contracts apply to the Debezium **`after`** payload on create/update (`op` in `c`/`u`). Deletes (`op=d`) pass through to clean by default (no `after` to validate) so Kafi’s distinct/join semantics still see removals — documented in the demo README. Optional later: validate `before` on deletes.

### Gate modes

| Mode | Env | Use |
|------|-----|-----|
| **local** (default) | `CG_MODE=local` | Python SDK `CompiledContract` + `validate()` — zero gateway dependency, fastest partner laptop path |
| **http** | `CG_MODE=http` + `GATEWAY_URL` + `CG_API_KEY` | Real gateway `/v1/ingest` — proves hosted path |

Both modes must produce the same clean/quarantine routing behavior.

---

## Stages (implementation order)

### Stage A — Scaffold (this PR)

- [x] RFC-092
- [x] `demo/ralph/` layout: README, compose, contracts, bridge, scripts
- [x] Three YAML contracts (orders / customers / products after-images)
- [x] Redpanda compose (reuse patterns from `docker-compose.kafka-demo.yml`)
- [x] `gate_bridge.py` local mode
- [x] Adapted producers writing to `*.raw`
- [x] `inject_bad.py` for one obvious violation per entity
- [x] Makefile target `make demo-ralph` / `make demo-ralph-down` / `demo-ralph-smoke`

### Stage B — Driftless path

- [x] Vendored/adapted `app.py` + `db/` + MCP client consuming **clean** topics
- [x] Attribution + Apache-2.0 NOTICE for xdgrulez code
- [x] End-to-end script: up → produce → bridge → Kafi `join_print` + quarantine (`make demo-ralph-e2e` PASS)
- [x] Document kafi install (`requirements-downstream.txt`)
- [x] MCP server boots and accepts client sessions (query richness depends on embed warmup)

### Stage C — Partner polish (before sending to Ralph)

- [x] `PARTNER.md` + README Stage B notes
- [x] `scripts/inspect_topics.py` (clean vs quarantine counts + samples)
- [x] `scripts/run_demo.sh` one-shot (`make demo-ralph-run` / `demo-ralph-run-mcp`)
- [x] MCP path returns real query results (retries + port reclaim)
- [x] Smoke / e2e / run_demo checklist in README
- [x] DM-ready blurb (`DM_BLURB.txt`)

### Stage D — Optional stretch (post-partner if useful)

- [ ] HTTP gateway mode against `make demo` / local gateway
- [ ] Jupyter notebook mirroring his Current talk style
- [ ] sregi sidebar note (schema export complementary — not required to run)

---

## Contracts (sketch)

**orders.after** — required: `id`, `product_id`, `customer_id`, `status`, `ts`; `status` enum aligned to his generator; `status_id` 0–6.

**customers.after** — required: `id`, `first_name`, `last_name`, `street_address`, `state`, `zip_code`; zip pattern; optional email/phone patterns if present in datagen.

**products.after** — required: `id`, `brand`, `name`, `sale_price`; `sale_price` number min 0.

Bad injectors: missing required field, invalid enum, negative price, malformed zip.

---

## Repo layout

```
demo/ralph/
  README.md
  NOTICE                    # xdgrulez / Apache-2.0 attribution
  docker-compose.yml        # Redpanda (+ console)
  Makefile                  # or root Makefile targets
  requirements.txt
  contracts/
    orders.yaml
    customers.yaml
    products.yaml
  bridge/
    gate_bridge.py
  produce/
    producers.py            # adapted driftless producers → *.raw
    inject_bad.py
    datagen/                # adapted generators + constants
  downstream/
    app.py                  # driftless MCP + Kafi, clean topics
    client.py
    db/
    kafka/
  scripts/
    smoke.sh                # up → wait → inject → assert quarantine nonempty
```

---

## Testing

- `scripts/smoke.sh`: Redpanda healthy → produce N good → bridge → clean lag advances; inject bad → quarantine message present; Driftless process stays up.
- Contract YAMLs parse via Python SDK `CompiledContract`.
- No `cargo` change required for Stage A–C; CI optional later (`demo/ralph` path in a workflow only if cheap).

---

## Risks

| Risk | Mitigation |
|------|------------|
| Driftless/kafi API churn (raw PoC) | Pin `kafi==` in requirements; vendor thin copies we control |
| Heavy deps (fastembed, lancedb, MCP) | Document RAM needs; allow “bridge-only” mode without MCP |
| Debezium unwrap bugs | Golden fixtures for c/u/d in `bridge/tests` or smoke asserts |
| Partner doesn’t run compose | Keep README ≤15 lines to first win; offer screen-share |

---

## Rollout / partner communication

1. Land Stage A+B on this branch; smoke locally.
2. DM Ralph with repo path / zip / branch link — **after** smoke passes.
3. Ask for feedback on DX and whether clean-topic split matches how he’d wire production.
4. Public LO thread: only if he wants; default keep demo private to DM.

---

## Open questions (resolved for v1)

| Question | Decision |
|----------|----------|
| Gateway vs local SDK? | Local default; HTTP optional |
| Validate full envelope or `after`? | `after` only |
| Deletes? | Pass-through to clean |
| Submodule vs vendor? | Vendor adapted files + NOTICE |
| RFC number? | **092** (091 is help-mode on another branch) |
