#!/usr/bin/env python3
"""Retrieval and generation eval over eval/golden.jsonl.

For each question, against each Knowledge Base (fixed vs semantic chunking):
  - Retrieve k=5, score hit@1 / hit@3 / hit@5, record latency
  - RetrieveAndGenerate with Nova Lite and Nova Pro, record latency
  - Bedrock Guardrails contextual grounding + relevance on the answer
  - Nova Lite LLM judge vs the gold answer

Writes results/eval-results.jsonl (one row per kb/model/question) and
results/eval-summary.json (aggregates, latency percentiles, estimated cost).
"""
from __future__ import annotations

import argparse
import json
import math
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from statistics import median

import boto3
from botocore.exceptions import ClientError

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import ask as ask_mod  # noqa: E402
from tf_outputs import load as load_outputs  # noqa: E402

# us-east-1 on-demand list prices checked 2026-11-11 (public pricing).
# Estimates only; Cost Explorer lags ~24h.
PRICE = {
    "titan_embed_v2_per_m_tokens": 0.02,
    "nova_lite_in_per_m": 0.06,
    "nova_lite_out_per_m": 0.24,
    "nova_pro_in_per_m": 0.80,
    "nova_pro_out_per_m": 3.20,
    "kb_retrieve_per_query": 0.002,  # Bedrock Knowledge Base Retrieve (per query)
    "guardrail_content_per_1k_units": 0.15,
    "s3vectors_query_per_1k": 0.0025,  # order-of-magnitude; storage is extra
}

JUDGE_MODEL = "us.amazon.nova-lite-v1:0"
JUDGE_SYSTEM = (
    "You score a RAG assistant. Reply with JSON only, no markdown, of the form "
    '{"correct":true|false,"partial":true|false,"hallucinated":true|false,"reason":"short"} '
    "correct=true if the answer contains the gold facts (paraphrase allowed) and "
    "does not contradict them. For questions with expected=abstain, correct=true "
    "only if the assistant refuses or says it does not know, and does not invent "
    "a specific fact. hallucinated=true if the answer asserts a fact that is not "
    "in the gold answer and is not supported by the retrieved context."
)


def load_golden(path: Path) -> list[dict]:
    rows = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        rows.append(json.loads(line))
    return rows


def retry_call(fn, retries=6):
    for attempt in range(retries):
        try:
            return fn()
        except ClientError as exc:
            code = exc.response.get("Error", {}).get("Code", "")
            if code in {"ThrottlingException", "ServiceQuotaExceededException", "TooManyRequestsException"} and attempt < retries - 1:
                time.sleep(min(30, 2 ** attempt))
                continue
            raise


def hit_at_k(chunks: list[dict], gold_sources: list[str], k: int) -> bool:
    if not gold_sources:
        return False
    for ch in chunks[:k]:
        blob = " ".join((ch.get("sources") or []) + [ch.get("text") or ""])
        for src in gold_sources:
            if src and src in blob:
                return True
    return False


def approx_tokens(text: str) -> int:
    return max(1, math.ceil(len(text or "") / 4))


def retrieve_row(runtime, kb_id: str, question: str, k: int) -> dict:
    resp = retry_call(lambda: ask_mod.retrieve(runtime, kb_id, question, k=k))
    chunks = []
    for item in resp.get("retrievalResults", []):
        chunks.append(
            {
                "score": item.get("score"),
                "sources": ask_mod.result_sources(item),
                "text": (item.get("content") or {}).get("text", ""),
            }
        )
    return {"chunks": chunks, "latency_s": resp["_latency_s"]}


