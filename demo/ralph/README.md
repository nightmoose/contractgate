# Ralph-native demo (RFC-092)

ContractGate in front of **Driftless / Kafi Streams**: CDC producers → `*.raw` → gate → clean topics → joins → LanceDB / MCP.

Adapted from [xdgrulez/driftless](https://github.com/xdgrulez/driftless) (Apache-2.0). See `NOTICE`.

```
producers  →  *.raw  →  gate_bridge (ContractGate)  →  orders/customers/products
                                              ↘ *.quarantine
                                                      ↓
                                              Kafi join → LanceDB → MCP
```

## One-shot (recommended)

From **repo root** (Docker Desktop running):

```bash
make demo-ralph-run          # gate + Kafi joins + quarantine inspect
# or
make demo-ralph-run-mcp      # same + Driftless MCP query
```

From `demo/ralph`:

```bash
./scripts/run_demo.sh
./scripts/run_demo.sh --with-mcp
./scripts/run_demo.sh --keep          # leave processes up for poking
```

Expect a final **`DEMO PASS`** with join count + quarantine samples.

Partner-facing runbook: **`PARTNER.md`**.

## Manual terminals (optional)

```bash
cd demo/ralph
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements-downstream.txt
docker compose up -d                  # Console http://localhost:8088

python bridge/gate_bridge.py          # A
python produce/producers.py           # B
python downstream/join_print.py       # C — wait ~30–45s for joins
python produce/inject_bad.py
python scripts/inspect_topics.py

# MCP (optional)
cd downstream && python app.py        # :8000
python client.py --query "delivered"
```

## What “good” looks like

| Check | Expectation |
|-------|-------------|
| Clean topics | Valid after-images only |
| `*.quarantine` | Bad status / email / zip / negative price |
| `join_print` | `[join] #N …` after warmup |
| MCP `--query delivered` | JSON summaries with scores |
| Story | Poison never reaches joins / agent memory |

## Make targets

| Target | What |
|--------|------|
| `make demo-ralph` | Start Redpanda only |
| `make demo-ralph-smoke` | Gate + quarantine only |
| `make demo-ralph-e2e` | Gate + Kafi joins + quarantine |
| `make demo-ralph-run` | Full partner demo script |
| `make demo-ralph-run-mcp` | Full demo + MCP |
| `make demo-ralph-down` | Stop Redpanda + wipe volume |

## Modes

- **local (default):** Python SDK validator in `gate_bridge.py` — no gateway process.
- **HTTP gateway:** Stage D in RFC-092 (not required for this partner demo).

## Known limitations

- First `pip install -r requirements-downstream.txt` downloads embedding weights (needs network).
- Three-way Kafi join needs ~30–45s warmup before join lines appear.
- Port **8000** must be free for MCP (`run_demo.sh --with-mcp` frees a stale listener).
- `inject_bad` always uses the same order id `999001` — quarantine count grows across runs; wipe with `make demo-ralph-down`.
- This is a **design-partner PoC**, not a production deployment guide.

## Attribution

Datagen + Driftless MCP/join path: Ralph M. Debusmann / xdgrulez.  
Contracts + gate wiring: ContractGate.
