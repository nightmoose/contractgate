# Changelog

All notable changes to the `contractgate` Python SDK.

## 0.1.0 — 2026-04-28

Initial scaffold per RFC-005.

- `Client` (sync) + `AsyncClient` (async) over httpx.
- `ingest`, `audit`, `get_contract`, `get_version`, `playground`.
- Local validator (`Contract.from_yaml`, `CompiledContract.validate`).
- Strict parity with Rust validator: same `ViolationKind`, same field
  paths, same message text. Locked via shared fixture corpus.
- No PII transforms in the local validator (RFC-004 invariant).
