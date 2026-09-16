#!/usr/bin/env python3
"""Produce Debezium-style shoe CDC into *.raw topics (RFC-092).

Adapted from xdgrulez/driftless producer.py (Apache-2.0) — writes to
orders.raw / customers.raw / products.raw so ContractGate can gate
before clean topics.
"""

from __future__ import annotations

import json
import os
import signal
import sys
import threading
import time
from pathlib import Path

from confluent_kafka import Producer
from confluent_kafka.admin import AdminClient, NewTopic

# datagen lives next to this file
sys.path.insert(0, str(Path(__file__).resolve().parent))
from datagen.shoe_customers import ShoeCustomerGenerator  # noqa: E402
from datagen.shoe_orders import ShoeOrderGenerator  # noqa: E402
from datagen.shoes import ShoeProductGenerator  # noqa: E402

BOOTSTRAP = os.environ.get("KAFKA_BOOTSTRAP", "127.0.0.1:9092")
RAW = {
    "orders": "orders.raw",
    "customers": "customers.raw",
    "products": "products.raw",
}


def ensure_topics(bootstrap: str) -> None:
    admin = AdminClient({"bootstrap.servers": bootstrap})
    names = list(RAW.values()) + [
        "orders",
        "customers",
        "products",
        "orders.quarantine",
        "customers.quarantine",
        "products.quarantine",
    ]
    existing = set(admin.list_topics(timeout=10).topics.keys())
    new = [NewTopic(t, num_partitions=1, replication_factor=1) for t in names if t not in existing]
    if not new:
        return
    fs = admin.create_topics(new)
    for t, f in fs.items():
        try:
            f.result()
            print(f"[producers] created topic {t}", flush=True)
        except Exception as exc:  # noqa: BLE001
            print(f"[producers] topic {t}: {exc}", flush=True)


def serialize(m: dict) -> tuple[str | None, bytes]:
    """kafi produce_list items are {key, value}; we publish value JSON."""
    key = m.get("key")
    key_b = str(key).encode() if key is not None else None
    val = m.get("value", m)
    return key_b, json.dumps(val, separators=(",", ":")).encode()


def loop_orders(producer: Producer, stop: threading.Event) -> None:
    gen = ShoeOrderGenerator()
    while not stop.is_set():
        for m in gen.generate(n_int=5):
            k, v = serialize(m)
            producer.produce(RAW["orders"], key=k, value=v)
        producer.poll(0)
        time.sleep(1.0)


def loop_customers(producer: Producer, stop: threading.Event) -> None:
    gen = ShoeCustomerGenerator()
    while not stop.is_set():
        for m in gen.generate(n_int=5):
            k, v = serialize(m)
            producer.produce(RAW["customers"], key=k, value=v)
        producer.poll(0)
        time.sleep(0.2)


def loop_products(producer: Producer, stop: threading.Event) -> None:
    gen = ShoeProductGenerator()
    while not stop.is_set():
        for m in gen.generate(n_int=5):
            k, v = serialize(m)
            producer.produce(RAW["products"], key=k, value=v)
        producer.poll(0)
        time.sleep(0.2)


def main() -> None:
    ensure_topics(BOOTSTRAP)
    producer = Producer({"bootstrap.servers": BOOTSTRAP, "linger.ms": 5, "acks": "all"})
    stop = threading.Event()

    def _stop(*_a):
        stop.set()

    signal.signal(signal.SIGINT, _stop)
    signal.signal(signal.SIGTERM, _stop)

    threads = [
        threading.Thread(target=loop_orders, args=(producer, stop), daemon=True),
        threading.Thread(target=loop_customers, args=(producer, stop), daemon=True),
        threading.Thread(target=loop_products, args=(producer, stop), daemon=True),
    ]
    for t in threads:
        t.start()
    print(f"[producers] writing to {list(RAW.values())} @ {BOOTSTRAP}", flush=True)
    while not stop.is_set():
        time.sleep(0.5)
    producer.flush(5)
    print("[producers] stopped", flush=True)


if __name__ == "__main__":
    main()
