#!/usr/bin/env python3
"""Drive the P5 serverless pipeline without writing to S3.

The box VPC endpoint policy denies s3:CreateBucket / s3:ListAllMyBuckets, so
this script never touches S3. It starts Step Functions executions (default)
or puts EventBridge events, then reads the DynamoDB ledger and SQS
quarantine depth.

Commands:
  python scripts/drive.py smoke              # 20 mixed records + one EventBridge batch
  python scripts/drive.py run --records N    # replay N synthetic charges (default 100000)
  python scripts/drive.py status             # ledger counts + queue depth
  python scripts/drive.py poison             # 10 bad rows to trip the reject-rate alarm
"""
from __future__ import annotations

import argparse
import json
import os
import random
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path

import boto3
from botocore.exceptions import ClientError

REGION = os.environ.get("AWS_REGION") or os.environ.get("AWS_DEFAULT_REGION") or "us-east-1"
STACK_PREFIX = os.environ.get("STACK_PREFIX", "nmb")
STATE_MACHINE = os.environ.get("STATE_MACHINE_ARN")
EVENT_BUS = os.environ.get("EVENT_BUS_NAME", f"{STACK_PREFIX}-pipeline")
TABLE = os.environ.get("LEDGER_TABLE", f"{STACK_PREFIX}-pipeline-ledger")
QUEUE_NAME = os.environ.get("QUARANTINE_QUEUE", f"{STACK_PREFIX}-quarantine")


# Same defect mix as examples/aws-kinesis/pipeline.py so the 100k replay is comparable.
RAW_DEFECTS = [
    ("negative_amount", lambda c: c.update(amount=-c["amount"])),
    ("uppercase_currency", lambda c: c.update(currency=c["currency"].upper())),
    ("unknown_status", lambda c: c.update(status="complete")),
    ("bad_id_prefix", lambda c: c.update(id="txn_" + c["id"][3:])),
    ("bad_email", lambda c: c.update(receipt_email="billing at example dot com")),
    ("missing_created", lambda c: c.pop("created")),
]


def session_clients():
    s = boto3.session.Session(region_name=REGION)
    return {
        "sfn": s.client("stepfunctions"),
        "events": s.client("events"),
        "ddb": s.client("dynamodb"),
        "sqs": s.client("sqs"),
    }


def resolve_state_machine(sfn):
    if STATE_MACHINE:
        return STATE_MACHINE
    name = f"{STACK_PREFIX}-contract-pipeline"
    paginator = sfn.get_paginator("list_state_machines")
    for page in paginator.paginate():
        for m in page["stateMachines"]:
            if m["name"] == name:
                return m["stateMachineArn"]
    raise SystemExit(f"state machine {name} not found")


def make_charge(i: int, rng: random.Random, t0: int) -> tuple[dict, str]:
    cur = rng.choices(["usd", "eur", "gbp", "jpy"], [80, 12, 7, 1])[0]
    status = rng.choices(["succeeded", "pending", "failed"], [90, 5, 5])[0]
    c = {
        "id": f"ch_3Qdemo{i:08d}",
        "object": "charge",
        "amount": rng.randint(199, 49999),
        "currency": cur,
        "customer": f"cus_Demo{rng.randint(1, 400):04d}",
        "description": f"Demo invoice INV-DEMO-{i}",
        "paid": status == "succeeded",
        "refunded": rng.random() < 0.03,
        "captured": status == "succeeded",
        "status": status,
        "created": t0 - rng.randint(0, 3 * 86400),
        "livemode": False,
        "receipt_email": f"billing-{i % 997}@example.com",
        "payment_intent": f"pi_3Qdemo{i:08d}",
    }
    if cur == "jpy":
        c.update(status="failed", paid=False, captured=False)
    label = "clean"
    r = rng.random()
    if r < 0.06:
        label, fn = rng.choice(RAW_DEFECTS)
        fn(c)
    elif r < 0.09:
        c.pop("customer")
        label = "guest_checkout"
    return c, label


def batches(n, size, seed=42):
    rng = random.Random(seed)
    t0 = int(time.time())
    labels = Counter()
    buf = []
    all_batches = []
    for i in range(n):
        c, label = make_charge(i, rng, t0)
        labels[label] += 1
        buf.append(c)
        if len(buf) == size:
            all_batches.append(buf)
            buf = []
    if buf:
        all_batches.append(buf)
    return all_batches, labels


def start_one(sfn, arn, batch_id, records, source):
    payload = {"batch_id": batch_id, "source": source, "records": records}
    r = sfn.start_execution(
        stateMachineArn=arn,
        name=batch_id[:80],
        input=json.dumps(payload, default=str),
    )
    return r["executionArn"]


