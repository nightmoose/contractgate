#!/usr/bin/env python3
"""ContractGate bridge for the Ralph demo (RFC-092).

Consumes Debezium-style envelopes from *.raw topics, validates the
``after`` payload with ContractGate (local Python SDK by default), and
produces to clean topics (orders/customers/products) or *.quarantine.

Deletes (op=d, no after) pass through to clean unchanged.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import time
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

# Allow `pip install -e sdks/python` OR sibling path from repo root.
_REPO = Path(__file__).resolve().parents[3]
_SDK = _REPO / "sdks" / "python" / "src"
if _SDK.is_dir():
    sys.path.insert(0, str(_SDK))

from contractgate.contract import CompiledContract, Contract  # noqa: E402
from contractgate.validator import validate  # noqa: E402

try:
    from confluent_kafka import Consumer, KafkaError, KafkaException, Producer
    from confluent_kafka.admin import AdminClient, NewTopic
except ImportError as e:  # pragma: no cover
    raise SystemExit(
        "confluent-kafka is required. pip install -r demo/ralph/requirements.txt"
    ) from e

ENTITIES = ("orders", "customers", "products")
CONTRACT_DIR = Path(__file__).resolve().parents[1] / "contracts"


def load_contracts() -> Dict[str, CompiledContract]:
    mapping = {
        "orders": "orders.yaml",
        "customers": "customers.yaml",
        "products": "products.yaml",
    }
    out: Dict[str, CompiledContract] = {}
    for entity, fname in mapping.items():
        path = CONTRACT_DIR / fname
        out[entity] = Contract.from_yaml(path.read_text(encoding="utf-8")).compile()
    return out


def unwrap_after(envelope: Dict[str, Any]) -> Tuple[Optional[Dict[str, Any]], str]:
    """Return (after_or_none, op). Supports bare after or {before,after,op}."""
    if "op" in envelope or "after" in envelope or "before" in envelope:
        op = str(envelope.get("op") or "c")
        after = envelope.get("after")
        return (after if isinstance(after, dict) else None), op
    # Already a flat after-image
    return envelope, "c"


def decide(
    compiled: CompiledContract, envelope: Dict[str, Any]
) -> Tuple[bool, Dict[str, Any]]:
    """Return (passed, meta). Deletes always pass."""
    after, op = unwrap_after(envelope)
    if op == "d" or after is None:
        return True, {"op": op, "reason": "delete_passthrough"}
    result = validate(compiled, after)
    meta = {
        "op": op,
        "passed": result.passed,
        "violations": [
            {"field": v.field, "message": v.message, "kind": str(v.kind)}
            for v in result.violations
        ],
    }
    return result.passed, meta


def make_producer(bootstrap: str) -> Producer:
    return Producer(
        {
            "bootstrap.servers": bootstrap,
            "linger.ms": 5,
            "acks": "all",
        }
    )


def ensure_topics(bootstrap: str) -> None:
    admin = AdminClient({"bootstrap.servers": bootstrap})
    names = [f"{e}.raw" for e in ENTITIES] + list(ENTITIES) + [
        f"{e}.quarantine" for e in ENTITIES
    ]
    existing = set(admin.list_topics(timeout=10).topics.keys())
    new = [NewTopic(t, num_partitions=1, replication_factor=1) for t in names if t not in existing]
    if not new:
        return
    for t, f in admin.create_topics(new).items():
        try:
            f.result()
            print(f"[gate_bridge] created topic {t}", flush=True)
        except Exception as exc:  # noqa: BLE001
            print(f"[gate_bridge] topic {t}: {exc}", flush=True)


def make_consumer(bootstrap: str, group: str) -> Consumer:
    c = Consumer(
        {
            "bootstrap.servers": bootstrap,
            "group.id": group,
            "auto.offset.reset": "earliest",
            "enable.auto.commit": True,
            # Don't abort the process if a topic is briefly missing at subscribe time.
            "allow.auto.create.topics": True,
        }
    )
    topics = [f"{e}.raw" for e in ENTITIES]
    c.subscribe(topics)
    return c


def entity_from_topic(topic: str) -> str:
    # orders.raw → orders
    return topic[: -len(".raw")] if topic.endswith(".raw") else topic


def run(bootstrap: str, group: str) -> None:
    contracts = load_contracts()
    ensure_topics(bootstrap)
    producer = make_producer(bootstrap)
    consumer = make_consumer(bootstrap, group)
    running = True

    def _stop(*_args: Any) -> None:
        nonlocal running
        running = False

    signal.signal(signal.SIGINT, _stop)
    signal.signal(signal.SIGTERM, _stop)

    print(
        f"[gate_bridge] bootstrap={bootstrap} group={group} "
        f"mode=local contracts={list(contracts)}",
        flush=True,
    )

    passed_n = failed_n = 0
    while running:
        msg = consumer.poll(0.5)
        if msg is None:
            continue
        if msg.error():
            err = msg.error()
            # Topic may not exist yet for a moment after create — keep polling.
            if err.code() in (
                KafkaError.UNKNOWN_TOPIC_OR_PART,
                KafkaError._PARTITION_EOF,
            ):
                continue
            raise KafkaException(err)

        entity = entity_from_topic(msg.topic())
        compiled = contracts[entity]
        try:
            envelope = json.loads(msg.value().decode("utf-8"))
        except Exception as exc:
            # Unparseable → quarantine whole payload
            failed_n += 1
            qtopic = f"{entity}.quarantine"
            producer.produce(
                qtopic,
                key=msg.key(),
                value=msg.value(),
                headers=[("cg-violation-reason", str(exc).encode())],
            )
            producer.poll(0)
            print(f"[gate_bridge] QUARANTINE {entity} (json): {exc}", flush=True)
            continue

        # Support both kafi produce_list shapes: {"key","value"} wrapper
        # already unwrapped by Kafka, or value is the debezium dict.
        if isinstance(envelope, dict) and "value" in envelope and "op" not in envelope:
            # accidental double wrap
            inner = envelope["value"]
            if isinstance(inner, dict):
                envelope = inner

        ok, meta = decide(compiled, envelope)
        key = msg.key()
        payload = json.dumps(envelope, separators=(",", ":")).encode("utf-8")

        if ok:
            passed_n += 1
            producer.produce(entity, key=key, value=payload)
            if passed_n % 50 == 0:
                print(f"[gate_bridge] ok={passed_n} fail={failed_n}", flush=True)
        else:
            failed_n += 1
            qtopic = f"{entity}.quarantine"
            reason = json.dumps(meta["violations"]).encode("utf-8")
            producer.produce(
                qtopic,
                key=key,
                value=payload,
                headers=[("cg-violation-reason", reason)],
            )
            print(
                f"[gate_bridge] QUARANTINE {entity}: {meta['violations'][:2]}",
                flush=True,
            )
        producer.poll(0)

    producer.flush(5)
    consumer.close()
    print(f"[gate_bridge] stopped ok={passed_n} fail={failed_n}", flush=True)


def main() -> None:
    p = argparse.ArgumentParser(description="RFC-092 ContractGate Ralph bridge")
    p.add_argument(
        "--bootstrap",
        default=os.environ.get("KAFKA_BOOTSTRAP", "127.0.0.1:9092"),
    )
    p.add_argument(
        "--group",
        default=os.environ.get("CG_BRIDGE_GROUP", "contractgate-ralph-bridge"),
    )
    args = p.parse_args()
    # Warm compile check
    load_contracts()
    run(args.bootstrap, args.group)


if __name__ == "__main__":
    main()
