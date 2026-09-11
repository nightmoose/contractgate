# Note for Ralph (design partner)

Hey Ralph — here’s a runnable demo that puts **ContractGate in front of Driftless / Kafi Streams**, using your shoe CDC datagen and clean-topic join path.

## Fastest path

```bash
# clone ContractGate, then:
make demo-ralph-run-mcp
```

That script will:

1. Start Redpanda
2. Run ContractGate on `*.raw` → clean / quarantine
3. Run Kafi Streams joins on **clean topics only**
4. Inject bad CDC (bad status, bad email/zip, negative price)
5. Print quarantine samples
6. Start Driftless MCP and run `search_customer_context` with query `delivered`

You should see a final **`DEMO PASS`**.

Interactive poking afterward:

```bash
./scripts/run_demo.sh --keep
# then in another shell:
python scripts/inspect_topics.py
python downstream/client.py --query "delivered"
# Console UI: http://localhost:8088
```

## Architecture (your pieces)

| Piece | Role |
|-------|------|
| Your datagen / CDC envelopes | `produce/` |
| **ContractGate** | `bridge/gate_bridge.py` |
| **Kafi Streams** | `downstream/join_print.py` + `downstream/kafka/` |
| **Driftless** LanceDB + MCP | `downstream/app.py` + `client.py` |

Topic names Driftless already expects (`orders` / `customers` / `products`) are preserved on the clean side.

## Feedback I’m hoping for

- Does `*.raw` → clean / quarantine match how you’d wire this for real?
- Anything awkward vs Kafi / Driftless DX?
- What would you want next?

No publicity ask — just use + honesty. Thanks for open-sourcing the PoC.
