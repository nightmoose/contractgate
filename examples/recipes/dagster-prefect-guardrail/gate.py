"""Gate batch rows with DataContractGate before a warehouse insert.

Passing rows are the PII-masked `transformed_event` values from
`contractgate.Client.ingest` (`POST /v1/ingest/{contract_id}`). Failing rows
are not inserted. They are already in DataContractGate quarantine; replay
them with POST /quarantine/replay.

    python gate.py --self-check
"""

from __future__ import annotations

import os
import sqlite3
from typing import Any, Iterable, Sequence


def open_client():
    """Live DataContractGate client. Imported here so --self-check stays stdlib."""
    import contractgate

    return contractgate.Client(
        base_url=os.environ.get("CONTRACTGATE_BASE_URL", "https://api.datacontractgate.com"),
        api_key=os.environ["CONTRACTGATE_API_KEY"],
    )


def gate_batch(records: Sequence[dict], *, client: Any, contract_id: str) -> dict:
    """Validate one batch. Returns accepted rows and the rejected summaries.

    `client` is a `contractgate.Client` (or a test double with `ingest`).
    """
    if not records:
        return {"accepted": [], "rejected": []}
    response = client.ingest(contract_id=contract_id, events=list(records))
    accepted: list[dict] = []
    rejected: list[dict] = []
    results = list(response.results)
    if len(results) != len(records):
        raise RuntimeError(
            f"DataContractGate returned {len(results)} results for {len(records)} records"
        )
    for result in results:
        if result.passed and isinstance(result.transformed_event, dict):
            accepted.append(result.transformed_event)
            continue
        violations = []
        for violation in result.violations or []:
            violations.append(
                {
                    "field": getattr(violation, "field", None),
                    "kind": getattr(violation, "kind", None),
                    "message": getattr(violation, "message", None),
                }
            )
        rejected.append(
            {
                "quarantine_id": result.quarantine_id,
                "violations": violations,
            }
        )
    return {"accepted": accepted, "rejected": rejected}


def chunks(records: Sequence[dict], batch_size: int) -> Iterable[Sequence[dict]]:
    if batch_size < 1:
        raise ValueError("batch_size must be >= 1")
    for start in range(0, len(records), batch_size):
        yield records[start : start + batch_size]


def run_batch(
    client: Any,
    records: Sequence[dict],
    *,
    contract_id: str,
    database: str,
    table: str = "orders",
    batch_size: int = 500,
) -> dict:
    """Micro-batch ingest, then materialize only the rows that passed."""
    accepted: list[dict] = []
    rejected: list[dict] = []
    for batch in chunks(records, batch_size):
        gated = gate_batch(batch, client=client, contract_id=contract_id)
        accepted.extend(gated["accepted"])
        rejected.extend(gated["rejected"])
    materialize(accepted, database=database, table=table)
    return {"accepted": len(accepted), "rejected": rejected}


def materialize(rows: Sequence[dict], *, database: str, table: str) -> None:
    """Insert `rows` into DuckDB, MotherDuck, or (for the self-check) sqlite.

    `database` is a DuckDB path or a MotherDuck URL such as `md:analytics`.
    When the `duckdb` package is missing, a `sqlite:` prefix uses the stdlib
    driver so the gate can be exercised without warehouse credentials.
    """
    if database.startswith("sqlite:"):
        _sqlite_insert(database.removeprefix("sqlite:"), table, rows)
        return
    import duckdb

    con = duckdb.connect(database)
    try:
        _duckdb_insert(con, table, rows)
    finally:
        con.close()


def _sqlite_insert(path: str, table: str, rows: Sequence[dict]) -> None:
    con = sqlite3.connect(path if path else ":memory:")
    try:
        _sql_insert(con, table, rows, param="?")
        con.commit()
    finally:
        con.close()


def _duckdb_insert(con: Any, table: str, rows: Sequence[dict]) -> None:
    _sql_insert(con, table, rows, param="?")


