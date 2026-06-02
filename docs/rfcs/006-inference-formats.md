# RFC-006 — Multi-Format Contract Inference

**Status:** Accepted (2026-04-27)
**Chunk:** Punchlist 01 — Inference Family Completion

---

## Goals

Extend `POST /contracts/infer` (JSON-only today) to three additional schema
formats — Avro, Protobuf, and OpenAPI/AsyncAPI — plus an evolution diff
summarizer that produces plain-English changelogs between two contract versions.

## Non-goals

- Binary Avro OCF container decoding (deferred; `apache_avro` dep adds build
  complexity — revisit when needed).
- LLM-backed diff prose (plug-in point reserved; rule-based ships first).
- AsyncAPI channel-level inference beyond extracting `components/schemas`
  (same code path as OpenAPI).
- Multi-tenant auth on the new routes (same `x-api-key` guard as existing
  protected routes).

---

## Decisions

| # | Question | Decision |
|---|---|---|
| Q1 | Routing | Per-format routes: `/contracts/infer/avro`, `/infer/proto`, `/infer/openapi` |
| Q2 | Avro / Proto approach | Schema-driven primary; JSON sample array accepted as secondary input on Avro |
| Q3 | Diff prose | Rule-based templates; `DiffSummarizer` trait reserves an LLM backend slot |
| Q4 | RFC required | Yes — this RFC |

---

## New Routes

All routes are protected (require `x-api-key`).

| Method | Path | Handler |
|--------|------|---------|
| POST | `/contracts/infer/avro` | `infer_avro::infer_avro_handler` |
| POST | `/contracts/infer/proto` | `infer_proto::infer_proto_handler` |
| POST | `/contracts/infer/openapi` | `infer_openapi::infer_openapi_handler` |
| POST | `/contracts/diff` | `infer_diff::diff_handler` |

---

## Avro (`src/infer_avro.rs`)

### Request

```json
{
  "name": "my_contract",
  "description": "optional",
  "schema": "<avsc JSON string — schema-driven>",
  "samples": [{"field": "value"}, ...]
}
```

Exactly one of `schema` or `samples` must be provided. If both are present,
`schema` takes precedence (schema-driven wins; samples are ignored).

### Avro → FieldType mapping

| Avro type | FieldType |
|-----------|-----------|
| `"null"` | skipped (marks field optional) |
| `"string"`, `"bytes"` | `String` |
| `"int"`, `"long"` | `Integer` |
| `"float"`, `"double"` | `Float` |
| `"boolean"` | `Boolean` |
| `"record"` | `Object` (recurse) |
| `"array"` | `Array` |
| `"map"` | `Object` |
| `"enum"` | `String` + `allowed_values` |
| union `["null", T]` | T with `required: false` |
| union of 2+ non-null types | `Any` |

### Schema-driven walk

Parse the `.avsc` string as JSON, identify the top-level `"record"` type, and
walk `fields`. Nested `"record"` types recurse into `properties`. This is
deterministic and needs no extra crate (`.avsc` is JSON).

### Sample-driven fallback

When only `samples` (JSON objects) are given, delegate directly to
`infer::infer_fields_from_objects` — identical to the existing JSON route.
The format difference is only in the route; the inference logic is reused.

---

## Protobuf (`src/infer_proto.rs`)

### Request

```json
{
  "name": "my_contract",
  "description": "optional",
  "proto_source": "syntax = \"proto3\";\nmessage Event { ... }",
  "message": "Event"
}
```

`message` names which top-level message to use as the contract root.
Defaults to the first message found.

### Proto → FieldType mapping

| Proto scalar | FieldType |
|---|---|
| `string`, `bytes` | `String` |
| `int32`, `int64`, `uint32`, `uint64`, `sint32`, `sint64`, `fixed32`, `fixed64`, `sfixed32`, `sfixed64` | `Integer` |
| `float`, `double` | `Float` |
| `bool` | `Boolean` |
| nested `message` type | `Object` (recurse) |
| `repeated T` | `Array` (items: T) |
| `enum` | `String` + `allowed_values` |
| `optional T` | T with `required: false` |

