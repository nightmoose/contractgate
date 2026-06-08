# RFC-077 — RAG-ingestion contract profile

**Status:** Draft
**Date:** 2026-06-08
**Branch:** TBD
**Addresses:** Positioning ContractGate as the data-contract layer upstream of
RAG / LLM training pipelines (see "Data Engineering for the LLM Age",
KDnuggets 2026-03).
**Severity:** P3 — net-new feature, no existing-behavior risk

---

## Problem

LLM/RAG pipelines (ingestion → chunking → embedding → vector store) are
garbage-in-garbage-out: unredacted PII, unattributed sources, and stale
documents poison vector stores and training corpora with no gate in front of
them. The industry framing ("AI-ready data needs the same rigor as tables")
is a real positioning opening for ContractGate.

The tempting-but-wrong response is to make the engine *understand* unstructured
content — PDF/Confluence parsers, chunking-strategy suggestions, toxicity /
readability / "semantic coherence" scoring, embedding-model compatibility,
synthetic-data hooks. All of that is probabilistic, slow, and ML-heavy. It
would blow the <15ms p99 budget (measured 31µs today), break determinism, and
turn a focused gateway into five half-products. **That is explicit non-goal
territory.**

The defensible move is narrower: the data flowing into a RAG pipeline still
has a **structured envelope** — source, lineage, freshness, a PII-redaction
attestation. Validating *that envelope* is exactly what the engine already
does. We enforce the contract on the metadata; the customer's
LangChain/LlamaIndex pipeline keeps doing the chunking and embedding.

## Goal

A first-class **contract profile** for RAG/training ingestion that:

1. requires a provenance + PII-attestation envelope on each record, and
2. reuses the existing quality/ontology/transform primitives to enforce it,

with **zero new logic on the hot path** and **no change to the locked contract
format** — only new *conventions* expressed in the existing fields.

## Non-goals (hard line)

- No unstructured-content parsing in the engine (PDF/HTML/Confluence). Records
  arrive as already-extracted JSON with the payload + envelope.
- No content scoring: toxicity, readability, semantic coherence, embedding
  previews. Non-deterministic; stays in the customer pipeline.
- No vector-DB / embedding-model awareness in the core. Connectors, if ever,
  are a separate RFC and live outside the validation engine.
- No new contract-format fields (the format is locked, see CLAUDE.md). This RFC
  ships a **profile + example contracts + docs**, not a schema change.

## Design

A "RAG-ingestion profile" is a normal contract whose ontology declares the
required envelope fields and whose `quality` rules enforce them. Everything
below already exists in `src/contract.rs`:

| Need | Existing primitive |
|---|---|
| Source must be present & allowlisted | ontology field, `required: true` + `enum: [...]` |
| Lineage / document id present | ontology field `required: true` + `Completeness` rule |
| Freshness (no stale docs) | `QualityRule { type: freshness, max_age_seconds }` |
| PII redaction attested | ontology bool field `required: true` + `enum: [true]` |
| PII actually masked on raw fields | existing RFC-004 `Transform { kind: mask }` |
| Reject undeclared envelope keys | `compliance_mode: true` |
| Per-record outcome / audit | existing `ValidationResult` + audit_log |

The only thing genuinely new is **a documented convention** for what the
envelope looks like, so contracts are portable across customers and the
dashboard can recognize the profile.

### Convention: the `_cg` envelope

A RAG record is `{ ...payload, _cg: { ...envelope } }`. The contract validates
the `_cg.*` paths via dot-notation (already supported by `QualityRule.field`).

```yaml
version: "1.0"
name: "rag_corpus_ingest"
description: "RAG ingestion contract — enforces provenance + PII attestation"
compliance_mode: true          # reject undeclared envelope keys

ontology:
  entities:
    - name: _cg.source
      type: string
      required: true
      enum: ["confluence", "gdrive", "s3-curated", "support-tickets"]
    - name: _cg.doc_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_:-]+$"
    - name: _cg.ingested_at
      type: integer
      required: true
    - name: _cg.pii_redacted
      type: boolean
      required: true
      enum: [true]             # attestation must be explicit true
    - name: text
      type: string
      required: true
      min_length: 1

quality:
  - field: _cg.ingested_at
    type: freshness
    max_age_seconds: 2592000   # 30d — drop stale docs before embedding
  - field: _cg.doc_id
    type: uniqueness
    scope: batch               # dedupe within an ingest batch
  - field: _cg.source
    type: validity
```

No engine code is required to make the above work — it is valid against the
current parser and validator today. (Verification step below confirms this
rather than asserting it.)

### What we actually build

1. **Example contracts** in `examples/contracts/rag/`:
   `rag_corpus_ingest.yaml`, `fine_tuning_corpus.yaml`. Real, parseable,
   exercised by a test.
2. **Profile recognition (optional, dashboard-only):** a contract is "RAG
   profile" if it declares the `_cg.pii_redacted` + `_cg.source` envelope.
   Pure read-side heuristic; **does not touch the validation hot path.**
3. **Docs:** `docs/rag-ingestion-reference.md` — the envelope convention, the
   example contracts, and an explicit "what ContractGate does / does not do"
   boundary (governance, not scoring) so the positioning doesn't overpromise.

### Hot path

Unchanged. These contracts run through the identical ingest validator as every
other contract. No new branches, no new allocations, p99 budget untouched.

## Testing

- A parse+validate test that loads each example contract and asserts: a clean
  record passes; a record with `pii_redacted: false` is rejected; a stale
  `ingested_at` is rejected; an undeclared `_cg.*` key is rejected under
  `compliance_mode`.
- Confirms the claim "no engine change needed" — if any example needs new code,
  that is a finding to fold back into this RFC before it leaves Draft.
- `cargo test` + `cargo check` run by maintainer (cargo unavailable here).

## Rollout

No migration. No API change. No config change. Additive examples + docs +
optional dashboard read-side label. Fully backward compatible.

## Open questions

- Dashboard profile label in this RFC, or split to a follow-up? (Lean: split —
  keep this RFC engine-free and docs-only.)
- Standardize the envelope key as `_cg` vs a customer-chosen prefix? (Lean:
  `_cg` default, overridable, documented.)
