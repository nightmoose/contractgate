# Chunk 1 — Inference Family Completion

**Theme:** Finish what `POST /contracts/infer` started.
**Why first:** Builds directly on this morning's endpoint. Reuses `infer_fields_from_objects` shape. Ships visible AI story fast.

## Items

- [ ] Avro sample inference `[M]` — accept `.avsc` or Avro-encoded payloads, walk schema → `Contract`.
- [ ] Protobuf sample inference `[M]` — parse `.proto` definition or descriptor, map to `FieldDefinition`.
- [ ] OpenAPI / AsyncAPI inference `[M]` — derive contract from `components.schemas` / channels.
- [ ] Evolution diff summarizer `[S]` — plain-English changelog between two contract versions. Builds on versioning + audit logging.

## Existing surface to reuse

- `src/infer.rs` — handler, `InferRequest`/`InferResponse`, type-merging logic.
- `crate::contract::{Contract, FieldDefinition, FieldType, Ontology}`.
- Versioning store (already shipped) for diff summarizer input.

## Open questions for the conversation

1. New endpoint per format (`/contracts/infer/avro`, `/openapi`) or content-type sniffing on the existing route?
2. Avro / Proto: schema-driven (parse the schema file directly) vs sample-driven (decode N records, infer)? Schema-driven is deterministic and faster.
3. Diff summarizer: rule-based templates (cheap, deterministic) or LLM-backed (richer prose, cost + latency)?
4. RFC required (per RFC-first rule)? Likely yes — endpoint shape changes.

## Suggested first step

Draft `docs/rfcs/00X-inference-formats.md` covering routing decision + Avro schema-walk approach. Get sign-off, then implement Avro first (simplest schema model).
