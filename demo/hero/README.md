# Hero demo — stop bad events, then drain the backlog

A ~15-minute, HTTP-only walkthrough of ContractGate's core value: bad events are
**stopped at ingest**, held in quarantine, and later **replayed** clean once the
contract is corrected — nothing silently hits the warehouse, nothing is lost.

Run it with [`scripts/hero_demo.sh`](../../scripts/hero_demo.sh).

## The story

1. Deploy two versions of the `hero_events` contract:
   - **v1.0.0 (strict)** — allowed methods `GET/POST/PUT/PATCH/DELETE`.
   - **v1.1.0 (relaxed)** — also allows `CONNECT` (the proxy fleet is legit).
2. Good events (`events_pass.json`) ingest and **pass**.
3. A producer still on the old rules emits `CONNECT` events
   (`events_quarantine.json`). Ingested **pinned to v1.0.0**, they fail the strict
   method rule and are **quarantined** — not forwarded.
4. Inspect the quarantine: see the events and their violations
   (`GET /quarantine`).
5. **Replay** the quarantined events against **v1.1.0** (`POST /quarantine/replay`)
   — they now pass. Backlog drained.

## Why two versions up front

ContractGate **blocks deploying a new version while events are quarantined** (a
safety feature — you must consciously handle the backlog first). So the real
workflow is to register the corrected version, then replay against it. The demo
pre-stages both versions and pins the bad batch to v1.0.0 to reproduce that
sequence deterministically.

## Run it

Requires `curl` + `jq` and a running gateway.

```bash
# Hosted / keyed gateway (deploy needs a service-role key):
KEY=cg_live_xxx HOST=https://contractgate-api.fly.dev bash scripts/hero_demo.sh

# Local `make demo` stack (dev-no-auth — pass the demo org id, no key):
ORG_ID=cccccccc-cccc-cccc-cccc-cccccccccccc HOST=http://localhost:8080 \
    bash scripts/hero_demo.sh
```

Each run uses a fresh timestamped contract name, so it's safe to re-run and never
collides with existing contracts. The script fails loudly (prints the response,
exits non-zero) if any step doesn't behave, so a green run is a real green run.

## Files

| File | Purpose |
|---|---|
| `contract_v1.0.0.yaml` | Strict contract (no `CONNECT`). |
| `contract_v1.1.0.yaml` | Relaxed contract (adds `CONNECT`). |
| `events_pass.json` | Valid events — all pass. |
| `events_quarantine.json` | `CONNECT` events — quarantine on v1.0.0, pass on v1.1.0. |
