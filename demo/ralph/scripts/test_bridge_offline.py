#!/usr/bin/env python3
"""Offline unit checks for gate_bridge decide() — no Docker required."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bridge"))
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "sdks" / "python" / "src"))

from gate_bridge import decide, load_contracts  # noqa: E402


def main() -> None:
    contracts = load_contracts()
    good_order = {
        "before": None,
        "after": {
            "id": 1,
            "product_id": "828044c8-832b-48b4-b1f0-89a500496335",
            "customer_id": "01a429f8-f576-4d9b-8d0e-4d5e8ef37e09",
            "status_id": 0,
            "status": "LOOKED_AT (in shopping cart)",
            "ts": 1788854186242,
        },
        "op": "c",
    }
    ok, meta = decide(contracts["orders"], good_order)
    assert ok, meta

    bad = {
        "before": None,
        "after": {**good_order["after"], "status": "NOPE"},
        "op": "c",
    }
    ok, meta = decide(contracts["orders"], bad)
    assert not ok, meta

    delete = {"before": good_order["after"], "after": None, "op": "d"}
    ok, meta = decide(contracts["orders"], delete)
    assert ok and meta.get("reason") == "delete_passthrough", meta

    print("offline bridge tests PASS")


if __name__ == "__main__":
    main()
