"""Validate a batch of Stripe-shaped charges with ContractGate.

Accepts three input shapes so the same function can be driven without S3:

- Direct invoke / Step Functions payload:
  {"batch_id": "...", "records": [ {...}, ... ], "source": "sfn"}
- EventBridge event (source nmb.contractgate, detail-type BatchReady):
  the `detail` object has the same batch_id + records fields.
- A single charge object (has "id" and "amount"): treated as a one-row batch.

On each batch the handler writes a ledger item to DynamoDB and sends every
failing row to the quarantine SQS queue. It does not write to S3: the box
VPC endpoint policy denies bucket create/list, and the nmb-lambda-validate
role is GetObject-only. See the README for how to drive the pipeline.

Set DRY_RUN=1 to skip AWS writes (used by unit tests and `python handler.py`).
"""
from __future__ import annotations

import json
import os
import time
import uuid
from datetime import datetime, timezone

import gates

CONTRACT = os.environ.get("CONTRACT_NAME", "stripe_charges_raw")
TABLE = os.environ.get("LEDGER_TABLE", "")
QUEUE_URL = os.environ.get("QUARANTINE_QUEUE_URL", "")
DRY_RUN = os.environ.get("DRY_RUN", "") == "1"

_ddb = None
_sqs = None


def _clients():
    global _ddb, _sqs
    if DRY_RUN:
        return None, None
    if _ddb is None or _sqs is None:
        import boto3

        _ddb = boto3.client("dynamodb")
        _sqs = boto3.client("sqs")
    return _ddb, _sqs


def parse_event(event: dict) -> tuple[str, list[dict], str]:
    if not isinstance(event, dict):
        raise ValueError("event must be an object")

    payload = event
    source = "direct"

    # EventBridge -> Step Functions passes the whole event.
    detail = event.get("detail")
    if isinstance(detail, str):
        try:
            detail = json.loads(detail)
        except json.JSONDecodeError:
            detail = None
    if isinstance(detail, dict) and (
        "records" in detail or "Records" in detail or "batch_id" in detail
    ):
        payload = detail
        source = str(event.get("source") or "eventbridge")

    records = payload.get("records")
    if records is None:
        records = payload.get("Records")
    if records is None and "id" in payload and "amount" in payload:
        records = [payload]
    if not isinstance(records, list):
        raise ValueError("event must include a records list")

    batch_id = (
        payload.get("batch_id")
        or payload.get("batchId")
        or f"batch-{uuid.uuid4()}"
    )
    source = payload.get("source") or source
    return str(batch_id), records, str(source)


def _write_ledger(batch_id, source, passed, failed, total, reject_rate, p50_ns):
    ddb, _ = _clients()
    if ddb is None or not TABLE:
        return
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    ddb.put_item(
        TableName=TABLE,
        Item={
            "batch_id": {"S": batch_id},
            "source": {"S": source},
            "contract": {"S": CONTRACT},
            "passed": {"N": str(passed)},
            "failed": {"N": str(failed)},
            "total": {"N": str(total)},
            "reject_rate": {"N": f"{reject_rate:.4f}"},
            "check_ns_p50": {"N": str(int(p50_ns))},
            "ts": {"S": now},
        },
    )


def _quarantine(batch_id, failures):
    _, sqs = _clients()
    if sqs is None or not QUEUE_URL or not failures:
        return
    entries = []
    for i, item in enumerate(failures):
        entries.append(
            {
                "Id": f"{i}",
                "MessageBody": json.dumps(
                    {
                        "batch_id": batch_id,
                        "contract": CONTRACT,
                        "record": item["record"],
                        "violations": item["violations"],
                    },
                    default=str,
                ),
            }
        )
        if len(entries) == 10:
            sqs.send_message_batch(QueueUrl=QUEUE_URL, Entries=entries)
            entries = []
    if entries:
        sqs.send_message_batch(QueueUrl=QUEUE_URL, Entries=entries)


def handler(event, context=None):
    batch_id, records, source = parse_event(event)
    passed_n = 0
    failures = []
    times = []
    # Keep a compact kind histogram for the ledger/log, not every row.
    kinds: dict[str, int] = {}

    for rec in records:
        obj = rec if isinstance(rec, dict) else {}
        ok, viol, ns = gates.check(CONTRACT, obj)
        times.append(ns)
        if ok:
            passed_n += 1
            continue
        failures.append({"record": obj, "violations": viol})
        for v in viol:
            key = f"{v.get('field')}:{v.get('kind')}"
            kinds[key] = kinds.get(key, 0) + 1

    failed_n = len(failures)
    total = len(records)
    reject_rate = (failed_n / total) if total else 0.0
    times_sorted = sorted(times)
    p50_ns = times_sorted[len(times_sorted) // 2] if times_sorted else 0

    _write_ledger(batch_id, source, passed_n, failed_n, total, reject_rate, p50_ns)
    _quarantine(batch_id, failures)

    # Space-delimited so a CloudWatch metric filter can extract the numbers.
    # reject_pct is 0-100.
    print(
        f"BATCH_RESULT {passed_n} {failed_n} {total} {reject_rate * 100:.2f} {batch_id}"
    )

    out = {
        "batch_id": batch_id,
        "source": source,
        "contract": CONTRACT,
        "passed": passed_n,
        "failed": failed_n,
        "total": total,
        "reject_rate": round(reject_rate, 4),
        "check_ns_p50": int(p50_ns),
        "violation_kinds": kinds,
    }
    if DRY_RUN:
        out["failures"] = failures
    return out


if __name__ == "__main__":
    os.environ["DRY_RUN"] = "1"
    DRY_RUN = True
    import pathlib

    samples = pathlib.Path(__file__).resolve().parent.parent / "samples"
    good = json.loads((samples / "stripe_charges_raw.pass.json").read_text())
    bad = json.loads((samples / "stripe_charges_raw.fail.json").read_text())
    ev = {"batch_id": "local-smoke", "source": "local", "records": [good, bad]}
    print(json.dumps(handler(ev), indent=2))
