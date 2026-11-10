"""The three ContractGate checks and the transforms between them.

Boundaries (same contract format and engine at each one):
  ingest  stripe_charges_raw     records as they arrive on the raw stream
  stage   stripe_charges_staged  the transform's output, before the staged stream
  egress  partner_payout_feed    payout rows before they leave for a partner
"""
from __future__ import annotations

import datetime as dt
import os
import zlib
import time
from pathlib import Path

from contractgate import Contract
from contractgate.validator import validate

HERE = Path(__file__).resolve().parent
COMPILED = {
    p.stem: Contract.from_yaml(p.read_text()).compile()
    for p in sorted((HERE / "contracts").glob("*.yaml"))
}

# A known transform bug, switched on for a slice of rows so the stage gate has
# something real to catch: the ms-epoch "created" leaks into charged_at as digits.
TRANSFORM_BUG_RATE = float(os.environ.get("TRANSFORM_BUG_RATE", "0.02"))


def check(contract: str, obj) -> tuple[bool, list[dict], int]:
    t0 = time.perf_counter_ns()
    r = validate(COMPILED[contract], obj if isinstance(obj, dict) else {})
    ns = time.perf_counter_ns() - t0
    viol = [{"field": v.field, "kind": getattr(v.kind, "value", str(v.kind)), "message": v.message} for v in r.violations]
    return r.passed, viol, ns


def to_staged(raw: dict, bug: bool = False) -> dict:
    created = raw.get("created", 0)
    charged_at = (
        str(created * 1000)
        if bug
        else dt.datetime.fromtimestamp(created, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    )
    out = {
        "charge_id": raw["id"],
        "customer_id": raw.get("customer"),
        "amount": round(raw["amount"] / 100, 2),
        "currency": str(raw["currency"]).upper(),
        "status": raw["status"],
        "paid": raw["paid"],
        "refunded": raw["refunded"],
        "charged_at": charged_at,
    }
    if raw.get("description"):
        out["description"] = raw["description"]
    if raw.get("payment_intent"):
        out["payment_intent_id"] = raw["payment_intent"]
    return out


PARTNERS = ["ptr_demo_acme_revshare", "ptr_demo_globex_revshare", "ptr_demo_initech_revshare"]


def payout_rows(staged_rows: list[dict], share: float = 0.10) -> list[dict]:
    """Roll succeeded, unrefunded charges up to one payout row per partner, day, and currency."""
    buckets: dict[tuple, float] = {}
    for r in staged_rows:
        day = r["charged_at"][:10]
        partner = PARTNERS[zlib.crc32(r["customer_id"].encode()) % len(PARTNERS)]
        key = (partner, day, r["currency"])
        net = r["amount"] if (r["status"] == "succeeded" and not r["refunded"]) else 0.0
        buckets[key] = buckets.get(key, 0.0) + net
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    rows = []
    for (partner, day, cur), total in sorted(buckets.items()):
        rows.append({
            "payout_id": f"po_demo_{day.replace('-', '')}_{cur}_{partner[9:]}",
            "partner_id": partner,
            "currency": cur,
            "amount": round(total * share, 2),
            "revenue_date": day,
            "status": "ready",
            "generated_at": now,
        })
    return rows
