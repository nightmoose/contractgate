#!/usr/bin/env python3
"""Refresh freshness-sensitive timestamps in walkthrough example records.

Several example contracts (kafka, kinesis, rag) carry a `freshness` quality rule
that rejects records older than a window (as short as 1h for kinesis). A record
committed with a fixed epoch goes stale and starts failing, so the "passing"
example stops passing. This script stamps the current epoch into the records
that are meant to satisfy freshness, keeping the walkthroughs runnable.

Run before validating the examples (and after checkout on an old branch):

    python3 scripts/refresh_example_freshness.py

Only records intended to PASS the freshness check are refreshed. The kafka/kinesis
`fail.json` records intentionally keep an old timestamp to demonstrate the
freshness rule biting, so they are left untouched.
"""
import json
import pathlib
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent

# (relative path, dotted field holding the epoch-seconds timestamp)
TARGETS = [
    ("examples/contracts/kafka/pass.json", "timestamp"),
    ("examples/contracts/kinesis/pass.json", "timestamp"),
    ("examples/contracts/rag/pass.json", "_cg.ingested_at"),
    ("examples/contracts/rag/fail.json", "_cg.ingested_at"),  # fails on pii_redacted, not freshness
]


def set_dotted(obj: dict, path: str, value: int) -> None:
    keys = path.split(".")
    for k in keys[:-1]:
        obj = obj[k]
    obj[keys[-1]] = value


def main() -> None:
    now = int(time.time())
    for rel, field in TARGETS:
        p = ROOT / rel
        if not p.exists():
            print(f"skip (missing): {rel}")
            continue
        data = json.loads(p.read_text())
        set_dotted(data, field, now)
        p.write_text(json.dumps(data) + "\n")
        print(f"refreshed {rel} :: {field} = {now}")


if __name__ == "__main__":
    main()
