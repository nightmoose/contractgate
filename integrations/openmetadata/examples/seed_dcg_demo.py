"""Create a clearly named test contract in DCG and push passing/failing events.

    DCG_API_URL=... DCG_API_KEY=... python examples/seed_dcg_demo.py --good 95 --bad 5
Prints the contract id. Delete it afterwards with --delete <id>.
"""
import argparse
import json
import os
import sys
import uuid

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from dcg_openmetadata import http  # noqa: E402

NAME = "om_integration_test_orders"
YAML = """version: "1.0.0"
name: "%s"
description: "TEST - OpenMetadata integration demo. Safe to delete."
x-openmetadata-table: "dcg_demo.analytics.public.orders"
ontology:
  entities:
    - name: order_id
      type: string
      required: true
      pattern: "^ord_[a-z0-9]+$"
    - name: customer_id
      type: string
      required: true
    - name: amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP"]
    - name: created_at
      type: integer
      required: true
glossary:
  - field: amount
    description: "Order total"
    constraints: "must be non-negative"
metrics:
  - name: revenue
    formula: "sum(amount)"
"""


def event(ok: bool) -> dict:
    e = {"order_id": "ord_" + uuid.uuid4().hex[:10], "customer_id": "cus_1",
         "amount": 42.5, "currency": "USD", "created_at": 1790000000}
    if not ok:
        e.update(amount=-1, currency="JPY")
    return e


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--good", type=int, default=0)
    ap.add_argument("--bad", type=int, default=0)
    ap.add_argument("--contract-id", help="reuse an existing contract")
    ap.add_argument("--delete", metavar="ID")
    a = ap.parse_args()
    base = os.environ["DCG_API_URL"].rstrip("/")
    hdr = {"x-api-key": os.environ["DCG_API_KEY"]} if os.environ.get("DCG_API_KEY") else {}
    if os.environ.get("DCG_ORG_ID"):  # local dev gateway only
        hdr["x-org-id"] = os.environ["DCG_ORG_ID"]
    if a.delete:
        print(http.request("DELETE", f"{base}/contracts/{a.delete}", hdr) or "deleted")
        return
    cid = a.contract_id
    if not cid:
        c = http.request("POST", f"{base}/contracts", hdr,
                         {"name": NAME, "description": "TEST - OpenMetadata integration demo "
                          "(openmetadata-table: dcg_demo.analytics.public.orders)",
                          "yaml_content": YAML % NAME})
        cid = c["id"]
        http.request("POST", f"{base}/contracts/{cid}/versions/1.0.0/promote", hdr, {})
    events = [event(True) for _ in range(a.good)] + [event(False) for _ in range(a.bad)]
    if events:
        r = http.request("POST", f"{base}/ingest/{cid}", hdr, events)
        print(json.dumps({k: r.get(k) for k in ("total", "passed", "failed", "quarantined")
                          if isinstance(r, dict)}), file=sys.stderr)
    print(cid)


if __name__ == "__main__":
    main()