def generate_row(runtime, kb_id: str, question: str, model_id: str, k: int) -> dict:
    resp = retry_call(lambda: ask_mod.generate(runtime, kb_id, question, model_id, k=k))
    citations = []
    context_parts = []
    for cite in resp.get("citations") or []:
        for ref in cite.get("retrievedReferences") or []:
            text = (ref.get("content") or {}).get("text", "")
            context_parts.append(text)
            citations.append({"sources": ask_mod.result_sources(ref), "text": text})
    answer = (resp.get("output") or {}).get("text", "")
    usage = resp.get("usage") or {}
    return {
        "answer": answer,
        "latency_s": resp["_latency_s"],
        "citations": citations,
        "context": "\n\n".join(context_parts),
        "input_tokens": usage.get("inputTokens") or usage.get("input_tokens"),
        "output_tokens": usage.get("outputTokens") or usage.get("output_tokens"),
    }


def apply_grounding(runtime_bedrock, guardrail_id: str, version: str, question: str, context: str, answer: str) -> dict:
    if not context or not answer:
        return {"error": "missing context or answer", "grounding_score": None, "relevance_score": None, "intervened": False}

    def _call():
        return runtime_bedrock.apply_guardrail(
            guardrailIdentifier=guardrail_id,
            guardrailVersion=str(version),
            source="OUTPUT",
            content=[
                {"text": {"text": context[:20000], "qualifiers": ["grounding_source"]}},
                {"text": {"text": question, "qualifiers": ["query"]}},
                {"text": {"text": answer}},
            ],
        )

    resp = retry_call(_call)
    scores = {"grounding_score": None, "relevance_score": None, "intervened": resp.get("action") == "GUARDRAIL_INTERVENED"}
    for assessment in resp.get("assessments") or []:
        policy = assessment.get("contextualGroundingPolicy") or {}
        for filt in policy.get("filters") or []:
            typ = (filt.get("type") or "").upper()
            score = filt.get("score")
            if typ == "GROUNDING":
                scores["grounding_score"] = score
            elif typ == "RELEVANCE":
                scores["relevance_score"] = score
    scores["usage"] = resp.get("usage") or {}
    return scores


def converse_judge(bedrock, question: str, gold: str, expected: str, answer: str, context: str) -> dict:
    user = (
        f"Question: {question}\n"
        f"Expected behavior: {expected}\n"
        f"Gold answer: {gold}\n"
        f"Assistant answer: {answer}\n"
        f"Retrieved context (trimmed):\n{context[:6000]}\n"
    )

    def _call():
        return bedrock.converse(
            modelId=JUDGE_MODEL,
            messages=[{"role": "user", "content": [{"text": user}]}],
            system=[{"text": JUDGE_SYSTEM}],
            inferenceConfig={"maxTokens": 250, "temperature": 0},
        )

    resp = retry_call(_call)
    text = resp["output"]["message"]["content"][0]["text"]
    usage = resp.get("usage") or {}
    start = text.find("{")
    end = text.rfind("}")
    parsed = {}
    if start >= 0 and end > start:
        try:
            parsed = json.loads(text[start : end + 1])
        except json.JSONDecodeError:
            parsed = {"parse_error": True, "raw": text}
    else:
        parsed = {"parse_error": True, "raw": text}
    parsed["input_tokens"] = usage.get("inputTokens")
    parsed["output_tokens"] = usage.get("outputTokens")
    return parsed


def percentile(values: list[float], p: float) -> float | None:
    if not values:
        return None
    xs = sorted(values)
    if len(xs) == 1:
        return xs[0]
    k = (len(xs) - 1) * p
    f = math.floor(k)
    c = math.ceil(k)
    if f == c:
        return xs[int(k)]
    return xs[f] * (c - k) + xs[c] * (k - f)


def already_done(path: Path) -> set[str]:
    keys = set()
    if not path.exists():
        return keys
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        keys.add(f"{row['id']}|{row['kb']}|{row['model']}")
    return keys


