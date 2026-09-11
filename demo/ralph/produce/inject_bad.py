#!/usr/bin/env python3
"""Inject deliberately bad CDC after-images into *.raw (RFC-092)."""

from __future__ import annotations

import argparse
import json
import os
import uuid

from confluent_kafka import Producer

BOOTSTRAP = os.environ.get("KAFKA_BOOTSTRAP", "127.0.0.1:9092")
ENTITIES = ("orders", "customers", "products")


def envelope(after: dict | None, op: str = "c", before: dict | None = None) -> bytes:
    return json.dumps(
        {"before": before, "after": after, "op": op},
        separators=(",", ":"),
    ).encode()


BAD = {
    "orders": {
        "key": "bad-order-1",
        "value": envelope(
            {
                "id": 999001,
                "product_id": "828044c8-832b-48b4-b1f0-89a500496335",
                "customer_id": "01a429f8-f576-4d9b-8d0e-4d5e8ef37e09",
                "status_id": 0,
                "status": "NOT_A_REAL_STATUS",
                "ts": 1788854186242,
            }
        ),
    },
    "customers": {
        "key": str(uuid.uuid4()),
        "value": envelope(
            {
                "id": str(uuid.uuid4()),
                "first_name": "Bad",
                "last_name": "Actor",
                "email": "not-an-email",
                "phone": "000",
                "street_address": "1 Evil Lane",
                "state": "NY",
                "zip_code": "xx",
            }
        ),
    },
    "products": {
        "key": str(uuid.uuid4()),
        "value": envelope(
            {
                "id": str(uuid.uuid4()),
                "brand": "Nope",
                "name": "Negative Price Shoe",
                "sale_price": -50,
                "rating": 0,
            }
        ),
    },
}


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument(
        "--entity",
        choices=["orders", "customers", "products", "all"],
        default="all",
    )
    p.add_argument("--bootstrap", default=BOOTSTRAP)
    args = p.parse_args()
    producer = Producer({"bootstrap.servers": args.bootstrap, "acks": "all"})
    entities = ENTITIES if args.entity == "all" else [args.entity]
    for entity in entities:
        item = BAD[entity]
        topic = f"{entity}.raw"
        producer.produce(topic, key=item["key"].encode(), value=item["value"])
        print(f"[inject_bad] → {topic} key={item['key']}", flush=True)
    producer.flush(5)


if __name__ == "__main__":
    main()