def wait_execution(sfn, arn, timeout=180):
    deadline = time.time() + timeout
    while time.time() < deadline:
        d = sfn.describe_execution(executionArn=arn)
        if d["status"] != "RUNNING":
            return d
        time.sleep(0.4)
    raise TimeoutError(f"execution still running: {arn}")


def put_eventbridge(events, batch_id, records, source="drive"):
    detail = json.dumps({"batch_id": batch_id, "source": source, "records": records})
    r = events.put_events(
        Entries=[
            {
                "Source": "nmb.contractgate",
                "DetailType": "BatchReady",
                "Detail": detail,
                "EventBusName": EVENT_BUS,
            }
        ]
    )
    failed = r.get("FailedEntryCount", 0)
    if failed:
        raise RuntimeError(f"PutEvents failed: {r}")
    return r["Entries"][0].get("EventId")


def ledger_scan(ddb, prefix=None):
    paginator = ddb.get_paginator("scan")
    items = []
    for page in paginator.paginate(TableName=TABLE):
        for it in page.get("Items", []):
            bid = it["batch_id"]["S"]
            if prefix and not bid.startswith(prefix):
                continue
            items.append(
                {
                    "batch_id": bid,
                    "source": it.get("source", {}).get("S"),
                    "passed": int(it["passed"]["N"]),
                    "failed": int(it["failed"]["N"]),
                    "total": int(it["total"]["N"]),
                    "reject_rate": float(it.get("reject_rate", {}).get("N", "0")),
                    "ts": it.get("ts", {}).get("S"),
                }
            )
    return items


def queue_attrs(sqs):
    url = sqs.get_queue_url(QueueName=QUEUE_NAME)["QueueUrl"]
    attrs = sqs.get_queue_attributes(
        QueueUrl=url,
        AttributeNames=[
            "ApproximateNumberOfMessages",
            "ApproximateNumberOfMessagesNotVisible",
            "ApproximateNumberOfMessagesDelayed",
        ],
    )["Attributes"]
    dlq_name = f"{QUEUE_NAME}-dlq"
    try:
        dlq_url = sqs.get_queue_url(QueueName=dlq_name)["QueueUrl"]
        dlq = sqs.get_queue_attributes(
            QueueUrl=dlq_url, AttributeNames=["ApproximateNumberOfMessages"]
        )["Attributes"]
    except ClientError:
        dlq = {"ApproximateNumberOfMessages": "n/a"}
    return url, attrs, dlq


def summarize(items, labels=None, extra=None):
    passed = sum(i["passed"] for i in items)
    failed = sum(i["failed"] for i in items)
    total = sum(i["total"] for i in items)
    out = {
        "batches": len(items),
        "passed": passed,
        "failed": failed,
        "total": total,
        "reject_rate": (failed / total) if total else 0.0,
    }
    if labels is not None:
        out["produced_labels"] = dict(labels)
    if extra:
        out.update(extra)
    return out


def cmd_run(args):
    c = session_clients()
    arn = resolve_state_machine(c["sfn"])
    run_id = args.run_id or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")
    chunks, labels = batches(args.records, args.batch_size, seed=args.seed)
    print(
        f"starting {len(chunks)} executions "
        f"({args.records} records, batch {args.batch_size}, "
        f"concurrency {args.concurrency}) on {arn}",
        flush=True,
    )
    t0 = time.time()
    exec_arns = []
    errors = []

    def submit(idx, recs):
        bid = f"{args.prefix}{run_id}-{idx:05d}"
        return start_one(c["sfn"], arn, bid, recs, args.source)

    with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futs = {pool.submit(submit, i, recs): i for i, recs in enumerate(chunks)}
        for fut in as_completed(futs):
            i = futs[fut]
            try:
                exec_arns.append(fut.result())
            except Exception as e:
                errors.append({"index": i, "error": str(e)})

    print(f"started {len(exec_arns)} / {len(chunks)} in {time.time() - t0:.1f}s", flush=True)

    results = []
    with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futs = [pool.submit(wait_execution, c["sfn"], a, args.timeout) for a in exec_arns]
        for fut in as_completed(futs):
            try:
                results.append(fut.result())
            except Exception as e:
                errors.append({"error": str(e)})

    elapsed = time.time() - t0
    status_counts = Counter(r["status"] for r in results)
    failed_execs = [r for r in results if r["status"] != "SUCCEEDED"]
    time.sleep(2)
    items = ledger_scan(c["ddb"], prefix=f"{args.prefix}{run_id}")
    _, qattrs, dlq = queue_attrs(c["sqs"])
    summary = summarize(
        items,
        labels=labels,
        extra={
            "run_id": run_id,
            "state_machine": arn,
            "executions_started": len(exec_arns),
            "execution_status": dict(status_counts),
            "start_errors": errors,
            "failed_executions": [
                {"arn": r["executionArn"], "status": r["status"], "error": r.get("error")}
                for r in failed_execs
            ],
            "elapsed_s": round(elapsed, 2),
            "records_per_s": round(args.records / elapsed, 1) if elapsed else 0,
            "quarantine_visible": int(qattrs.get("ApproximateNumberOfMessages", 0)),
            "quarantine_not_visible": int(qattrs.get("ApproximateNumberOfMessagesNotVisible", 0)),
            "dlq_visible": dlq.get("ApproximateNumberOfMessages"),
            "region": REGION,
            "ts": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        },
    )
    out_dir = Path(__file__).resolve().parents[1] / "runs"
    out_dir.mkdir(exist_ok=True)
    path = out_dir / f"{run_id}-{args.records}recs.json"
    path.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    print(f"wrote {path}")
    if failed_execs or errors:
        sys.exit(2)


