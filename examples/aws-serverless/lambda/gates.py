"""In-process ContractGate checks for the P5 validate Lambda.

Uses the same stripe_charges_raw contract as examples/aws-kinesis and
examples/contract-chain/stripe. The Python package is self-contained so
the function does not call the staging or production ContractGate API.
"""
from __future__ import annotations

import time
from pathlib import Path

from contractgate import Contract
from contractgate.validator import validate

HERE = Path(__file__).resolve().parent
COMPILED = {
    p.stem: Contract.from_yaml(p.read_text()).compile()
    for p in sorted((HERE / "contracts").glob("*.yaml"))
}


def check(contract: str, obj) -> tuple[bool, list[dict], int]:
    t0 = time.perf_counter_ns()
    r = validate(COMPILED[contract], obj if isinstance(obj, dict) else {})
    ns = time.perf_counter_ns() - t0
    viol = [
        {
            "field": v.field,
            "kind": getattr(v.kind, "value", str(v.kind)),
            "message": v.message,
        }
        for v in r.violations
    ]
    return r.passed, viol, ns
