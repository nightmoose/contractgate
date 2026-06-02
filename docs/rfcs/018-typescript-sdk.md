# RFC-018: TypeScript SDK

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #5                                                        |
| Supersedes    | RFC-011 partially (TS only; Go + Java deferred)                        |
| Depends on    | RFC-005 (Python SDK shape reference), RFC-014 (CLI freezes wire shape) |

## Summary

Single first-party SDK in v0.1: TypeScript / Node.js. npm package
`@contractgate/sdk`. Mirrors the Python SDK (RFC-005) shape: HTTP
client + local validator + audit reads.

Go and Java SDKs are deferred until pilot demand makes one of them
the obvious next pick.

## Goals

1. TypeScript SDK published as `@contractgate/sdk` on npm.
2. ESM only in v0.1 (Node 20+). Skip CJS dual to ship faster.
3. Same response model names and error hierarchy as Python SDK
   (`ContractGateError → HTTPError, ValidationError, ContractCompileError,
   AuthError`).
4. Local validator parity with Rust + Python via shared JSON fixtures
   under `tests/conformance/`.
5. Zero runtime deps beyond `js-yaml`. Use built-in Node `fetch`.

## Non-goals

- Go SDK.
- Java SDK.
- Browser bundle. Node only in v0.1.
- CJS dual build.
- PII transform support in local validator. Server-side only (same rule
  as RFC-005).
- Auto-retry / circuit breakers. Document `fetch` retry pattern in README;
  defer to consumer.
- Codegen from OpenAPI. Hand-written types in v1; `utoipa`-generated spec
  is a future RFC.

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Repo location | **`sdks/typescript/`** within this repo (monorepo). Same pattern as `sdks/python/`. |
| Q2 | Module format | **ESM only** (Node 20+). |
| Q3 | HTTP transport | **Built-in `fetch`** (Node 20 has it). No `undici` direct dep. |
| Q4 | YAML parser | **`js-yaml`**. |
| Q5 | Versioning | **Lockstep with gateway minor.** SDK 0.5.x ↔ gateway 0.5.x. |
| Q6 | Conformance fixtures | **`tests/conformance/*.json`** at repo root, shared with Python and Rust. New corpus this RFC builds. |
| Q7 | Public API shape | **Mirrors Python SDK** — `Client`, `Contract.fromYaml`, `compile`, `validate`. |
| Q8 | Codegen | **No.** Hand-written types in `src/types.ts`. Auto-gen is a follow-up RFC once OpenAPI spec exists. |

## Current state

- Python SDK shipped (RFC-005). Treat as DX reference.
- No OpenAPI spec on disk.
- No `tests/conformance/` corpus.

## Design

### Layout

```
sdks/typescript/
├── package.json                # @contractgate/sdk, ESM, type=module
├── tsconfig.json               # strict, target ES2022
├── src/
│   ├── index.ts                # public re-exports
│   ├── client.ts               # HTTP client (fetch)
│   ├── types.ts                # hand-written wire types
│   ├── contract.ts             # YAML → Contract parse + compile
│   ├── validator.ts            # local validate()
│   └── errors.ts               # error hierarchy
├── tests/
│   ├── client.test.ts
│   ├── validator.test.ts
│   └── conformance.test.ts     # walks ../../tests/conformance/*.json
└── README.md
```

### Public API

```ts
import { Client, Contract } from "@contractgate/sdk";

const c = new Client({ baseUrl: "http://localhost:8080", apiKey: process.env.CG_KEY });

const result = await c.ingest({ contractId, events: [...] });
for (const r of result.results) {
  if (!r.passed) for (const v of r.violations) console.log(v.field, v.kind, v.message);
}

const contract = Contract.fromYaml(yamlString);
const compiled = contract.compile();
const vr = compiled.validate({ user_id: "alice_01", event_type: "click", timestamp: 1700000000 });
console.assert(vr.passed, vr.violations);
```

### Conformance fixture format

```json
// tests/conformance/event_user_id_too_short.json
{
  "name": "user_id below min_length is rejected",
  "contract_yaml": "version: \"1.0\"\nname: ...\n...",
  "event": {"user_id": "x", "event_type": "click", "timestamp": 1700000000},
  "expected": {
    "passed": false,
    "violations": [
      {"field": "user_id", "kind": "min_length", "message": "..."}
    ]
  }
}
```

Each runner (Rust, Python, TypeScript) walks the fixture dir, parses
`contract_yaml`, runs its local validator on `event`, asserts equality
against `expected`. Any drift fails CI in three places at once.

The Python SDK's existing parity tests are migrated to this corpus
as part of this RFC's rollout.

### Errors

```ts
export class ContractGateError extends Error {}
export class HTTPError extends ContractGateError {
  constructor(public status: number, public body: string) { super(`HTTP ${status}`); }
}
export class ValidationError extends ContractGateError {}
export class ContractCompileError extends ContractGateError {}
export class AuthError extends ContractGateError {}
```

## Test plan

- `tests/client.test.ts` — mock fetch, assert headers, body, error mapping.
- `tests/validator.test.ts` — direct unit tests on `compile()` + `validate()`
  semantics.
- `tests/conformance.test.ts` — runs every fixture under
  `tests/conformance/`. Same suite passes for Rust + Python + TS.
- Integration smoke: spin gateway via Compose (RFC-017), run a real
  ingest from the SDK, assert response.

## Rollout

1. Sign-off this RFC.
2. Build out `tests/conformance/` corpus from existing Rust unit tests.
   Target ~30 fixtures covering every `ViolationKind`.
3. Wire Python SDK to consume the corpus (replace its inline parity tests).
4. Wire Rust to consume the corpus from a new test module.
5. `sdks/typescript/` scaffold + `package.json` + tsconfig.
6. `client.ts` (HTTP first; forces auth + error plumbing).
7. `contract.ts` + `validator.ts` (local validation).
8. `tests/conformance.test.ts` runner.
9. Publish `@contractgate/sdk@0.5.0` to npm.
10. `cargo check && cargo test`; `cd sdks/typescript && npm test`.
11. Update `MAINTENANCE_LOG.md`.

## Deferred

- Go SDK.
- Java SDK.
- Browser bundle.
- CJS build.
- OpenAPI-driven codegen (separate RFC; needs `utoipa` annotations on
  the gateway first).
- PyO3-backed Python validator.
- Auto-retry helper.
- Streaming ingest (no streaming API exists).
