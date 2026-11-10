"""AWS Lambda handler for the ingest + stage gates (Kinesis event source mapping on nmb-cg-raw).

Package this file with gates.py, contracts/, and `pip install contractgate -t .`
(pure Python, no native build). Set STAGED_STREAM and QUARANTINE_STREAM.
The execution role needs kinesis:GetRecords/GetShardIterator/DescribeStream/ListShards
on the raw stream and kinesis:PutRecords on the other two.

With DRY_RUN=1 the handler returns what it would write and calls no AWS API,
which is how `python lambda_function.py` tests it locally.
"""
from __future__ import annotations

import base64
import json
import os

import gates

STAGED = os.environ.get("STAGED_STREAM", "nmb-cg-staged")
QUARANTINE = os.environ.get("QUARANTINE_STREAM", "nmb-cg-quarantine")
DRY_RUN = os.environ.get("DRY_RUN") == "1"
_kin = None


def _put(stream, rows, key):
    global _kin
    if DRY_RUN or not rows:
        return
    if _kin is None:
        import boto3
        _kin = boto3.client("kinesis")
    for i in range(0, len(rows), 500):
        _kin.put_records(StreamName=stream, Records=[{"Data": json.dumps(r).encode(), "PartitionKey": key(r)} for r in rows[i:i + 500]])


def handler(event, context=None):
    staged, quarantined = [], []
    for rec in event.get("Records", []):
        raw = json.loads(base64.b64decode(rec["kinesis"]["data"]))
        ok, viol, _ = gates.check("stripe_charges_raw", raw)
        if not ok:
            quarantined.append({"boundary": "ingest", "contract": "stripe_charges_raw", "violations": viol, "record": raw})
            continue
        row = gates.to_staged(raw)
        ok, viol, _ = gates.check("stripe_charges_staged", row)
        if not ok:
            quarantined.append({"boundary": "stage", "contract": "stripe_charges_staged", "violations": viol, "record": row})
            continue
        staged.append(row)
    _put(STAGED, staged, lambda r: r["customer_id"])
    _put(QUARANTINE, quarantined, lambda r: r["boundary"])
    out = {"records": len(event.get("Records", [])), "staged": len(staged), "quarantined": len(quarantined)}
    if DRY_RUN:
        out["quarantine"] = quarantined
    return out


if __name__ == "__main__":
    os.environ["DRY_RUN"] = "1"
    DRY_RUN = True
    import pathlib
    stripe = pathlib.Path(__file__).resolve().parent.parent / "contract-chain" / "stripe"
    good = json.loads((stripe / "stripe_charges_raw.pass.json").read_text())
    bad = json.loads((stripe / "stripe_charges_raw.fail.json").read_text())
    guest = dict(good); guest.pop("customer")
    ev = {"Records": [{"kinesis": {"data": base64.b64encode(json.dumps(x).encode()).decode()}} for x in (good, bad, guest)]}
    print(json.dumps(handler(ev), indent=2))
