# RFC-011: SDK Rollout (TS, Go, Java)

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 06 — SDK Rollout                                             |
| Depends on    | RFC-005 (Python SDK shape), RFC-007 (CLI / wire-shape freeze)          |

## Summary

Three additional first-party SDKs, mirroring the Python SDK (RFC-005) shape:

1. **Node.js / TypeScript** — npm `@contractgate/sdk`. Ships first.
2. **Go** — module `github.com/contractgate/contractgate-go`. Ships second.
3. **Java** — Maven `dev.contractgate:contractgate-sdk`, Gradle-compatible.
   Ships last.

Each SDK exposes `publish`, `fetch`, `validate` (HTTP + local), and
typed event hooks. Wire shape is locked by an OpenAPI spec checked into
the repo, which becomes the contract every SDK conforms to.

## Goals

1. Three SDKs, identical capability surface, same response model names.
2. **Conformance suite** — shared JSON fixtures (event + expected
   violations) every SDK runs against. Drift is impossible to land silently.
3. **OpenAPI spec is the source of truth** — `openapi/contractgate.yaml`
   regenerated on each gateway change; CI fails if SDK clients are
   out of date relative to it.
4. Versioning: SDKs lockstep with gateway minor versions
   (gateway 0.4.x → SDK 0.4.x).
5. TS: ESM + CJS dual build, zero runtime deps beyond `undici` (Node 18+
   built-in fetch is preferred where available).
6. Go: zero deps beyond stdlib + `gopkg.in/yaml.v3`.
7. Java: minimal deps — `com.fasterxml.jackson.core:jackson-databind` +
   `org.yaml:snakeyaml`. Java 17+.

## Non-goals

- C# / .NET SDK — separate RFC if/when demand exists.
- Rust SDK — the gateway crate is already the reference; a wrapper crate
  is unnecessary.
- Pure-language local validators with PII transform support. Same rule
  RFC-005 set: local validators report pass/fail only; transforms are
  server-side.
- Streaming ingest. No streaming API on the gateway.
- Browser bundle. TS SDK targets Node only in v0.1.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Codegen vs hand-written | **Hand-written transport, codegen'd response models from OpenAPI.** Best of both: nice ergonomics, zero drift on shapes. |
| Q2 | Versioning | **Lockstep with gateway minor.** Patch versions independent. |
| Q3 | TS distribution | **`@contractgate/sdk` on npm, ESM + CJS dual.** No browser entry in v0.1. |
| Q4 | Go module path | **`github.com/contractgate/contractgate-go`** (separate repo, vanity import deferred). |
| Q5 | Java coords | **`dev.contractgate:contractgate-sdk:0.4.0`** on Maven Central via Sonatype. |
| Q6 | Conformance fixtures | **`tests/conformance/*.json`** at repo root, shared by all SDKs (and Python). Fixture format documented in `tests/conformance/README.md`. |
| Q7 | OpenAPI source generation | **`utoipa` crate** annotates handlers; `cargo run --bin gen-openapi` writes `openapi/contractgate.yaml`; CI asserts file unchanged. |
| Q8 | SDK repo layout | **Each SDK in its own repo.** `sdks-ts`, `sdks-go`, `sdks-java`. Conformance fixtures fetched as a git submodule or release artifact. |

## Current state

- Python SDK shipped (RFC-005). Treat as DX reference.
- No OpenAPI spec exists today. Wire shapes documented per-RFC and in
  `src/*.rs` types.
- `utoipa` not yet a dep.
- `tests/conformance/` does not exist.

## Design

### OpenAPI spec generation

```rust
// src/main.rs (additive)
#[derive(OpenApi)]
#[openapi(
    paths(
        ingest::ingest_handler,
        ingest::batch_ingest_handler,
        validation::validate_handler,
        contract::create_contract_handler,
        contract::list_contracts_handler,
        contract::get_contract_handler,
        contract::create_version_handler,
        infer::infer_handler,
        infer_diff::diff_handler,
        // ... all public routes
    ),
    components(schemas(
        Contract, FieldDefinition, FieldType, Ontology,
        IngestEventResult, BatchIngestResponse, Violation, ViolationKind,
        // ... all wire types
    ))
)]
struct ApiDoc;

// src/bin/gen-openapi.rs
fn main() {
    let yaml = ApiDoc::openapi().to_yaml().unwrap();
    std::fs::write("openapi/contractgate.yaml", yaml).unwrap();
}
```

