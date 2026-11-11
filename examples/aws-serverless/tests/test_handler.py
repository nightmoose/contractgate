"""Local tests for the P5 validate handler. No AWS calls (DRY_RUN=1)."""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ["DRY_RUN"] = "1"
sys.path.insert(0, str(ROOT / "lambda"))

import handler  # noqa: E402

SAMPLES = ROOT / "samples"
GOOD = json.loads((SAMPLES / "stripe_charges_raw.pass.json").read_text())
BAD = json.loads((SAMPLES / "stripe_charges_raw.fail.json").read_text())


def test_direct_batch_pass_and_fail():
    out = handler.handler({"batch_id": "t1", "source": "test", "records": [GOOD, BAD]})
    assert out["passed"] == 1
    assert out["failed"] == 1
    assert out["total"] == 2
    assert out["batch_id"] == "t1"
    assert len(out["failures"]) == 1
    fields = {v["field"] for v in out["failures"][0]["violations"]}
    assert "amount" in fields
    assert "currency" in fields or "status" in fields


def test_eventbridge_envelope():
    ev = {
        "version": "0",
        "source": "nmb.contractgate",
        "detail-type": "BatchReady",
        "detail": {"batch_id": "eb-1", "records": [GOOD]},
    }
    out = handler.handler(ev)
    assert out["batch_id"] == "eb-1"
    assert out["passed"] == 1
    assert out["failed"] == 0
    assert out["source"] == "nmb.contractgate"


def test_eventbridge_detail_as_string():
    ev = {
        "source": "nmb.contractgate",
        "detail-type": "BatchReady",
        "detail": json.dumps({"batch_id": "eb-2", "records": [BAD]}),
    }
    out = handler.handler(ev)
    assert out["batch_id"] == "eb-2"
    assert out["failed"] == 1
    assert out["passed"] == 0


def test_single_charge():
    out = handler.handler(GOOD)
    assert out["total"] == 1
    assert out["passed"] == 1


def test_empty_batch():
    out = handler.handler({"batch_id": "empty", "records": []})
    assert out["total"] == 0
    assert out["reject_rate"] == 0


def test_missing_records_raises():
    try:
        handler.handler({"batch_id": "nope"})
    except ValueError as e:
        assert "records" in str(e)
    else:
        raise AssertionError("expected ValueError")


if __name__ == "__main__":
    tests = [
        test_direct_batch_pass_and_fail,
        test_eventbridge_envelope,
        test_eventbridge_detail_as_string,
        test_single_charge,
        test_empty_batch,
        test_missing_records_raises,
    ]
    for t in tests:
        t()
        print("ok", t.__name__)
    print(f"{len(tests)} passed")
