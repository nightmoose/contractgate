#!/usr/bin/env python3
"""Dagster ``@asset_check`` and Prefect task guardrail for DataContractGate.

Records are validated with the Python SDK before anything is loaded into
DuckDB, MotherDuck, or Snowflake. Failures are left in quarantine. The load
receives ``transformed_event`` only.

Offline check (no Dagster, Prefect, or warehouse):

    python pipeline_guardrail.py --self-check

The SDK calls below are what a Dagster asset or a Prefect task should run.
``build_dagster_definitions`` and ``prefect_flow`` import those frameworks
only when you call them.
"""

from __future__ import annotations

import os
import sys
from typing import Any, Iterable, Sequence

DEFAULT_BASE_URL = "https://api.datacontractgate.com"


def gate_records(results: Sequence[dict[str, Any]]) -> tuple[list[Any], list[dict[str, Any]]]:
    """Split ingest ``results`` into rows safe to load and rows held in quarantine."""
    load: list[Any] = []
    held: list[dict[str, Any]] = []
    for row in results:
        if row.get("passed"):
            load.append(row.get("transformed_event"))
        else:
            held.append(
                {
                    "index": row.get("index"),
                    "quarantine_id": row.get("quarantine_id"),
                    "violations": row.get("violations") or [],
                }
            )
    return load, held


def ingest_and_gate(client: Any, contract_id: str, records: Iterable[dict[str, Any]]) -> tuple[list[Any], list[dict[str, Any]]]:
    """Validate ``records`` with ``POST /v1/ingest/{contract_id}``.

    ``client`` is a ``contractgate.Client`` or ``aischemagate.Client``. Both
    talk to DataContractGate. Failed events stay in ``response.results``;
    they are not raised.
    """
    batch = list(records)
    if not batch:
        return [], []
    response = client.ingest(contract_id=contract_id, events=batch)
    results = []
    for row in response.results:
        violations = [
            {
                "field": v.field,
                "kind": getattr(v.kind, "value", v.kind),
                "message": v.message,
            }
            for v in row.violations
        ]
        results.append(
            {
                "index": row.index,
                "passed": row.passed,
                "quarantine_id": row.quarantine_id,
                "transformed_event": row.transformed_event,
                "violations": violations,
            }
        )
    return gate_records(results)


def load_warehouse(destination: str, rows: Sequence[Any], execute) -> int:
    """Insert accepted rows. ``execute`` is the driver's ``execute`` or ``executemany``.

    ``destination`` is one of ``duckdb``, ``motherduck``, or ``snowflake``.
    The SQL is the same shape; the connection is the caller's.
    """
    if destination not in {"duckdb", "motherduck", "snowflake"}:
        raise ValueError(f"unsupported destination {destination}")
    if not rows:
        return 0
    # MotherDuck uses the DuckDB client with a ``md:`` database. Snowflake
    # uses the connector cursor. Both accept a parameter list per row.
    statement = "INSERT INTO orders_clean (order_id, email, status) VALUES (?, ?, ?)"
    if destination == "snowflake":
        statement = "INSERT INTO orders_clean (order_id, email, status) VALUES (%s, %s, %s)"
    params = [(row["order_id"], row["email"], row["status"]) for row in rows]
    execute(statement, params)
    return len(params)


def build_dagster_definitions(client: Any, contract_id: str):
    """Dagster asset that loads only gated rows, plus an asset check."""
    from dagster import AssetCheckResult, AssetCheckSeverity, Definitions, asset, asset_check

    @asset
    def orders_clean():
        raw = [{"order_id": "ord_1", "email": "ada@example.com", "status": "paid"}]
        load, held = ingest_and_gate(client, contract_id, raw)
        # Replace this with a real DuckDB / MotherDuck / Snowflake connection.
        load_warehouse("duckdb", load, lambda *_args: None)
        return {"loaded": len(load), "quarantined": len(held)}

    @asset_check(asset=orders_clean)
    def orders_contract(orders_clean):
        quarantined = orders_clean["quarantined"]
        return AssetCheckResult(
            passed=quarantined == 0,
            severity=AssetCheckSeverity.ERROR,
            metadata={"quarantined": quarantined, "loaded": orders_clean["loaded"]},
        )

    return Definitions(assets=[orders_clean], asset_checks=[orders_contract])


