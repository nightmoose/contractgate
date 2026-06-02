# contractgate (Python SDK)

First-party Python SDK for [ContractGate][gw] — a high-performance
semantic contract enforcement gateway (Patent Pending).

[gw]: https://datacontractgate.com

## Install

```bash
pip install contractgate
```

Python 3.9+. Runtime deps: `httpx`, `PyYAML`.

## Quickstart — HTTP client

```python
from contractgate import Client

cg = Client(base_url="https://gw.example.com", api_key="cg_live_...")

result = cg.ingest(
    contract_id="11111111-1111-1111-1111-111111111111",
    events=[
        {"user_id": "alice_01", "event_type": "click", "timestamp": 1712000000},
    ],
)

print(result.passed, "/", result.total, "events passed")
for r in result.results:
    if not r.passed:
        for v in r.violations:
            print(v.field, v.kind, v.message)
```

Async equivalent:

```python
import asyncio
from contractgate import AsyncClient

async def main():
    async with AsyncClient(base_url="...", api_key="...") as cg:
        result = await cg.ingest(contract_id="...", events=[...])

asyncio.run(main())
```

## Quickstart — local validator

Pure-Python port of the Rust validator. Useful in unit tests and
pre-commit hooks:

```python
from contractgate import Contract

contract = Contract.from_yaml(open("user_events.yaml").read())
compiled = contract.compile()

vr = compiled.validate({
    "user_id": "alice_01",
    "event_type": "click",
    "timestamp": 1712000000,
})
assert vr.passed, vr.violations
```

## Caveats

- **Local validator does not run RFC-004 PII transforms** (`mask`,
  `hash`, `drop`, `redact`). The per-contract salt is server-side
  only. The gateway is the single source of truth for the
  post-transform payload — read it from each per-event result's
  `transformed_event` field.
- **Audit honesty**: every per-event result carries the
  `contract_version` that *actually matched* the event (relevant
  under `multi_stable_resolution: fallback`). Surface it as-is — do
  not substitute the requested version.
- **Retries are off by default.** Layer `httpx.HTTPTransport(retries=)`
  or `tenacity` if you need them. Avoid client-side retry on ingest
  to prevent double-write; use the gateway's quarantine replay
  endpoint instead.
- `httpx` is pinned `>=0.25,<1.0`; we'll widen once 1.x ships.

## License

MIT. See [`LICENSE`](LICENSE).