def cmd_smoke(args):
    c = session_clients()
    arn = resolve_state_machine(c["sfn"])
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")
    chunks, labels = batches(20, 20, seed=1)
    bid = f"smoke-{run_id}"
    exec_arn = start_one(c["sfn"], arn, bid, chunks[0], "smoke-sfn")
    desc = wait_execution(c["sfn"], exec_arn, timeout=120)
    print("sfn", desc["status"], desc.get("output"))

    eb_chunks, _ = batches(5, 5, seed=2)
    eb_id = f"smoke-eb-{run_id}"
    event_id = put_eventbridge(c["events"], eb_id, eb_chunks[0], source="smoke-eventbridge")
    print("eventbridge", event_id, "batch", eb_id)
    deadline = time.time() + 60
    eb_items = []
    while time.time() < deadline:
        eb_items = ledger_scan(c["ddb"], prefix=eb_id)
        if eb_items:
            break
        time.sleep(2)
    sfn_items = ledger_scan(c["ddb"], prefix=bid)
    _, qattrs, dlq = queue_attrs(c["sqs"])
    out = {
        "sfn_execution": desc["status"],
        "sfn_output": json.loads(desc["output"]) if desc.get("output") else None,
        "sfn_ledger": sfn_items,
        "eventbridge_event_id": event_id,
        "eventbridge_ledger": eb_items,
        "produced_labels": dict(labels),
        "quarantine_visible": qattrs.get("ApproximateNumberOfMessages"),
        "dlq_visible": dlq.get("ApproximateNumberOfMessages"),
    }
    print(json.dumps(out, indent=2))
    if desc["status"] != "SUCCEEDED":
        sys.exit(2)
    if not sfn_items:
        print("warning: SFN succeeded but ledger item not found yet", file=sys.stderr)
        sys.exit(3)
    if not eb_items:
        print("warning: EventBridge batch not in ledger after 60s", file=sys.stderr)
        sys.exit(4)


def cmd_poison(args):
    c = session_clients()
    arn = resolve_state_machine(c["sfn"])
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")
    bad = []
    for i in range(10):
        bad.append(
            {
                "id": f"txn_poison{i:04d}",
                "object": "charge",
                "amount": -1,
                "currency": "USD",
                "paid": True,
                "refunded": False,
                "status": "complete",
            }
        )
    bid = f"poison-{run_id}"
    exec_arn = start_one(c["sfn"], arn, bid, bad, "poison")
    desc = wait_execution(c["sfn"], exec_arn, timeout=120)
    out = json.loads(desc["output"]) if desc.get("output") else None
    print(json.dumps({"status": desc["status"], "output": out}, indent=2))
    if desc["status"] != "SUCCEEDED":
        sys.exit(2)


def cmd_status(args):
    c = session_clients()
    items = ledger_scan(c["ddb"], prefix=args.prefix or None)
    _, qattrs, dlq = queue_attrs(c["sqs"])
    print(
        json.dumps(
            {
                "ledger": summarize(items),
                "quarantine": qattrs,
                "dlq": dlq,
            },
            indent=2,
        )
    )


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("run", help="replay N synthetic charges through Step Functions")
    r.add_argument("--records", type=int, default=100000)
    r.add_argument("--batch-size", type=int, default=200)
    r.add_argument("--concurrency", type=int, default=10)
    r.add_argument("--seed", type=int, default=42)
    r.add_argument("--timeout", type=int, default=180, help="seconds to wait for one execution")
    r.add_argument("--run-id", default=None)
    r.add_argument("--prefix", default="run-")
    r.add_argument("--source", default="sfn-drive")
    r.set_defaults(func=cmd_run)

    s = sub.add_parser("smoke", help="20-record SFN smoke + 5-record EventBridge batch")
    s.set_defaults(func=cmd_smoke)

    x = sub.add_parser("poison", help="10 all-bad rows (high reject rate)")
    x.set_defaults(func=cmd_poison)

    t = sub.add_parser("status", help="ledger sums and queue depth")
    t.add_argument("--prefix", default=None)
    t.set_defaults(func=cmd_status)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