def prefect_flow(client: Any, contract_id: str):
    """Prefect flow: gate, then load. Quarantined rows never reach the warehouse task."""
    from prefect import flow, task

    @task
    def extract() -> list[dict[str, Any]]:
        return [{"order_id": "ord_1", "email": "ada@example.com", "status": "paid"}]

    @task
    def guard(records: list[dict[str, Any]]) -> list[Any]:
        load, _held = ingest_and_gate(client, contract_id, records)
        return load

    @task
    def load_snowflake(rows: list[Any]) -> int:
        return load_warehouse("snowflake", rows, lambda *_args: None)

    @flow(name="orders-guardrail")
    def orders() -> int:
        return load_snowflake(guard(extract()))

    return orders


class _FakeResult:
    def __init__(self, **kwargs: Any) -> None:
        self.__dict__.update(kwargs)


class _FakeViolation:
    def __init__(self, field: str, kind: str, message: str) -> None:
        self.field = field
        self.kind = kind
        self.message = message


class _FakeClient:
    def ingest(self, *, contract_id: str, events: list[dict[str, Any]]) -> Any:
        assert contract_id
        results = []
        for index, event in enumerate(events):
            if event.get("status") == "paid":
                results.append(
                    _FakeResult(
                        index=index,
                        passed=True,
                        quarantine_id=None,
                        transformed_event={**event, "email": "****"},
                        violations=[],
                    )
                )
            else:
                results.append(
                    _FakeResult(
                        index=index,
                        passed=False,
                        quarantine_id=f"quar-{index}",
                        transformed_event={**event, "email": "****"},
                        violations=[_FakeViolation("status", "enum_violation", "not in enum")],
                    )
                )
        return _FakeResult(results=results)


def _self_check() -> None:
    load, held = gate_records(
        [
            {
                "index": 0,
                "passed": True,
                "transformed_event": {"order_id": "a", "email": "****", "status": "paid"},
                "violations": [],
            },
            {
                "index": 1,
                "passed": False,
                "quarantine_id": "q-1",
                "transformed_event": {"email": "****"},
                "violations": [{"field": "status", "kind": "enum_violation"}],
            },
        ]
    )
    assert load == [{"order_id": "a", "email": "****", "status": "paid"}]
    assert held[0]["quarantine_id"] == "q-1"

    client = _FakeClient()
    load, held = ingest_and_gate(
        client,
        "11111111-1111-4111-8111-111111111111",
        [
            {"order_id": "a", "email": "ada@example.com", "status": "paid"},
            {"order_id": "b", "email": "bea@example.com", "status": "nope"},
        ],
    )
    assert len(load) == 1 and load[0]["email"] == "****"
    assert held[0]["violations"][0]["kind"] == "enum_violation"
    assert "ada@example.com" not in str(load)

    executed: list[Any] = []

    def capture(statement: str, params: list[tuple[Any, ...]]) -> None:
        executed.append((statement, params))

    assert load_warehouse("duckdb", load, capture) == 1
    assert "INSERT INTO orders_clean" in executed[0][0]
    assert executed[0][1][0][1] == "****"
    assert load_warehouse("snowflake", [], capture) == 0
    try:
        load_warehouse("postgres", load, capture)
    except ValueError as err:
        assert "unsupported" in str(err)
    else:
        raise AssertionError("unknown destination should fail")
    print("pipeline guardrail self-check passed")


def main(argv: list[str]) -> None:
    if "--self-check" in argv:
        _self_check()
        return
    from contractgate import Client

    client = Client(
        base_url=os.environ.get("CONTRACTGATE_BASE_URL", DEFAULT_BASE_URL),
        api_key=os.environ["CONTRACTGATE_API_KEY"],
    )
    records = [{"order_id": "ord_1", "email": "ada@example.com", "status": "paid"}]
    load, held = ingest_and_gate(client, os.environ["CONTRACTGATE_CONTRACT_ID"], records)
    print(f"loaded={len(load)} quarantined={len(held)}")


if __name__ == "__main__":
    main(sys.argv)