### Parser strategy

Hand-written line-oriented parser (no extra dep):
1. Strip comments (`//` and `/* */`).
2. Find `message <Name> { ... }` blocks by brace-matching.
3. Within a block, parse each field line: `[optional|repeated] <type> <name> = <tag>;`.
4. Collect `enum` blocks for `allowed_values`.
5. Recurse into nested message types.

Covers proto3 ≥90% of real-world schemas. Exotic features (oneof, map<K,V>,
Any, extensions) fall back to `FieldType::Any` with a note in `description`.

---

## OpenAPI / AsyncAPI (`src/infer_openapi.rs`)

### Request

```json
{
  "name": "my_contract",
  "description": "optional",
  "openapi_source": "<yaml or json string>",
  "schema_name": "Event"
}
```

`schema_name` selects a schema from `components/schemas` (OpenAPI 3.x) or
`components/schemas` inside the AsyncAPI envelope. If omitted, the first
schema found is used.

### JSON Schema → FieldType mapping

| JSON Schema type | FieldType |
|---|---|
| `"string"` | `String` |
| `"integer"` | `Integer` |
| `"number"` | `Float` |
| `"boolean"` | `Boolean` |
| `"object"` | `Object` (recurse `properties`) |
| `"array"` | `Array` (recurse `items`) |
| no `type`, has `properties` | `Object` |
| otherwise | `Any` |

`required` array at the object level marks individual fields required.
`enum` values map to `allowed_values`. `minimum`/`maximum` map to `min`/`max`.
`pattern` passes through. `minLength`/`maxLength` pass through.

---

## Evolution Diff Summarizer (`src/infer_diff.rs`)

### Request

```json
{
  "contract_yaml_a": "<yaml string — older version>",
  "contract_yaml_b": "<yaml string — newer version>"
}
```

### Response

```json
{
  "summary": "3 changes: 1 field added, 1 field removed, 1 type changed.",
  "changes": [
    {"kind": "field_added",   "field": "session_id", "detail": "string, required"},
    {"kind": "field_removed", "field": "legacy_id",  "detail": "string, optional"},
    {"kind": "type_changed",  "field": "amount",     "detail": "integer → float"}
  ]
}
```

### Change kinds (rule-based)

| Kind | Trigger |
|---|---|
| `field_added` | field in B not in A |
| `field_removed` | field in A not in B |
| `type_changed` | same field, different `field_type` |
| `required_changed` | same field, `required` flipped |
| `enum_value_added` | value in B.allowed_values not in A |
| `enum_value_removed` | value in A.allowed_values not in B |
| `pattern_changed` | same field, different `pattern` |
| `constraint_changed` | min/max/min_length/max_length changed |

### LLM extension point

```rust
pub trait DiffSummarizer: Send + Sync {
    fn summarize(&self, changes: &[DiffChange]) -> String;
}
pub struct RuleBasedSummarizer;   // ships now
// pub struct LlmSummarizer { ... }  // future
```

The handler takes `Arc<dyn DiffSummarizer>` via app state, defaulting to
`RuleBasedSummarizer`.  Swapping in an LLM backend requires only a new impl —
no handler changes.

---

## Test Plan

Each module carries inline unit tests:
- Avro: round-trip a record schema with union nullable fields, enum, nested record.
- Proto: parse a proto3 message with all scalar types, optional, repeated, enum.
- OpenAPI: parse a YAML schema with required array, enum, nested object.
- Diff: field added/removed, type change, required flip, enum delta.

---

## Rollout

1. ✅ RFC sign-off
2. `src/infer_avro.rs` + route
3. `src/infer_proto.rs` + route
4. `src/infer_openapi.rs` + route
5. `src/infer_diff.rs` + route
6. Wire all routes in `main.rs`
7. `cargo check && cargo test`
8. Update `MAINTENANCE_LOG.md`
9. Parity fixture corpus (`tests/fixtures/infer/`) — deferred to next session
