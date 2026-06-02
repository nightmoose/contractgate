"""
ContractGate v1 ingest — Python example (extends the existing SDK).

Requires: pip install contractgate   (or the local SDK from sdks/python/)

Run against the public demo:
    export CONTRACTGATE_API_KEY=cg_live_<your_key>
    export CONTRACTGATE_CONTRACT_ID=<uuid>
    python python_example.py
"""

import os
import contractgate  # existing SDK — do not duplicate validation logic

BASE_URL    = os.getenv("CONTRACTGATE_BASE_URL", "https://contractgate.io")
API_KEY     = os.environ["CONTRACTGATE_API_KEY"]
CONTRACT_ID = os.environ["CONTRACTGATE_CONTRACT_ID"]

# ---------------------------------------------------------------------------
# Option A — use the SDK client (recommended)
# ---------------------------------------------------------------------------

client = contractgate.Client(api_key=API_KEY, base_url=BASE_URL)

result = client.ingest(
    contract_id=CONTRACT_ID,
    events=[
        {"user_id": "u_001", "event_type": "login",    "timestamp": 1_714_000_000},
        {"user_id": "u_002", "event_type": "purchase", "timestamp": 1_714_000_001, "amount": 99.0},
        {"user_id": "u_003", "event_type": "bad_type", "timestamp": 1_714_000_002},  # will fail
    ],
    version="1.0.0",   # optional — defaults to latest stable
    dry_run=False,
    atomic=False,
)

print(f"total={result.total}  passed={result.passed}  failed={result.failed}")
for r in result.results:
    status = "✓" if r.passed else "✗"
    qid    = f"  quarantine_id={r.quarantine_id}" if r.quarantine_id else ""
    print(f"  [{status}] index={r.index}  version={r.contract_version}{qid}")

# ---------------------------------------------------------------------------
# Option B — raw httpx (no SDK dependency)
# ---------------------------------------------------------------------------

import httpx

with httpx.Client(base_url=BASE_URL, headers={"X-Api-Key": API_KEY}) as http:
    resp = http.post(
        f"/v1/ingest/{CONTRACT_ID}",
        json=[
            {"user_id": "u_raw", "event_type": "view", "timestamp": 1_714_000_100},
        ],
        headers={"Idempotency-Key": "python-example-001"},
    )
    resp.raise_for_status()
    data = resp.json()
    print(f"\nraw httpx: passed={data['passed']}")
    print(f"  X-Idempotency-Replay: {resp.headers.get('x-idempotency-replay', 'n/a')}")