def estimate_cost(summary_counts: dict, embed_tokens: int) -> dict:
    lite_in = summary_counts.get("lite_in", 0)
    lite_out = summary_counts.get("lite_out", 0)
    pro_in = summary_counts.get("pro_in", 0)
    pro_out = summary_counts.get("pro_out", 0)
    retrieves = summary_counts.get("retrieves", 0)
    ground_units = summary_counts.get("guardrail_units", 0)
    embed = (embed_tokens / 1_000_000) * PRICE["titan_embed_v2_per_m_tokens"]
    lite = (lite_in / 1e6) * PRICE["nova_lite_in_per_m"] + (lite_out / 1e6) * PRICE["nova_lite_out_per_m"]
    pro = (pro_in / 1e6) * PRICE["nova_pro_in_per_m"] + (pro_out / 1e6) * PRICE["nova_pro_out_per_m"]
    retrieve = retrieves * PRICE["kb_retrieve_per_query"]
    # RetrieveAndGenerate also retrieves; count those too (already in retrieves if we add them)
    guard = (ground_units / 1000) * PRICE["guardrail_content_per_1k_units"]
    s3v = (retrieves / 1000) * PRICE["s3vectors_query_per_1k"]
    parts = {
        "embeddings_two_kbs": round(embed, 5),
        "nova_lite": round(lite, 5),
        "nova_pro": round(pro, 5),
        "kb_retrieve": round(retrieve, 5),
        "guardrails": round(guard, 5),
        "s3_vectors_queries": round(s3v, 5),
    }
    parts["total_estimated"] = round(sum(parts.values()), 5)
    parts["notes"] = (
        "Embeddings counted once per KB ingest (approx tokens from corpus). "
        "Generate input/output tokens are observed when the API returns them, "
        "otherwise approximated from text length / 4. Cost Explorer lags ~24h."
    )
    return parts


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default="us-east-1")
    parser.add_argument("--k", type=int, default=5)
    parser.add_argument("--golden", default=str(ROOT / "eval" / "golden.jsonl"))
    parser.add_argument("--out", default=str(ROOT / "results" / "eval-results.jsonl"))
    parser.add_argument("--limit", type=int, default=0, help="Optional cap for a smoke run")
    parser.add_argument("--workers", type=int, default=2)
    args = parser.parse_args()

    outputs = load_outputs()
    kbs = outputs["knowledge_bases"]
    guard_id = outputs["guardrail_id"]
    guard_ver = str(outputs["guardrail_version"])
    questions = load_golden(Path(args.golden))
    if args.limit:
        questions = questions[: args.limit]

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    done = already_done(out_path)

    runtime = boto3.client("bedrock-agent-runtime", region_name=args.region)
    bedrock_rt = boto3.client("bedrock-runtime", region_name=args.region)

    # Cache retrieves per (question, kb)
    retrieve_cache: dict[tuple[str, str], dict] = {}

    jobs = []
    for q in questions:
        for kb_name in kbs:
            for model_name, model_id in ask_mod.MODELS.items():
                key = f"{q['id']}|{kb_name}|{model_name}"
                if key not in done:
                    jobs.append((q, kb_name, model_name, model_id, key))

    print(f"{len(questions)} questions, {len(jobs)} remaining generate jobs, {len(done)} already done")

    def run_one(job):
        q, kb_name, model_name, model_id, key = job
        kb_id = kbs[kb_name]["knowledge_base_id"]
        cache_key = (q["id"], kb_name)
        if cache_key not in retrieve_cache:
            retrieve_cache[cache_key] = retrieve_row(runtime, kb_id, q["question"], args.k)
        ret = retrieve_cache[cache_key]
        chunks = ret["chunks"]
        gold_sources = q.get("source_docs") or []
        gen = generate_row(runtime, kb_id, q["question"], model_id, args.k)
        context = gen["context"] or "\n\n".join(ch["text"] for ch in chunks)
        grounding = apply_grounding(
            bedrock_rt, guard_id, guard_ver, q["question"], context, gen["answer"]
        )
        judge = converse_judge(
            bedrock_rt,
            q["question"],
            q.get("gold_answer") or "",
            q.get("expected") or "answer",
            gen["answer"],
            context,
        )
        in_tok = gen["input_tokens"] or (approx_tokens(q["question"]) + approx_tokens(context) + 80)
        out_tok = gen["output_tokens"] or approx_tokens(gen["answer"])
        row = {
            "id": q["id"],
            "question": q["question"],
            "expected": q.get("expected") or "answer",
            "category": q.get("category"),
            "kb": kb_name,
            "model": model_name,
            "model_id": model_id,
            "gold_sources": gold_sources,
            "hit@1": hit_at_k(chunks, gold_sources, 1),
            "hit@3": hit_at_k(chunks, gold_sources, 3),
            "hit@5": hit_at_k(chunks, gold_sources, 5),
            "retrieve_latency_s": round(ret["latency_s"], 4),
            "generate_latency_s": round(gen["latency_s"], 4),
            "retrieved_sources": [ch["sources"] for ch in chunks],
            "answer": gen["answer"],
            "input_tokens": in_tok,
            "output_tokens": out_tok,
            "grounding": grounding,
            "judge": judge,
        }
        return key, row

    # Keep retrieve cache fills single-threaded per kb/question by running sequentially
    # for retrieve, then generate with a small pool. Simplest: sequential with 1-2 workers
    # but protect the cache with a simple pre-pass.
    for q in questions:
        for kb_name in kbs:
            cache_key = (q["id"], kb_name)
            if cache_key in retrieve_cache:
                continue
            # only if some remaining job needs it
            if any(j[0]["id"] == q["id"] and j[1] == kb_name for j in jobs):
                kb_id = kbs[kb_name]["knowledge_base_id"]
                print(f"retrieve {q['id']} {kb_name} ...", flush=True)
                retrieve_cache[cache_key] = retrieve_row(runtime, kb_id, q["question"], args.k)

    with out_path.open("a") as fh:
        if args.workers <= 1:
            for job in jobs:
                key, row = run_one(job)
                fh.write(json.dumps(row) + "\n")
                fh.flush()
                print(
                    f"{row['id']} {row['kb']}/{row['model']} hit@5={row['hit@5']} "
                    f"correct={row['judge'].get('correct')} gen={row['generate_latency_s']}s",
                    flush=True,
                )
        else:
            with ThreadPoolExecutor(max_workers=args.workers) as pool:
                futs = [pool.submit(run_one, job) for job in jobs]
                for fut in as_completed(futs):
                    key, row = fut.result()
                    fh.write(json.dumps(row) + "\n")
                    fh.flush()
                    print(
                        f"{row['id']} {row['kb']}/{row['model']} hit@5={row['hit@5']} "
                        f"correct={row['judge'].get('correct')} gen={row['generate_latency_s']}s",
                        flush=True,
                    )

    # Reload all rows for the summary (includes prior runs)
    rows = []
    for line in out_path.read_text().splitlines():
        if line.strip():
            rows.append(json.loads(line))

    def agg(predicate):
        subset = [r for r in rows if predicate(r)]
        n = len(subset)
        if n == 0:
            return {"n": 0}
        answerable = [r for r in subset if r.get("expected") != "abstain"]
        an = len(answerable) or 1
        gen_lat = [r["generate_latency_s"] for r in subset]
        ret_lat = [r["retrieve_latency_s"] for r in subset]
        gscores = [r["grounding"].get("grounding_score") for r in subset if r.get("grounding", {}).get("grounding_score") is not None]
        rscores = [r["grounding"].get("relevance_score") for r in subset if r.get("grounding", {}).get("relevance_score") is not None]
        return {
            "n": n,
            "n_answerable": len(answerable),
            "hit@1": round(sum(1 for r in answerable if r["hit@1"]) / an, 4),
            "hit@3": round(sum(1 for r in answerable if r["hit@3"]) / an, 4),
            "hit@5": round(sum(1 for r in answerable if r["hit@5"]) / an, 4),
            "judge_correct": round(sum(1 for r in subset if r.get("judge", {}).get("correct") is True) / n, 4),
            "judge_hallucinated": round(sum(1 for r in subset if r.get("judge", {}).get("hallucinated") is True) / n, 4),
            "grounding_intervened": round(sum(1 for r in subset if r.get("grounding", {}).get("intervened")) / n, 4),
            "grounding_score_mean": round(sum(gscores) / len(gscores), 4) if gscores else None,
            "relevance_score_mean": round(sum(rscores) / len(rscores), 4) if rscores else None,
            "retrieve_latency_s": {
                "p50": round(percentile(ret_lat, 0.5) or 0, 4),
                "p95": round(percentile(ret_lat, 0.95) or 0, 4),
            },
            "generate_latency_s": {
                "p50": round(percentile(gen_lat, 0.5) or 0, 4),
                "p95": round(percentile(gen_lat, 0.95) or 0, 4),
            },
        }

    slices = {
        "all": agg(lambda r: True),
        "kb_fixed": agg(lambda r: r["kb"] == "fixed"),
        "kb_semantic": agg(lambda r: r["kb"] == "semantic"),
        "model_lite": agg(lambda r: r["model"] == "lite"),
        "model_pro": agg(lambda r: r["model"] == "pro"),
        "fixed_lite": agg(lambda r: r["kb"] == "fixed" and r["model"] == "lite"),
        "fixed_pro": agg(lambda r: r["kb"] == "fixed" and r["model"] == "pro"),
        "semantic_lite": agg(lambda r: r["kb"] == "semantic" and r["model"] == "lite"),
        "semantic_pro": agg(lambda r: r["kb"] == "semantic" and r["model"] == "pro"),
        "abstain": agg(lambda r: r.get("expected") == "abstain"),
    }

    counts = {"lite_in": 0, "lite_out": 0, "pro_in": 0, "pro_out": 0, "retrieves": 0, "guardrail_units": 0}
    seen_retrieves = set()
    for r in rows:
        if r["model"] == "lite":
            counts["lite_in"] += int(r.get("input_tokens") or 0)
            counts["lite_out"] += int(r.get("output_tokens") or 0)
        else:
            counts["pro_in"] += int(r.get("input_tokens") or 0)
            counts["pro_out"] += int(r.get("output_tokens") or 0)
        counts["lite_in"] += int((r.get("judge") or {}).get("input_tokens") or 0)
        counts["lite_out"] += int((r.get("judge") or {}).get("output_tokens") or 0)
        rk = (r["id"], r["kb"])
        if rk not in seen_retrieves:
            seen_retrieves.add(rk)
            counts["retrieves"] += 1
        counts["retrieves"] += 1  # each RetrieveAndGenerate also retrieves
        usage = (r.get("grounding") or {}).get("usage") or {}
        counts["guardrail_units"] += int(usage.get("contentPolicyUnits") or usage.get("contextualGroundingPolicyUnits") or 1)

    # Embedding tokens: 2 KBs * corpus approx, if manifest exists
    embed_tokens = 0
    manifest = ROOT / "corpus" / "build" / "manifest.json"
    if manifest.exists():
        items = json.loads(manifest.read_text())
        embed_tokens = 2 * sum(int(x.get("approx_tokens") or 0) for x in items)

    cost = estimate_cost(counts, embed_tokens)
    summary = {
        "n_rows": len(rows),
        "n_questions": len({r["id"] for r in rows}),
        "slices": slices,
        "token_counts": counts,
        "embed_tokens_est": embed_tokens,
        "cost_usd_estimated": cost,
        "guardrail_id": guard_id,
        "guardrail_version": guard_ver,
        "kbs": {k: {"knowledge_base_id": v["knowledge_base_id"]} for k, v in kbs.items()},
    }
    summary_path = ROOT / "results" / "eval-summary.json"
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    print(f"Wrote {out_path} and {summary_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
