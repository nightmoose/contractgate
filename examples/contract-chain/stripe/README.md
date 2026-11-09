# Stripe contract chain (example)

**Linked contracts that walk a Stripe charges export from ingest → stage → mart → egress.**

Public brand: **DataContractGate** (and **AISchemaGate** for LLM output). Not affiliated with Stripe. All sample ids (`ch_3Qdemo…`, `cus_Demo…`, `po_demo_…`) are obviously fake.

This chain is documentation + playground material. It does not add product features.

## Contracts

| Stage | Contract | Gate | Endpoint |
|---|---|---|---|
| (a) Raw ingest | `stripe_charges_raw` | Ingest | `POST /v1/ingest/{contract_id}` |
| (b) Cleaned / typed | `stripe_charges_staged` | Stage | `POST /v1/ingest/{contract_id}` |
| (c) Daily mart | `revenue_daily_mart` | Stage / mart (alongside dbt · GE · Soda) | `POST /v1/ingest/{contract_id}` (or `/egress/...` if publishing the mart) |
| (d) Partner feed | `partner_payout_feed` | Egress | `POST /egress/{contract_id}` |

Linkage is recorded in each contract’s `description` (`derived_from: …` / `chain: …`) and in YAML comments — no new schema fields.

`stripe_charges_raw` mirrors the public catalog seed [`catalog/seeds/stripe-charges`](../../../catalog/seeds/stripe-charges/).

Architecture overview: [`docs/architecture/where-to-enforce-contracts.md`](../../../docs/architecture/where-to-enforce-contracts.md).

## Files

```
stripe_charges_raw.yaml          + .pass.json / .fail.json
stripe_charges_staged.yaml       + .pass.json / .fail.json
revenue_daily_mart.yaml          + .pass.json / .fail.json
partner_payout_feed.yaml         + .pass.json / .fail.json
```

Each `.pass.json` should validate with 0 violations. Each `.fail.json` is a concrete, intentional violation (wrong enum, case, pattern, or range).

## Validate locally (`cg test`)

From the repo root (requires a built CLI — `cargo build -p contractgate --bin contractgate`, then `alias cg=…` / `cg` on PATH):

```bash
cg test --contract examples/contract-chain/stripe/stripe_charges_raw.yaml \
        --data examples/contract-chain/stripe/stripe_charges_raw.pass.json
# expect exit 0

cg test --contract examples/contract-chain/stripe/stripe_charges_raw.yaml \
        --data examples/contract-chain/stripe/stripe_charges_raw.fail.json
# expect exit 1 (amount < 0, currency case, status enum)
```

Repeat for `stripe_charges_staged`, `revenue_daily_mart`, and `partner_payout_feed`.

Python SDK (same engine rules):

```bash
python - <<'PY'
from pathlib import Path
from contractgate.contract import Contract
from contractgate.validator import validate
import json

base = Path("examples/contract-chain/stripe")
for name in [
    "stripe_charges_raw",
    "stripe_charges_staged",
    "revenue_daily_mart",
    "partner_payout_feed",
]:
    compiled = Contract.from_yaml((base / f"{name}.yaml").read_text()).compile()
    ok = validate(compiled, json.loads((base / f"{name}.pass.json").read_text()))
    bad = validate(compiled, json.loads((base / f"{name}.fail.json").read_text()))
    assert ok.passed and not bad.passed, name
    print(name, "pass=OK fail=OK", [v.kind for v in bad.violations][:3])
PY
```

## Playground (hosted)

1. Open [Playground / ODCS generator](https://datacontractgate.com/odcs-generator.html) or the app contract editor.
2. Paste a YAML file from this folder as the contract.
3. Paste the matching `.pass.json` — expect accept.
4. Paste `.fail.json` — expect the concrete violation(s) called out below.

Or create four contracts in the app from these YAMLs, then call the gateway with your API key:

```bash
# Ingest / stage / mart (same path; different contract_id)
curl -sS -X POST "https://api.datacontractgate.com/v1/ingest/$CONTRACT_ID?dry_run=true" \
  -H "X-Api-Key: $DCG_API_KEY" -H "Content-Type: application/json" \
  --data @examples/contract-chain/stripe/stripe_charges_raw.pass.json

# Egress (partner feed) — forward only response.payload
curl -sS -X POST "https://api.datacontractgate.com/egress/$CONTRACT_ID?dry_run=true" \
  -H "X-Api-Key: $DCG_API_KEY" -H "Content-Type: application/json" \
  --data @examples/contract-chain/stripe/partner_payout_feed.pass.json
```

Auth: `X-Api-Key`. Prefer `?dry_run=true` while wiring examples so nothing is persisted or metered.

## What each `.fail.json` is meant to catch

| Contract | Example violation in `.fail.json` |
|---|---|
| `stripe_charges_raw` | `amount: -50` (min 0); `currency: "USD"` (must be lowercase); `status: "refunded"` (not in enum) |
| `stripe_charges_staged` | `currency: "usd"` (must be uppercase); `charged_at` missing `T…Z` ISO shape |
| `revenue_daily_mart` | `revenue_date` not `YYYY-MM-DD`; `status: "paid"`; `charge_count: -1` |
| `partner_payout_feed` | `payout_id` missing `po_demo_` prefix; `amount: 0` (< 0.01); `status: "queued"` |

## Orchestrator note

Pending quarantine for these contract ids is the signal back to Airflow/dbt — see [`examples/airflow/`](../../airflow/) and [`examples/dbt/`](../../dbt/). There is no per-run `batch_id` yet; filter by `contract_id` + time window + `status=pending`.
