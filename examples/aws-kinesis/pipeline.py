#!/usr/bin/env python3
"""Kinesis Data Streams pipeline with ContractGate checks at ingest, between stages, and egress.

  producer ──► nmb-cg-raw ──► [ingest gate: stripe_charges_raw]
                                 │ pass → transform → [stage gate: stripe_charges_staged]
                                 │                       │ pass → nmb-cg-staged ──► [egress gate: partner_payout_feed]
                                 │                       │                              │ pass → nmb-cg-payouts
                                 └── fail ───────────────┴── fail ──────────────────────┴── fail → nmb-cg-quarantine

Commands:
  python pipeline.py up                 create the four streams (tagged), wait until ACTIVE
  python pipeline.py run --records N    produce N charges and run every gate, print the numbers
  python pipeline.py down               delete the streams
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import random
import statistics
import sys
import threading
import time

import boto3

import gates

PREFIX = os.environ.get("STREAM_PREFIX", "nmb-cg")
STREAMS = {"raw": 2, "staged": 2, "quarantine": 1, "payouts": 1}
TAGS = {"Project": "nightmoose-bots", "Owner": "chief", "Purpose": "contractgate-kinesis-demo"}
kin = boto3.client("kinesis", region_name=os.environ.get("AWS_REGION", os.environ.get("AWS_DEFAULT_REGION", "us-east-1")))


def name(k):
    return f"{PREFIX}-{k}"


def up():
    for k, shards in STREAMS.items():
        try:
            kin.create_stream(StreamName=name(k), ShardCount=shards, StreamModeDetails={"StreamMode": "PROVISIONED"}, Tags=TAGS)
            print("creating", name(k), shards, "shard(s)")
        except kin.exceptions.ResourceInUseException:
            print("exists", name(k))
    for k in STREAMS:
        kin.get_waiter("stream_exists").wait(StreamName=name(k), WaiterConfig={"Delay": 5})
        print("ACTIVE", name(k))


def down():
    for k in STREAMS:
        try:
            kin.delete_stream(StreamName=name(k), EnforceConsumerDeletion=True)
            print("deleting", name(k))
        except kin.exceptions.ResourceNotFoundException:
            print("gone", name(k))


# ── synthetic Stripe-shaped charges with realistic defects ──────────────────
RAW_DEFECTS = [
    ("negative_amount", lambda c: c.update(amount=-c["amount"])),
    ("uppercase_currency", lambda c: c.update(currency=c["currency"].upper())),
    ("unknown_status", lambda c: c.update(status="complete")),
    ("bad_id_prefix", lambda c: c.update(id="txn_" + c["id"][3:])),
    ("bad_email", lambda c: c.update(receipt_email="billing at example dot com")),
    ("missing_created", lambda c: c.pop("created")),
]


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
        c.update(status="failed", paid=False, captured=False)  # a currency whose day nets to zero
    label = "clean"
    r = rng.random()
    if r < 0.06:
        label, f = rng.choice(RAW_DEFECTS)
        f(c)
    elif r < 0.09:
        c.pop("customer")  # guest checkout: fine at ingest, staged requires customer_id
        label = "guest_checkout"
    return c, label


def produce(n: int, rate: int, stats: dict):
    rng = random.Random(42)
    t0 = int(time.time())
    batch, sent, start = [], 0, time.time()
    for i in range(n):
        c, label = make_charge(i, rng, t0)
        stats["produced_labels"][label] += 1
        stats["produced"] = i + 1
        batch.append({"Data": json.dumps(c).encode(), "PartitionKey": c.get("customer", c["id"])})
        if len(batch) == 500 or i == n - 1:
            pending = batch
            while pending:
                r = kin.put_records(StreamName=name("raw"), Records=pending)
                if r["FailedRecordCount"]:
                    pending = [rec for rec, res in zip(pending, r["Records"]) if "ErrorCode" in res]
                    stats["put_retries"] += len(pending)
                    time.sleep(0.2)
                else:
                    pending = []
            sent += len(batch)
            batch = []
            ahead = sent / rate - (time.time() - start)
            if ahead > 0:
                time.sleep(ahead)
    stats["produce_s"] = time.time() - start


def put_all(stream: str, records: list[dict]):
    for j in range(0, len(records), 500):
        chunk = records[j:j + 500]
        while chunk:
            r = kin.put_records(StreamName=stream, Records=chunk)
            chunk = [rec for rec, res in zip(chunk, r["Records"]) if "ErrorCode" in res] if r["FailedRecordCount"] else []
            if chunk:
                time.sleep(0.2)


def shard_iters(stream: str, since: float):
    shards = kin.list_shards(StreamName=stream)["Shards"]
    return [kin.get_shard_iterator(StreamName=stream, ShardId=s["ShardId"], ShardIteratorType="AT_TIMESTAMP", Timestamp=since)["ShardIterator"] for s in shards]


def ingest_and_stage_worker(it: str, stats: dict, lock: threading.Lock, stop: threading.Event):
    rng = random.Random(hash(it) & 0xFFFF)
    while it and not stop.is_set():
        r = kin.get_records(ShardIterator=it, Limit=2000)
        it = r.get("NextShardIterator")
        recs = r["Records"]
        if not recs:
            time.sleep(0.25)
            continue
        staged_out, quarantine_out, lat, gate_ns = [], [], [], []
        for rec in recs:
            raw = json.loads(rec["Data"])
            ok, viol, ns = gates.check("stripe_charges_raw", raw)
            gate_ns.append(ns)
            if not ok:
                quarantine_out.append({"Data": json.dumps({"boundary": "ingest", "contract": "stripe_charges_raw", "violations": viol, "record": raw}).encode(), "PartitionKey": "ingest"})
                for v in viol:
                    stats["violations"][("ingest", v["field"], v["kind"])] += 1
                continue
            staged = gates.to_staged(raw, bug=rng.random() < gates.TRANSFORM_BUG_RATE)
            ok2, viol2, ns2 = gates.check("stripe_charges_staged", staged)
            gate_ns.append(ns2)
            if not ok2:
                quarantine_out.append({"Data": json.dumps({"boundary": "stage", "contract": "stripe_charges_staged", "violations": viol2, "record": staged}).encode(), "PartitionKey": "stage"})
                for v in viol2:
                    stats["violations"][("stage", v["field"], v["kind"])] += 1
                continue
            staged_out.append({"Data": json.dumps(staged).encode(), "PartitionKey": staged["customer_id"]})
        put_all(name("staged"), staged_out)
        put_all(name("quarantine"), quarantine_out)
        done = time.time()
        for rec in recs:
            lat.append((done - rec["ApproximateArrivalTimestamp"].timestamp()) * 1000)
        with lock:
            stats["raw_read"] += len(recs)
            stats["ingest_quarantined"] += sum(1 for q in quarantine_out if b'"boundary": "ingest"' in q["Data"])
            stats["stage_quarantined"] += sum(1 for q in quarantine_out if b'"boundary": "stage"' in q["Data"])
            stats["staged_out"] += len(staged_out)
            stats["lat_ms"].extend(lat)
            stats["gate_ns"].extend(gate_ns)
        time.sleep(0.2)  # GetRecords limit: 5 calls/s per shard


def egress(stats: dict, since: float, expect: int, deadline: float):
    rows = []
    its = shard_iters(name("staged"), since)
    while len(rows) < expect and time.time() < deadline:
        nxt = []
        for it in its:
            r = kin.get_records(ShardIterator=it, Limit=5000)
            rows.extend(json.loads(x["Data"]) for x in r["Records"])
            if r.get("NextShardIterator"):  # a closed (resharded) shard ends with None
                nxt.append(r["NextShardIterator"])
        its = nxt
        time.sleep(0.25)
    payouts = gates.payout_rows(rows)
    out, held = [], []
    for p in payouts:
        ok, viol, _ = gates.check("partner_payout_feed", p)
        if ok:
            out.append({"Data": json.dumps(p).encode(), "PartitionKey": p["partner_id"]})
        else:
            held.append({"Data": json.dumps({"boundary": "egress", "contract": "partner_payout_feed", "violations": viol, "record": p}).encode(), "PartitionKey": "egress"})
            for v in viol:
                stats["violations"][("egress", v["field"], v["kind"])] += 1
            stats["held_examples"].append({"payout": p, "violations": viol})
    put_all(name("payouts"), out)
    put_all(name("quarantine"), held)
    stats.update(staged_read=len(rows), payout_rows=len(payouts), payouts_sent=len(out), payouts_held=len(held))


def run(n: int, rate: int, json_out: str | None):
    stats = {"produced_labels": collections.Counter(), "violations": collections.Counter(), "put_retries": 0,
             "raw_read": 0, "ingest_quarantined": 0, "stage_quarantined": 0, "staged_out": 0,
             "lat_ms": [], "gate_ns": [], "held_examples": []}
    since = time.time() - 1
    lock, stop = threading.Lock(), threading.Event()
    workers = [threading.Thread(target=ingest_and_stage_worker, args=(it, stats, lock, stop), daemon=True) for it in shard_iters(name("raw"), since)]
    for w in workers:
        w.start()
    t0 = time.time()
    prod = threading.Thread(target=produce, args=(n, rate, stats), daemon=True)
    prod.start()
    print(f"producing {n:,} charges at {rate:,}/s into {name('raw')} ...", flush=True)
    while (stats["raw_read"] < n or prod.is_alive()) and time.time() - t0 < 600:
        time.sleep(1)
        print(f"  t={time.time() - t0:5.1f}s  produced={stats.get('produced', 0):>7,}  gated={stats['raw_read']:>7,}"
              f"  quarantined ingest={stats['ingest_quarantined']:>5,} stage={stats['stage_quarantined']:>5,}  staged={stats['staged_out']:>7,}", flush=True)
    prod.join()
    stop.set()
    gate_s = time.time() - t0
    print(f"egress: reading {stats['staged_out']:,} staged rows, rolling up partner payouts ...", flush=True)
    egress(stats, since, stats["staged_out"], time.time() + 120)
    lat = sorted(stats["lat_ms"])
    gns = sorted(stats["gate_ns"])
    q = lambda a, p: a[min(len(a) - 1, int(len(a) * p))] if a else 0
    summary = {
        "records_produced": n, "producer_rate_target": rate, "produce_seconds": round(stats["produce_s"], 1),
        "ingest_and_stage_seconds": round(gate_s, 1), "throughput_rec_per_s": round(n / gate_s, 0),
        "raw_read": stats["raw_read"], "ingest_quarantined": stats["ingest_quarantined"],
        "stage_quarantined": stats["stage_quarantined"], "staged_out": stats["staged_out"],
        "staged_read_by_egress": stats.get("staged_read"), "payout_rows": stats.get("payout_rows"),
        "payouts_sent": stats.get("payouts_sent"), "payouts_held": stats.get("payouts_held"),
        "kinesis_arrival_to_gated_ms": {"p50": round(q(lat, .5)), "p95": round(q(lat, .95)), "p99": round(q(lat, .99))},
        "check_us_per_record": {"p50": round(q(gns, .5) / 1000, 1), "p99": round(q(gns, .99) / 1000, 1), "mean": round(statistics.mean(gns) / 1000, 1) if gns else 0},
        "produced_labels": dict(stats["produced_labels"]),
        "violations": {f"{b}:{f}:{k}": c for (b, f, k), c in sorted(stats["violations"].items())},
        "held_examples": stats["held_examples"][:3], "put_retries": stats["put_retries"],
    }
    print(json.dumps(summary, indent=2))
    if json_out:
        open(json_out, "w").write(json.dumps(summary, indent=2))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["up", "run", "down"])
    ap.add_argument("--records", type=int, default=20000)
    ap.add_argument("--rate", type=int, default=1500, help="producer records/s (raw has 2 shards = 2000/s write limit)")
    ap.add_argument("--json")
    a = ap.parse_args()
    {"up": up, "down": down}.get(a.cmd, lambda: run(a.records, a.rate, a.json))()
