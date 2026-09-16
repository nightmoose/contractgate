#!/usr/bin/env python3
"""Inspect clean vs quarantine topics (confluent-kafka — fast, no full scan)."""

from __future__ import annotations

import argparse
import json
import os
import sys

from confluent_kafka import Consumer, KafkaException, TopicPartition
from confluent_kafka.admin import AdminClient


def end_offsets(bootstrap: str, topic: str) -> int:
    c = Consumer(
        {
            "bootstrap.servers": bootstrap,
            "group.id": f"cg-ralph-inspect-{os.getpid()}",
            "enable.auto.commit": False,
        }
    )
    try:
        md = c.list_topics(topic, timeout=10)
        if topic not in md.topics or md.topics[topic].error is not None:
            return -1
        tps = [
            TopicPartition(topic, p)
            for p in md.topics[topic].partitions
        ]
        ends = c.get_watermark_offsets
        total = 0
        for tp in tps:
            low, high = c.get_watermark_offsets(tp, timeout=10)
            total += max(0, high - low)
        return total
    finally:
        c.close()


def sample(bootstrap: str, topic: str, n: int) -> list:
    c = Consumer(
        {
            "bootstrap.servers": bootstrap,
            "group.id": f"cg-ralph-inspect-sample-{os.getpid()}-{topic}",
            "auto.offset.reset": "earliest",
            "enable.auto.commit": False,
        }
    )
    out = []
    try:
        c.subscribe([topic])
        deadline = 5.0
        import time

        t0 = time.time()
        while len(out) < n and time.time() - t0 < deadline:
            msg = c.poll(0.5)
            if msg is None:
                continue
            if msg.error():
                continue
            try:
                out.append(json.loads(msg.value().decode("utf-8")))
            except Exception:
                out.append({"_raw": msg.value()[:80]})
        return out
    finally:
        c.close()


def preview(val: dict) -> dict:
    if "after" in val or "op" in val:
        after = val.get("after") or {}
        return {
            "op": val.get("op"),
            "id": after.get("id") if isinstance(after, dict) else None,
            "status": after.get("status") if isinstance(after, dict) else None,
            "email": after.get("email") if isinstance(after, dict) else None,
            "sale_price": after.get("sale_price") if isinstance(after, dict) else None,
        }
    return {"keys": list(val)[:8]}


def main() -> None:
    p = argparse.ArgumentParser(description="RFC-092 topic inspector")
    p.add_argument("--bootstrap", default=os.environ.get("KAFKA_BOOTSTRAP", "127.0.0.1:9092"))
    p.add_argument("--n", type=int, default=2, help="sample messages per topic")
    args = p.parse_args()

    admin = AdminClient({"bootstrap.servers": args.bootstrap})
    names = sorted(admin.list_topics(timeout=10).topics.keys())
    print(f"bootstrap={args.bootstrap}")
    print(f"topics={names}")
    print()

    entities = ["orders", "customers", "products"]
    for e in entities:
        for topic in (e, f"{e}.quarantine", f"{e}.raw"):
            try:
                count = end_offsets(args.bootstrap, topic)
            except Exception as exc:  # noqa: BLE001
                print(f"{topic:24} ERROR {exc}")
                continue
            if count < 0:
                print(f"{topic:24} MISSING")
                continue
            print(f"{topic:24} messages≈{count}")
            if count > 0 and args.n > 0 and "quarantine" in topic:
                for s in sample(args.bootstrap, topic, args.n):
                    print(f"    {preview(s) if isinstance(s, dict) else s}")
        print()
    print("Tip: quarantine samples above are the gated bad events.")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"inspect failed: {exc}", file=sys.stderr)
        sys.exit(1)
