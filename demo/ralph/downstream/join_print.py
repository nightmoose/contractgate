#!/usr/bin/env python3
"""Lightweight Stage B: Kafi joins on CLEAN topics → stdout (no LanceDB/MCP).

Use this when you want to prove gated data reaches Driftless' Streams path
without pulling embeddings. Full MCP path is ``app.py``.
"""

from __future__ import annotations

import os
import signal
import sys
import time
from pathlib import Path

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

from kafka.kafka import streams  # noqa: E402

os.environ.setdefault("KAFKA_BOOTSTRAP", "127.0.0.1:9092")

_count = 0


def sink_fun(m_w_tuple_list):
    global _count
    for m, w in m_w_tuple_list:
        if w != 1:
            continue
        v = m.get("value") or m
        _count += 1
        cust = v.get("customer") or {}
        prod = v.get("product") or {}
        print(
            f"[join] #{_count} order={v.get('id')} "
            f"customer={cust.get('first_name')} {cust.get('last_name')} "
            f"product={prod.get('name')} status={v.get('status')}",
            flush=True,
        )


def main() -> None:
    print(
        f"[join_print] reading clean topics @ {os.environ['KAFKA_BOOTSTRAP']}",
        flush=True,
    )
    stop_fun = streams(sink_fun, emulated=False)
    running = True

    def _stop(*_a):
        nonlocal running
        running = False

    signal.signal(signal.SIGINT, _stop)
    signal.signal(signal.SIGTERM, _stop)
    while running:
        time.sleep(0.5)
    try:
        stop_fun()
    except Exception:
        pass
    print(f"[join_print] stopped after {_count} joins", flush=True)


if __name__ == "__main__":
    main()
