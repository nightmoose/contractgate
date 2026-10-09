# @nightmoose/contractgate-sdk

Official TypeScript SDK for [ContractGate](https://datacontractgate.com) — HTTP client + local validator.

**Node 20+, ESM only.**

## Install

```bash
npm install @nightmoose/contractgate-sdk
```

## Quick start

```ts
import { Client, Contract } from "@nightmoose/contractgate-sdk";

// HTTP client
const c = new Client({ baseUrl: "https://gw.example.com", apiKey: process.env.CG_KEY });

const result = await c.ingest({ contractId: "...", events: [{ user_id: "alice_01", event_type: "click", timestamp: 1700000000 }] });
for (const r of result.results) {
  if (!r.passed) for (const v of r.violations) console.log(v.field, v.kind, v.message);
}

// Local validator (no network)
const contract = Contract.fromYaml(yamlString);
const compiled = contract.compile();
const vr = compiled.validate({ user_id: "alice_01", event_type: "click", timestamp: 1700000000 });
console.assert(vr.passed, vr.violations);
```

## Client API

| Method | Description |
|--------|-------------|
| `ingest({ contractId, events, version?, dryRun?, atomic? })` | Validate + persist a batch |
| `egress({ contractId, events, version?, disposition?, dryRun? })` | Validate outbound payload |
| `audit({ contractId?, limit?, offset? })` | Read audit entries |
| `stats()` | Global ingestion stats |
| `getContract(id)` | Fetch contract metadata |
| `listContracts()` | List all contracts |
| `listVersions(contractId)` | List versions |
| `getVersion(contractId, version)` | Fetch a specific version |
| `getLatestStable(contractId)` | Fetch the latest stable version |
| `playgroundValidate({ yamlContent, event })` | Validate without persisting |

## Local validator

```ts
import { Contract } from "@nightmoose/contractgate-sdk";

const contract = Contract.fromYaml(`
version: "1.0"
name: "events"
ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: timestamp
      type: integer
      required: true
      min: 0
`);

const compiled = contract.compile();
const { passed, violations } = compiled.validate({ user_id: "alice", timestamp: 1700000000 });
```

The local validator mirrors the Rust engine exactly. Conformance is enforced via a shared fixture corpus under `tests/conformance/`.

## Error hierarchy

```
ContractGateError
├── ContractCompileError     — bad contract YAML
├── ConnectionError          — network / DNS failure
└── HTTPError
    ├── BadRequestError      — 400
    ├── AuthError            — 401
    ├── NotFoundError        — 404
    ├── ConflictError        — 409
    ├── ValidationFailedError — 422 (whole-batch rejection)
    └── ServerError          — 5xx
```

Per-event validation failures in a 207 Multi-Status response do **not** raise. They surface in `result.results[n].violations`.

## Version policy

SDK version is kept in lockstep with the ContractGate gateway minor version. `0.1.x` of the SDK works with gateway `0.1.x`.
