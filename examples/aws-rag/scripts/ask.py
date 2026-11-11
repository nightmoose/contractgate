#!/usr/bin/env python3
"""Ask a ContractGate Knowledge Base (Retrieve + Nova generation).

Usage:
  python scripts/ask.py "What does make demo do?"
  python scripts/ask.py --kb semantic --model pro "How does quarantine replay work?"
  python scripts/ask.py --retrieve-only "stripe-charges id pattern"
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import boto3

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from tf_outputs import load as load_outputs  # noqa: E402

MODELS = {
    "lite": "us.amazon.nova-lite-v1:0",
    "pro": "us.amazon.nova-pro-v1:0",
}

SYSTEM = (
    "You are a DataContractGate documentation assistant. Answer only from the "
    "retrieved context. If the context is insufficient, say you do not know. "
    "Cite source paths when you can. Do not invent APIs, prices, or limits."
)


def retrieve(runtime, kb_id: str, question: str, k: int = 5) -> dict:
    t0 = time.perf_counter()
    resp = runtime.retrieve(
        knowledgeBaseId=kb_id,
        retrievalQuery={"text": question},
        retrievalConfiguration={"vectorSearchConfiguration": {"numberOfResults": k}},
    )
    resp["_latency_s"] = time.perf_counter() - t0
    return resp


def generate(runtime, kb_id: str, question: str, model_id: str, k: int = 5) -> dict:
    t0 = time.perf_counter()
    resp = runtime.retrieve_and_generate(
        input={"text": question},
        retrieveAndGenerateConfiguration={
            "type": "KNOWLEDGE_BASE",
            "knowledgeBaseConfiguration": {
                "knowledgeBaseId": kb_id,
                "modelArn": model_id,
                "retrievalConfiguration": {
                    "vectorSearchConfiguration": {"numberOfResults": k}
                },
                "generationConfiguration": {
                    "inferenceConfig": {
                        "textInferenceConfig": {
                            "maxTokens": 512,
                            "temperature": 0,
                            "topP": 0.9,
                        }
                    },
                    "promptTemplate": {
                        "textPromptTemplate": (
                            SYSTEM
                            + "\n\nContext:\n$search_results$\n\nQuestion:\n$query$\n\nAnswer:"
                        )
                    },
                },
            },
        },
    )
    resp["_latency_s"] = time.perf_counter() - t0
    return resp


def result_sources(item: dict) -> list[str]:
    sources = []
    loc = item.get("location") or {}
    custom = (loc.get("customDocumentLocation") or {}).get("id")
    if custom:
        sources.append(custom)
    s3 = (loc.get("s3Location") or {}).get("uri")
    if s3:
        sources.append(s3)
    meta = item.get("metadata") or {}
    for key in ("source_path", "x-amz-bedrock-kb-source-uri", "x-amz-bedrock-kb-data-source-id"):
        val = meta.get(key)
        if val:
            sources.append(str(val))
    return sources


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("question")
    parser.add_argument("--region", default="us-east-1")
    parser.add_argument("--kb", choices=["fixed", "semantic"], default="fixed")
    parser.add_argument("--model", choices=list(MODELS), default="lite")
    parser.add_argument("--k", type=int, default=5)
    parser.add_argument("--retrieve-only", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    outputs = load_outputs()
    kb_id = outputs["knowledge_bases"][args.kb]["knowledge_base_id"]
    runtime = boto3.client("bedrock-agent-runtime", region_name=args.region)

    retrieved = retrieve(runtime, kb_id, args.question, k=args.k)
    chunks = []
    for item in retrieved.get("retrievalResults", []):
        chunks.append(
            {
                "score": item.get("score"),
                "sources": result_sources(item),
                "text": (item.get("content") or {}).get("text", "")[:800],
            }
        )

    payload = {
        "question": args.question,
        "kb": args.kb,
        "kb_id": kb_id,
        "retrieve_latency_s": round(retrieved["_latency_s"], 3),
        "chunks": chunks,
    }

    if not args.retrieve_only:
        gen = generate(runtime, kb_id, args.question, MODELS[args.model], k=args.k)
        citations = []
        for cite in (gen.get("citations") or []):
            for ref in cite.get("retrievedReferences") or []:
                citations.append(
                    {
                        "sources": result_sources(ref),
                        "text": (ref.get("content") or {}).get("text", "")[:400],
                    }
                )
        payload.update(
            {
                "model": MODELS[args.model],
                "answer": (gen.get("output") or {}).get("text", ""),
                "generate_latency_s": round(gen["_latency_s"], 3),
                "citations": citations,
            }
        )

    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(f"kb={args.kb} retrieve={payload['retrieve_latency_s']}s")
        if "answer" in payload:
            print(f"model={payload['model']} generate={payload['generate_latency_s']}s")
            print("\n" + payload["answer"] + "\n")
        print("Sources:")
        for ch in chunks:
            print(f"  {ch['score']:.4f}  {ch['sources']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
