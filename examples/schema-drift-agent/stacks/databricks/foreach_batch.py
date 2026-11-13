#!/usr/bin/env python3
"""Gate an Auto Loader micro-batch before it is written to Delta.

Template for Azure Databricks. `make demo` runs `--self-check` only.
That check splits a canned `POST /v1/ingest/{contract_id}` response into
rows that would land and rows that would be quarantined. It does not
start Spark, call a gateway, or write a Delta table.

Drop this next to a notebook or a job and pass a real gateway URL.
The check has to finish before `saveAsTable` on the landing table.
Failed records go only to the quarantine table.
"""

from __future__ import annotations

import argparse
import json
import urllib.request

INGEST_BATCH_LIMIT = 1000


def split_ingest(records: list[dict], response: dict) -> tuple[list[dict], list[dict]]:
    """Split one ingest response. Indexes are positions inside this request."""
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
                **record,
                "_gate_passed": False,
                "_gate_violations": json.dumps((row or {}).get("violations") or []),
                "_gate_quarantine_id": None if row is None else row.get("quarantine_id"),
            }
        )
    return landed, quarantined


def post_ingest(gateway_url: str, contract_id: str, api_key: str, records: list[dict]) -> dict:
    """POST one batch to the gateway. Not called by --self-check."""
    url = gateway_url.rstrip("/") + f"/v1/ingest/{contract_id}"
    request = urllib.request.Request(
        url,
        data=json.dumps(records).encode(),
        method="POST",
    )
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


def foreach_batch(
    batch_df,
    batch_id: int,
    *,
    gateway_url: str,
    contract_id: str,
    api_key: str,
    landing_table: str,
    quarantine_table: str,
) -> None:
    """Auto Loader hook. Write passed rows to Delta, and only those rows.

    (spark.readStream.format("cloudFiles")
        .option("cloudFiles.format", "json")
        .load(source_path)
        .writeStream
        .foreachBatch(lambda df, bid: foreach_batch(df, bid, gateway_url=..., ...))
        .option("checkpointLocation", checkpoint)
        .start())
    """
    rows = [row.asDict(recursive=True) for row in batch_df.toLocalIterator()]
    landed, quarantined = gate_records(rows, gateway_url, contract_id, api_key)
    spark = batch_df.sparkSession
    if landed:
        (
            spark.createDataFrame(landed)
            .write.format("delta")
            .mode("append")
            .saveAsTable(landing_table)
        )
    if quarantined:
        for record in quarantined:
            record["_gate_batch_id"] = batch_id
        (
            spark.createDataFrame(quarantined)
            .write.format("delta")
            .mode("append")
            .saveAsTable(quarantine_table)
        )


def self_check() -> None:
    records = [
        {"order_id": "ord_1001", "amount": 49.99},
        {"order_id": "ord_2004", "amount": 25000},
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
                        "message": "Field 'amount' value 25000 exceeds maximum 300",
                    }
                ],
                "quarantine_id": "00000000-0000-0000-0000-000000000001",
            },
        ]
    }
    landed, quarantined = split_ingest(records, response)
    assert [row["order_id"] for row in landed] == ["ord_1001"], landed
    assert len(quarantined) == 1, quarantined
    assert quarantined[0]["order_id"] == "ord_2004"
    assert quarantined[0]["_gate_passed"] is False
    assert "range_violation" in quarantined[0]["_gate_violations"]
    assert "_gate_passed" not in landed[0]
    print("databricks foreachBatch self-check: landed ord_1001, quarantined ord_2004")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if not args.self_check:
        raise SystemExit("This template is not a job runner. Pass --self-check, or import foreach_batch.")
    self_check()


if __name__ == "__main__":
    main()
