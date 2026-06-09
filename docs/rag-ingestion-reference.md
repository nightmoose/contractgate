# RAG / LLM Ingestion Reference (RFC-077)

ContractGate gates documents **before** they reach your RAG or fine-tuning
pipeline — before chunking, before embedding, before they land in a vector
store or training set. It enforces the *structured envelope* around each
record: provenance, freshness, an explicit PII-redaction attestation, and a
source allowlist.

## What ContractGate does / does not do

This boundary is the whole point — keep it sharp:

**Does (the gate):**

- Enforces that every record carries a declared provenance envelope.
- Rejects documents whose source is not on the allowlist.
- Rejects stale documents (freshness window).
- Rejects records that do not attest PII was redacted (`pii_redacted: true`).
- Dedupes within an ingest batch.
- Emits a structured, auditable pass/quarantine outcome per record.

**Does not (stays in your pipeline):**

- Parse PDFs/HTML/Confluence — records arrive as already-extracted JSON.
- Chunk text or choose a chunking strategy.
- Compute embeddings or know which embedding model you use.
- Score content for toxicity, readability, or "semantic coherence."
- Detect PII. `pii_redacted` is an *attestation* the caller makes; the gate
  enforces that the attestation is present and true, not that it is correct.

If you need content scoring or PII *detection*, that runs upstream in your
pipeline; ContractGate enforces the contract on the envelope it produces.

## The `_cg` envelope convention

A RAG record is your payload plus a `_cg` envelope object:

```json
{
  "text": "Quarterly revenue grew 12% QoQ.",
  "_cg": {
    "source": "confluence",
    "doc_id": "conf:page-12345",
    "ingested_at": 1781013878,
    "pii_redacted": true
  }
}
```

The envelope is declared in the contract as a **nested `object` entity** — not
as flat dotted field names. Ontology validation descends into the object by
key; quality rules (freshness, uniqueness) address the same fields by
dot-notation path.

```yaml
ontology:
  entities:
    - name: _cg
      type: object
      required: true
      properties:
        - name: source
          type: string
          required: true
          enum: ["confluence", "gdrive", "s3-curated", "support-tickets"]
        - name: pii_redacted
          type: boolean
          required: true
          enum: [true]          # attestation must be explicit true

quality:
  - field: _cg.ingested_at      # dot-path resolves into the nested object
    type: freshness
    max_age_seconds: 2592000
```

The `enum: [true]` on `pii_redacted` is the attestation gate: a record with
`pii_redacted: false` (or any non-`true` value) is rejected with an
`EnumViolation`.

## Example contracts

Runnable, and exercised by `src/rag_contract_tests.rs`:

- [`examples/contracts/rag/rag_corpus_ingest.yaml`](../examples/contracts/rag/rag_corpus_ingest.yaml)
  — RAG corpus documents.
- [`examples/contracts/rag/fine_tuning_corpus.yaml`](../examples/contracts/rag/fine_tuning_corpus.yaml)
  — prompt/completion pairs with a `license_ok` attestation.

## Wiring it into a pipeline

Validate each record at ingest before chunking. Conceptually (LangChain /
LlamaIndex or any pipeline):

```python
for record in raw_records:
    result = contractgate_validate(record)   # POST /v1/ingest/{contract_id}
    if not result["passed"]:
        quarantine(record, result["violations"])
        continue
    pipeline.chunk_embed_and_index(record)    # your existing steps, unchanged
```

Failing records are quarantined with their violations rather than silently
poisoning the vector store. See the
[POST /v1/ingest reference](v1-ingest-reference.md) for the endpoint contract.

## Known limitation: envelope strictness

`compliance_mode: true` rejects undeclared keys at the **top level** only; it
does not currently police undeclared keys *inside* `_cg`. Required-field and
`enum` attestations inside the envelope are still enforced. Strict
nested-envelope rejection is a possible future engine change (tracked in
RFC-077's open questions), not a current guarantee — don't rely on it.