def _sql_insert(con: Any, table: str, rows: Sequence[dict], *, param: str) -> None:
    if not rows:
        con.execute(f'CREATE TABLE IF NOT EXISTS "{table}" (order_id VARCHAR)')
        return
    columns = list(rows[0].keys())
    for row in rows[1:]:
        for key in row:
            if key not in columns:
                columns.append(key)
    col_sql = ", ".join(f'"{name}" VARCHAR' for name in columns)
    con.execute(f'CREATE TABLE IF NOT EXISTS "{table}" ({col_sql})')
    placeholders = ", ".join(param for _ in columns)
    names = ", ".join(f'"{name}"' for name in columns)
    sql = f'INSERT INTO "{table}" ({names}) VALUES ({placeholders})'
    con.executemany(sql, [tuple(str(row.get(name, "")) for name in columns) for row in rows])


def dagster_asset_body(
    records: Sequence[dict],
    *,
    client: Any,
    contract_id: str,
    database: str,
    table: str = "orders",
    batch_size: int = 500,
) -> dict:
    """Body of a Dagster asset. See README for the `@asset` wrapper."""
    return run_batch(
        client,
        records,
        contract_id=contract_id,
        database=database,
        table=table,
        batch_size=batch_size,
    )


def prefect_flow_body(
    records: Sequence[dict],
    *,
    client: Any,
    contract_id: str,
    database: str,
    table: str = "orders",
    batch_size: int = 500,
) -> dict:
    """Body of a Prefect flow. See README for the `@flow` wrapper."""
    return run_batch(
        client,
        records,
        contract_id=contract_id,
        database=database,
        table=table,
        batch_size=batch_size,
    )


class _Violation:
    def __init__(self, field: str, kind: str, message: str) -> None:
        self.field = field
        self.kind = kind
        self.message = message


class _Result:
    def __init__(self, passed: bool, event: dict | None, quarantine_id: str | None, violations: list) -> None:
        self.passed = passed
        self.transformed_event = event
        self.quarantine_id = quarantine_id
        self.violations = violations


class _Response:
    def __init__(self, results: list) -> None:
        self.results = results


class _FakeClient:
    def ingest(self, *, contract_id: str, events: list) -> _Response:
        del contract_id
        results = []
        for event in events:
            if event.get("currency") not in {"USD", "EUR", "GBP"} or event.get("amount", 0) < 0:
                results.append(
                    _Result(
                        False,
                        None,
                        "q-" + str(event.get("order_id")),
                        [_Violation("currency", "enum_violation", "currency is not an allowed value")],
                    )
                )
            else:
                masked = dict(event)
                results.append(_Result(True, masked, None, []))
        return _Response(results)


def self_check() -> None:
    records = [
        {"order_id": "ORD-000001", "amount": 10, "currency": "USD"},
        {"order_id": "ORD-000002", "amount": 4, "currency": "JPY"},
        {"order_id": "ORD-000003", "amount": -1, "currency": "EUR"},
        {"order_id": "ORD-000004", "amount": 8, "currency": "GBP"},
    ]
    summary = run_batch(
        _FakeClient(),
        records,
        contract_id="orders-contract",
        database="sqlite:",
        table="orders",
        batch_size=2,
    )
    assert summary["accepted"] == 2, summary
    assert len(summary["rejected"]) == 2
    assert summary["rejected"][0]["quarantine_id"] == "q-ORD-000002"
    assert summary["rejected"][0]["violations"][0]["kind"] == "enum_violation"

    # In-memory sqlite cannot be reopened. Re-run against a temp file and read it.
    import tempfile

    tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
    path = tmp.name
    tmp.close()
    try:
        summary = run_batch(
            _FakeClient(),
            records,
            contract_id="orders-contract",
            database=f"sqlite:{path}",
            table="orders",
            batch_size=2,
        )
        con = sqlite3.connect(path)
        try:
            stored = [row[0] for row in con.execute('SELECT order_id FROM orders ORDER BY order_id')]
        finally:
            con.close()
    finally:
        os.unlink(path)
    assert stored == ["ORD-000001", "ORD-000004"], stored
    assert summary["accepted"] == 2

    dagster = dagster_asset_body(
        records[:1],
        client=_FakeClient(),
        contract_id="orders-contract",
        database="sqlite:",
    )
    assert dagster["accepted"] == 1
    prefect = prefect_flow_body(
        [],
        client=_FakeClient(),
        contract_id="orders-contract",
        database="sqlite:",
    )
    assert prefect["accepted"] == 0
    print("batch guardrail self-check ok")


if __name__ == "__main__":
    import sys

    if "--self-check" in sys.argv:
        self_check()
    else:
        raise SystemExit("run: python gate.py --self-check")
