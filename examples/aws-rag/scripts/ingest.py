#!/usr/bin/env python3
"""Ingest corpus/build into both Knowledge Bases via IngestKnowledgeBaseDocuments.

Uses the CUSTOM data sources created by Terraform. Does not call S3.
Batch size is 10 (API max). Polls until every document is INDEXED or FAILED.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import boto3
from botocore.exceptions import ClientError

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from collect_corpus import OUT, collect  # noqa: E402
from tf_outputs import load as load_outputs  # noqa: E402

BATCH = 10
TERMINAL = {"INDEXED", "FAILED", "IGNORED", "NOT_FOUND", "METADATA_UPDATE_FAILED"}
OK = {"INDEXED"}


def documents_for(manifest: list[dict]) -> list[dict]:
    docs = []
    for item in manifest:
        text = (OUT / item["path"]).read_text(errors="replace")
        if not text.strip():
            continue
        docs.append(
            {
                "content": {
                    "dataSourceType": "CUSTOM",
                    "custom": {
                        "customDocumentIdentifier": {"id": item["id"]},
                        "sourceType": "IN_LINE",
                        "inlineContent": {
                            "type": "TEXT",
                            "textContent": {"data": text},
                        },
                    },
                },
                "metadata": {
                    "type": "IN_LINE_ATTRIBUTE",
                    "inlineAttributes": [
                        {
                            "key": "source_path",
                            "value": {"type": "STRING", "stringValue": item["id"]},
                        }
                    ],
                },
            }
        )
    return docs


def ingest_one(client, kb_id: str, ds_id: str, docs: list[dict]) -> None:
    for i in range(0, len(docs), BATCH):
        batch = docs[i : i + BATCH]
        for attempt in range(8):
            try:
                resp = client.ingest_knowledge_base_documents(
                    knowledgeBaseId=kb_id,
                    dataSourceId=ds_id,
                    documents=batch,
                )
                break
            except ClientError as exc:
                code = exc.response.get("Error", {}).get("Code", "")
                if code in {"ThrottlingException", "ServiceQuotaExceededException", "ConflictException"} and attempt < 7:
                    time.sleep(2 ** attempt)
                    continue
                raise
        print(f"  submitted {i + len(batch)}/{len(docs)} (kb={kb_id})")
        # brief pause so we don't stampede the ingest quota
        time.sleep(0.4)


def list_statuses(client, kb_id: str, ds_id: str) -> list[dict]:
    items = []
    token = None
    while True:
        kwargs = {"knowledgeBaseId": kb_id, "dataSourceId": ds_id, "maxResults": 100}
        if token:
            kwargs["nextToken"] = token
        resp = client.list_knowledge_base_documents(**kwargs)
        items.extend(resp.get("documentDetails", []))
        token = resp.get("nextToken")
        if not token:
            break
    return items


def doc_id(detail: dict) -> str:
    ident = detail.get("identifier", {})
    custom = ident.get("custom") or {}
    return custom.get("id") or json.dumps(ident)


def wait_until_done(client, kb_id: str, ds_id: str, expected: int, timeout_s: int) -> list[dict]:
    deadline = time.time() + timeout_s
    last = []
    while time.time() < deadline:
        last = list_statuses(client, kb_id, ds_id)
        by_status: dict[str, int] = {}
        for d in last:
            by_status[d.get("status", "?")] = by_status.get(d.get("status", "?"), 0) + 1
        pending = [d for d in last if d.get("status") not in TERMINAL]
        print(f"  kb={kb_id} {by_status} pending={len(pending)} listed={len(last)}/{expected}")
        if last and len(last) >= expected and not pending:
            return last
        time.sleep(8)
    return last


def summarize(label: str, details: list[dict], expected: int) -> dict:
    failed = [d for d in details if d.get("status") not in OK]
    indexed = [d for d in details if d.get("status") in OK]
    summary = {
        "kb": label,
        "expected": expected,
        "listed": len(details),
        "indexed": len(indexed),
        "failed": [
            {"id": doc_id(d), "status": d.get("status"), "reason": d.get("statusReason")}
            for d in failed
        ],
    }
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default="us-east-1")
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--skip-collect", action="store_true")
    args = parser.parse_args()

    if not args.skip_collect:
        collect()
    manifest_path = OUT / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    docs = documents_for(manifest)
    outputs = load_outputs()
    kbs = outputs["knowledge_bases"]
    client = boto3.client("bedrock-agent", region_name=args.region)

    results = {}
    for label, meta in kbs.items():
        kb_id = meta["knowledge_base_id"]
        ds_id = meta["data_source_id"]
        print(f"Ingesting {len(docs)} documents into {label} ({kb_id})")
        ingest_one(client, kb_id, ds_id, docs)
        details = wait_until_done(client, kb_id, ds_id, expected=len(docs), timeout_s=args.timeout)
        results[label] = summarize(label, details, len(docs))
        print(json.dumps(results[label], indent=2))

    out = Path(__file__).resolve().parents[1] / "results" / "ingest-status.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"documents": len(docs), "kbs": results}, indent=2) + "\n")
    print(f"Wrote {out}")
    failed_any = any(r["failed"] or r["indexed"] < r["expected"] for r in results.values())
    return 1 if failed_any else 0


if __name__ == "__main__":
    sys.exit(main())
