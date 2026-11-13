#!/usr/bin/env python3
"""Hold a Glue DynamicFrame at the gate before the Redshift load.

Template only. `make demo` runs `--self-check`, which splits a canned
ingest response. It does not start Glue, write S3, or connect to Redshift.

The gate sits after the source read and before
`write_dynamic_frame` to Redshift. Rows that fail go to an S3 quarantine
prefix. Rows that pass are the only ones handed to the Redshift connection.
"""

from __future__ import annotations

import argparse
import json
import urllib.request

INGEST_BATCH_LIMIT = 1000


def split_ingest(records: list[dict], response: dict) -> tuple[list[dict], list[dict]]:
    by_index = {row["index"]: row for row in response.get("results") or []}
    landed: list[dict] = []
    quarantined: list[dict] = []
    for index, record in enumerate(records):
        row = by_index.get(index)
        if row is not None and row.get("passed"):
            landed.append(record)
            continue
        quarantined.append(
            {
                "record": record,
                "violations": (row or {}).get("violations") or [],
                "quarantine_id": None if row is None else row.get("quarantine_id"),
            }
        )
    return landed, quarantined


def post_ingest(gateway_url: str, contract_id: str, api_key: str, records: list[dict]) -> dict:
    """POST one batch. Not called by --self-check."""
    url = gateway_url.rstrip("/") + f"/v1/ingest/{contract_id}"
    request = urllib.request.Request(url, data=json.dumps(records).encode(), method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("X-Api-Key", api_key)
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read().decode())


def gate_records(
    records: list[dict], gateway_url: str, contract_id: str, api_key: str
) -> tuple[list[dict], list[dict]]:
    landed: list[dict] = []
    quarantined: list[dict] = []
    for start in range(0, len(records), INGEST_BATCH_LIMIT):
        chunk = records[start : start + INGEST_BATCH_LIMIT]
        ok, bad = split_ingest(chunk, post_ingest(gateway_url, contract_id, api_key, chunk))
        landed.extend(ok)
        quarantined.extend(bad)
    return landed, quarantined


def glue_job_shape() -> str:
    """The call order inside a Glue job. Shown here so the template has one place to read it."""
    return """
from awsglue.context import GlueContext
from pyspark.context import SparkContext

glueContext = GlueContext(SparkContext.getOrCreate())
source = glueContext.create_dynamic_frame.from_catalog(database="raw", table_name="orders")
records = [row.asDict() for row in source.toDF().toLocalIterator()]
landed, quarantined = gate_records(records, GATEWAY_URL, CONTRACT_ID, API_KEY)

# Quarantine stays off the warehouse. A Glue catalog table over this prefix
# is enough for review.
if quarantined:
    quarantine_df = glueContext.spark_session.createDataFrame(quarantined)
    quarantine_df.write.mode("append").json("s3://your-bucket/quarantine/orders/")

# Redshift receives only the rows the gate passed.
if landed:
    passed = glueContext.spark_session.createDataFrame(landed)
    passed_dyf = DynamicFrame.fromDF(passed, glueContext, "passed_orders")
    glueContext.write_dynamic_frame.from_options(
        frame=passed_dyf,
        connection_type="redshift",
        connection_options={
            "url": REDSHIFT_JDBC_URL,
            "user": REDSHIFT_USER,
            "password": REDSHIFT_PASSWORD,
            "dbtable": "orders.orders_raw",
            "redshiftTmpDir": "s3://your-bucket/redshift-tmp/",
        },
    )
"""


def self_check() -> None:
    records = [
        {"order_id": "ord_1001", "amount": 49.99},
        {"order_id": "ord_2003", "amount": 1550},
    ]
    response = {
        "results": [
            {"index": 0, "passed": True, "violations": [], "quarantine_id": None},
            {
                "index": 1,
                "passed": False,
                "violations": [
                    {
                        "field": "amount",
                        "kind": "range_violation",
                        "message": "Field 'amount' value 1550 exceeds maximum 300",
                    }
                ],
                "quarantine_id": "00000000-0000-0000-0000-000000000002",
            },
        ]
    }
    landed, quarantined = split_ingest(records, response)
    assert [row["order_id"] for row in landed] == ["ord_1001"], landed
    assert quarantined[0]["record"]["order_id"] == "ord_2003"
    assert quarantined[0]["violations"][0]["kind"] == "range_violation"
    assert "write_dynamic_frame" in glue_job_shape()
    assert "quarantine/orders" in glue_job_shape()
    print("glue/redshift self-check: landed ord_1001, quarantined ord_2003 before the load")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if not args.self_check:
        raise SystemExit("This template is not a Glue job. Pass --self-check, or import gate_records.")
    self_check()


if __name__ == "__main__":
    main()
