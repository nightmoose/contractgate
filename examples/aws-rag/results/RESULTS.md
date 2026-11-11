# P3 eval results (2026-11-11, us-east-1)

Run by Chief (executor), Wednesday Nov 11, 2026, 6:30–6:38 AM ET.
Corpus: 117 files (README, reference docs, production runbook, catalog seeds), about 551 KB / 137k tokens, ingested into both Knowledge Bases. 117/117 documents indexed on each. No S3 docs bucket was required; custom document ingest worked from the box.

## Headline numbers (60 questions, 58 answerable + 2 abstain)

| Slice | hit@1 | hit@3 | hit@5 | Judge correct | Hallucinated | Grounding score (mean) | Generate p50 / p95 |
|---|---|---|---|---|---|---|---|
| All (n=240) | 69.0% | 87.9% | 91.4% | 90.0% | 2.9% | 0.84 | 1.12 s / 1.77 s |
| Fixed chunking | 70.7% | 87.9% | 91.4% | 93.3% | 0.8% | 0.88 | 1.12 s / 1.77 s |
| Semantic chunking | 67.2% | 87.9% | 91.4% | 86.7% | 5.0% | 0.81 | 1.12 s / 1.65 s |
| Nova Lite | 69.0% | 87.9% | 91.4% | 90.0% | 3.3% | 0.82 | 1.14 s / 1.90 s |
| Nova Pro | 69.0% | 87.9% | 91.4% | 90.0% | 2.5% | 0.87 | 1.10 s / 1.56 s |
| **Fixed + Nova Pro** | 70.7% | 87.9% | 91.4% | **95.0%** | **0.0%** | 0.89 | 1.12 s / 1.67 s |
| Semantic + Nova Pro | 67.2% | 87.9% | 91.4% | 85.0% | 5.0% | 0.85 | 1.09 s / 1.48 s |
| Abstain (n=8) | — | — | — | 100% | 0% | 0.93 | 0.98 s / 1.24 s |

Retrieve latency was 0.32 s p50 and 0.39 s p95, similar on both indexes. Guardrails contextual grounding intervened on 19% of answers (14% fixed, 24% semantic) at a 0.75 threshold; mean relevance was 0.99. The two abstain questions (SOC 2 report number; Azure Event Hubs / Pub/Sub ingress flags) were refused on every kb/model pair.

## What the comparison shows

Fixed-size chunking (512 tokens, 20% overlap) beat semantic chunking on this corpus for answer quality, even though hit@5 was the same (91.4%). Fixed + Nova Pro was the best cell: 57/60 judged correct and no judged hallucinations. Semantic retrieval missed the stripe-charges id-pattern question (Q53) and more often produced answers the judge marked wrong. Nova Lite and Nova Pro had the same overall correct rate (90%); Pro was slightly cleaner on hallucinations and a bit faster at p95, at higher token cost.

hit@5 misses on the answerable set were Q05, Q07, Q08, Q13, and Q16 (and Q53 on semantic only). Most of those facts live in markdown tables (events/sec, plan allowances, dashboard stack, version states), which a 512-token window can split.

## Spend

About **$0.25 estimated**, under the $25 budget. Breakdown: Nova Pro $0.090, Guardrails grounding $0.127, Nova Lite (generation + judge) $0.025, Titan V2 embeddings (both KBs) $0.005, S3 Vectors PUTs/storage/queries about $0.006. This is a self-managed Knowledge Base on S3 Vectors, so the managed "Standard Retrieve" fee ($1 / 1k calls) does not apply. Cost Explorer lags about 24 hours.

## Left running

Nothing in this stack bills by the hour. Left in place because storage is tiny (1,085 vectors: 408 fixed + 677 semantic, roughly 11 MB):

- S3 Vectors bucket `nmb-rag-vectors` (AES256), indexes `fixed` and `semantic`
- Knowledge Bases `nmb-rag-fixed` (`5G4UMSCHFY`) and `nmb-rag-semantic` (`B6DBQKPPN2`)
- Custom data sources `6XP9ZIYBAF` and `QCYUADTUYH`
- Guardrail `nmb-rag-grounding` (`zsbv4nv1picq` v1), no idle charge

Tear down with `make destroy` in this directory. That does not delete the `nmb-bedrock-kb` role.

## Artifacts

- `eval/golden.jsonl` — the 60-question labeled set
- `results/eval-summary.json` — aggregates and cost
- `results/eval-results.jsonl` — one row per question × kb × model
- `results/ingest-status.json` — 117/117 indexed on both KBs