CI lane: run `cargo run --bin gen-openapi`, assert `git diff` is empty.
Forces every wire-shape change to also update the spec.

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

Each SDK's test runner walks the fixture dir, parses `contract_yaml`,
runs its local validator on `event`, asserts equality against `expected`.

The Rust gateway has the same runner — drift between Rust and any SDK
fails CI in two places.

### TS SDK (`sdks-ts`)

```
sdks-ts/
├── package.json                # @contractgate/sdk, dual ESM/CJS
├── tsconfig.json               # strict
├── src/
│   ├── index.ts                # public re-exports
│   ├── client.ts               # HTTP client (uses undici / fetch)
│   ├── types.ts                # codegen'd from openapi/contractgate.yaml
│   ├── contract.ts             # YAML → Contract parse + compile
│   ├── validator.ts            # local validate()
│   └── errors.ts               # ContractGateError hierarchy
├── tests/
│   ├── client.test.ts
│   ├── validator.test.ts
│   └── conformance.test.ts     # walks ../tests/conformance/*.json
└── README.md
```

Public API:

```ts
import { Client, Contract, ValidationResult } from "@contractgate/sdk";

const c = new Client({ baseUrl: "...", apiKey: "..." });
const result = await c.ingest({ contractId: "...", events: [...] });

const contract = Contract.fromYaml(yamlString);
const compiled = contract.compile();
const vr = compiled.validate({ user_id: "alice_01", ... });
```

### Go SDK (`sdks-go`)

```
sdks-go/
├── go.mod
├── client.go                   # http.Client wrapper, ctx-first
├── types.go                    # codegen'd from openapi
├── contract.go                 # YAML parse, compile
├── validator.go                # local validate
├── errors.go                   # error types
└── conformance_test.go         # runs shared fixtures
```

Public API:

```go
c := contractgate.NewClient("https://...", "apikey")
res, err := c.Ingest(ctx, contractID, events)

contract, err := contractgate.ParseContract(yamlBytes)
compiled, err := contract.Compile()
vr := compiled.Validate(event)
```

### Java SDK (`sdks-java`)

```
sdks-java/
├── build.gradle.kts
├── src/main/java/dev/contractgate/sdk/
│   ├── Client.java
│   ├── Contract.java
│   ├── Validator.java
│   ├── types/      # POJOs codegen'd
│   └── errors/
└── src/test/java/...
```

Public API:

```java
Client c = Client.builder()
    .baseUrl("https://...")
    .apiKey("...")
    .build();
IngestResult res = c.ingest(contractId, events);

Contract contract = Contract.fromYaml(yamlString);
CompiledContract compiled = contract.compile();
ValidationResult vr = compiled.validate(event);
```

## Test plan

- Each SDK runs the conformance suite in CI.
- Each SDK runs an integration test against a `docker compose` gateway.
- `cargo run --bin gen-openapi` no-diff check in this repo's CI.
- Snapshot tests on each SDK's response models so wire-format changes in
  the OpenAPI spec ripple loudly.

## Rollout

1. Sign-off this RFC.
2. Add `utoipa` annotations to gateway handlers + types. Generate
   `openapi/contractgate.yaml`. Add CI no-diff lane.
3. Build out `tests/conformance/` corpus from existing Rust unit tests.
   Wire the Python SDK to consume it (back-fill RFC-005's parity tests).
4. **TS SDK** — repo, build, codegen, transport, validator, conformance.
   Publish `@contractgate/sdk@0.4.0`.
5. **Go SDK** — same shape.
6. **Java SDK** — same shape, plus Sonatype publishing dance.
7. Update `MAINTENANCE_LOG.md`.

## Deferred

- C# / .NET.
- Rust wrapper crate.
- Browser-only TS bundle.
- PyO3-backed Python validator (RFC-005 deferred item).
