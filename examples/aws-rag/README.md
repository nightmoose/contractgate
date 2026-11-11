# RAG over ContractGate docs (Bedrock Knowledge Bases + S3 Vectors)

Project 3 of the AWS learning plan. An "ask the docs" assistant over this
repository's README, reference docs, production runbook, and public-catalog
seed contracts. Retrieval uses Amazon Titan Text Embeddings V2 and Amazon S3
Vectors. Generation and the LLM judge stay on Amazon Nova Lite and Nova Pro
(no Marketplace models).

Account `065614567889`, region `us-east-1`. Budget cap $25. This stack does
not touch ContractGate production, Fly, or application runtime config.

## What it creates

Two Knowledge Bases share one S3 Vectors bucket and use the existing
`nmb-bedrock-kb` role (path `/nmb/`, `nmb-boundary` attached, created in P1):

| Knowledge Base | Chunking | Index |
|---|---|---|
| `nmb-rag-fixed` | fixed size, 512 tokens, 20% overlap | `fixed` |
| `nmb-rag-semantic` | semantic, 300 tokens, breakpoint 95, buffer 0 | `semantic` |

Documents are ingested through the Bedrock **custom document** API
(`IngestKnowledgeBaseDocuments`), not from an S3 bucket. The bot box's VPC
endpoint policy denies `s3:CreateBucket` and `s3:ListAllMyBuckets`, so a docs
bucket cannot be bootstrapped from here. Custom ingest talks only to
`bedrock-agent`, which works from the box. A workflow-dispatch job in
`.github/workflows/aws-rag.yml` can still create `nmb-rag-docs-065614567889`
from GitHub Actions (OIDC) if an S3 data source is wanted later.

A Bedrock Guardrail (`nmb-rag-grounding`) with contextual grounding and
relevance filters is used by the eval harness. It has no idle charge. The
Lexsi prompt-injection guardrail is not reused.

Terraform state is **local** under `terraform/terraform.tfstate` (gitignored)
and is a **separate root** from `infra/aws/`. Do not apply this stack into
the P1 state file.

## Ask the docs

After `make apply` and `make ingest`:

```bash
cd examples/aws-rag
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt

python3 scripts/ask.py "What does make demo do?"
python3 scripts/ask.py --kb semantic --model pro "How does quarantine replay work?"

# Local HTTP endpoint (not deployed; no hourly cost)
python3 scripts/serve.py
# curl -s localhost:8787/ask -H 'content-type: application/json' \
#   -d '{"question":"What is GET /health for?","kb":"fixed","model":"lite"}'
```

## Eval

`eval/golden.jsonl` is a 60-question labeled set (58 answerable, 2 should
abstain) with gold answers and source paths. `scripts/eval_rag.py` reports
retrieval hit@1/3/5, Guardrails grounding/relevance, a Nova Lite judge,
p50/p95 latency, and an estimated USD cost.

Results from the 2026-11-11 us-east-1 run are in [`results/RESULTS.md`](results/RESULTS.md).
Headline: hit@5 91.4%, judge correct 90% overall and 95% for fixed chunking +
Nova Pro (0 judged hallucinations). Retrieve p50 0.32 s, generate p50 1.12 s.
Estimated spend about $0.25. Fixed-size chunking beat semantic chunking on
answer quality for this corpus.


```bash
make eval
# results/eval-summary.json and results/eval-results.jsonl
```

## Apply and teardown

On the box, `nightmoose-bots` keys live in the environment. Creating a
Knowledge Base requires `iam:PassRole` on `nmb-bedrock-kb`, which only
`nmb-terraform-deploy` has, so apply goes through `infra/aws/with-deploy-role.sh`
(a subshell; the caller's keys are not overwritten):

```bash
export PATH=$HOME/.local/bin:$PATH AWS_DEFAULT_REGION=us-east-1
cd examples/aws-rag
make fmt
make init
make validate
make plan
make apply
make ingest
make eval
```

Nothing in this stack bills by the hour. S3 Vectors storage for a small
index can stay (pennies). To tear it down: `make destroy`. That removes the
vector bucket, indexes, Knowledge Bases, data sources, and grounding
guardrail. It does not delete `nmb-bedrock-kb`.

## Files

- `terraform/` — separate Terraform root (S3 Vectors, two KBs, custom data sources, grounding guardrail)
- `scripts/collect_corpus.py` — copies README, docs (except RFCs / images / data-room), and catalog seeds
- `scripts/ingest.py` — custom-document ingest into both KBs
- `scripts/ask.py` — CLI
- `scripts/serve.py` — local HTTP `/ask`
- `scripts/eval_rag.py` — labeled eval harness
- `eval/golden.jsonl` — 60 questions
- `results/` — ingest status, eval results, and summary (committed after a run)
