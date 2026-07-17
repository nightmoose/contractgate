# RAG Corpus Walkthrough

Gate documents **before** they reach chunking and embedding: ContractGate
enforces the provenance envelope (source, freshness, PII attestation); your
pipeline still does the chunking and embedding. Full detail in the
[RAG ingestion reference](../rag-ingestion-reference.md).

## 1. The contract

The full runnable file is
[`examples/contracts/rag/rag_corpus_ingest.yaml`](../../examples/contracts/rag/rag_corpus_ingest.yaml).
The envelope is a nested `_cg` object; `pii_redacted` must attest `true`.

```yaml
version: "1.0"
name: "rag_corpus_ingest"
compliance_mode: true
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
        - name: doc_id
          type: string
          required: true
          pattern: "^[a-zA-Z0-9_:-]+$"
        - name: ingested_at
          type: integer
          required: true
        - name: pii_redacted
          type: boolean
          required: true
          enum: [true]
    - name: text
      type: string
      required: true
      min_length: 1
quality:
  - field: _cg.ingested_at
    type: freshness
    max_age_seconds: 2592000
  - field: _cg.doc_id
    type: uniqueness
    scope: batch
```

## 2. The command

Validate a data file locally — no server, no network
([`cg test` reference](../cg-test-reference.md)):

```
cg test --contract examples/contracts/rag/rag_corpus_ingest.yaml --data corpus.ndjson
```

## 3. A passing record

> ⏱ **Freshness:** this contract's `freshness` rule rejects documents older than 30 days,
> so the passing record's `_cg.ingested_at` must be recent. The committed examples
> (`examples/contracts/rag/pass.json`, `fail.json`) are kept current by
> `scripts/refresh_example_freshness.py` — run it before validating.

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

```
contract: rag_corpus_ingest (v1.0)
  PASS  1
1/1 records passed   validated in 0.1ms
```

## 4. A failing record

A document that didn't clear PII redaction. `pii_redacted: false` does not
satisfy the `enum: [true]` attestation, so the gate blocks it:

```json
{
  "text": "Contact john@example.com for details.",
  "_cg": {
    "source": "confluence",
    "doc_id": "conf:page-12345",
    "ingested_at": 1781013878,
    "pii_redacted": false
  }
}
```

```
  FAIL  1
record   0  _cg.pii_redacted   enum_violation   Field '_cg.pii_redacted' value false not in allowed set: [true]
1/1 records failed (100%)
```

Two other failures the same contract catches: a `source` outside the allowlist
(`enum_violation` on `_cg.source`) and an `ingested_at` older than 30 days
(`freshness_violation`).

## 5. Wire it in

Validate each record before chunking; quarantine failures instead of embedding
them. Against the live endpoint
([POST /v1/ingest reference](../v1-ingest-reference.md)):

```python
for record in raw_records:
    result = contractgate_validate(record)   # POST /v1/ingest/{contract_id}
    if not result["passed"]:
        quarantine(record, result["violations"])
        continue
    pipeline.chunk_embed_and_index(record)    # your existing steps, unchanged
```

In CI, gate a corpus file before a rebuild — `cg test` exits non-zero if any
record fails:

```
cg test --contract rag_corpus_ingest.yaml --data corpus.ndjson --quiet || exit 1
```
