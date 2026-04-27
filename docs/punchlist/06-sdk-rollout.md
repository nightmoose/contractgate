# Chunk 6 — SDK Rollout

**Theme:** TS, Go, Java clients. Mirror the Python SDK shape.
**Why now:** Depends on stable CLI surface (Chunk 2) — SDKs and CLI share the API contract; freeze it once.

## Items

- [ ] Node.js / TypeScript SDK (npm) `[M]` — first to ship; broadest surface area in target buyers.
- [ ] Go SDK `[L]` — second; idiomatic ctx-first, errors-as-values.
- [ ] Java SDK (Maven + Gradle) `[L]` — last; longest tail of release pipeline setup.

Each SDK exposes: `publish`, `fetch`, `validate`, plus typed event hooks for downstream pipeline connectors.

## Hard dependency

- Chunk 2 CLI must lock the wire format and config grammar before SDK work starts. Otherwise three clients drift.

## Pairs well with (later)

- Spark / Flink connectors built on top of Java + Scala wrappers.
- Airflow provider built on top of Python SDK (already shipped).
- dbt package — independent.

## Open questions for the conversation

1. Codegen vs hand-written? OpenAPI spec → generated client guarantees parity. Hand-written gives nicer ergonomics.
2. Versioning strategy — lockstep with server, or independent semver per SDK?
3. Distribution: npm under `@contractgate/sdk`? Maven coords? Go module path?
4. Test strategy — shared conformance suite each SDK runs against a live server fixture?
5. RFC required for the conformance suite shape; per-SDK impl is skip-RFC.

## Suggested first step

Publish the OpenAPI spec for the gateway as a checked-in artifact. TS SDK consumes it. The spec itself becomes the SDK contract.
