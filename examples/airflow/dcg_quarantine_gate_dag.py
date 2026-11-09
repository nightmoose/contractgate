"""
DataContractGate quarantine gate for Airflow.

Pattern: load (send X-Batch-Id = this DAG run's run_id) → quarantine check →
dbt build. The check calls GET /quarantine/summary and fails or short-circuits
when pending is above zero for that run_id.

Time-window fallback: set Variable dcg_quarantine_use_time_window to "true",
or leave run_id empty. A gateway that does not have GET /quarantine/summary
(HTTP 404) also falls back to paging GET /quarantine and keeping
status=pending rows with quarantined_at >= data_interval_start.

Requires:
  - Airflow connection `datacontractgate_api` (HTTP) with host =
    https://api.datacontractgate.com  (or your self-host gateway)
    and password = API key  (or set Variable datacontractgate_api_key)
  - Variable `dcg_quarantine_contract_ids` = JSON list of contract UUIDs
    e.g. ["11111111-1111-1111-1111-111111111111"]

The load task must send the same run id the check reads:

    headers = ingest_headers(context["run_id"], api_key)
    # headers["X-Batch-Id"] == context["run_id"]

No custom provider package — HttpHook + requests only.
"""

from __future__ import annotations

import json
from datetime import datetime, timezone
from typing import Any

import requests
from airflow import DAG
from airflow.hooks.base import BaseHook
from airflow.models import Variable
from airflow.operators.empty import EmptyOperator
from airflow.operators.python import PythonOperator, ShortCircuitOperator

CONN_ID = "datacontractgate_api"
DEFAULT_BASE = "https://api.datacontractgate.com"
PAGE_LIMIT = 100  # API clamps 1–500


def _api_key_and_base() -> tuple[str, str]:
    try:
        conn = BaseHook.get_connection(CONN_ID)
        host = (conn.host or "").rstrip("/") or DEFAULT_BASE
        if host and not host.startswith("http"):
            host = "https://" + host
        key = conn.password or Variable.get("datacontractgate_api_key", default_var="")
    except Exception:
        host = DEFAULT_BASE
        key = Variable.get("datacontractgate_api_key", default_var="")
    if not key:
        raise ValueError(
            "Set API key as password on Airflow connection "
            f"'{CONN_ID}' or Variable 'datacontractgate_api_key'"
        )
    return key, host


def _contract_ids() -> list[str]:
    raw = Variable.get("dcg_quarantine_contract_ids", default_var="[]")
    ids = json.loads(raw) if isinstance(raw, str) else raw
    if not isinstance(ids, list) or not ids:
        raise ValueError(
            "Variable dcg_quarantine_contract_ids must be a non-empty JSON list of UUIDs"
        )
    return [str(x) for x in ids]


def ingest_headers(run_id: str, api_key: str) -> dict[str, str]:
    """Headers for POST /v1/ingest or POST /egress in this DAG run.

    X-Batch-Id is the Airflow run_id. The quarantine check reads that same
    value back from GET /quarantine/summary.
    """
    return {
        "X-Api-Key": api_key,
        "X-Batch-Id": run_id,
        "Accept": "application/json",
        "Content-Type": "application/json",
    }


def fetch_pending_count_by_batch(
    contract_ids: list[str],
    batch_id: str,
    *,
    api_key: str,
    base_url: str,
) -> int:
    """Sum by_status.pending from GET /quarantine/summary for each contract.

    Raises requests.HTTPError, including 404 when the gateway has no summary
    route. The caller treats 404 as the time-window fallback.
    """
    headers = {"X-Api-Key": api_key, "Accept": "application/json"}
    pending = 0
    for contract_id in contract_ids:
        resp = requests.get(
            f"{base_url}/quarantine/summary",
            headers=headers,
            params={"contract_id": contract_id, "batch_id": batch_id},
            timeout=60,
        )
        resp.raise_for_status()
        body = resp.json()
        if not isinstance(body, dict):
            raise TypeError(f"Expected object from /quarantine/summary, got {type(body)}")
        by_status = body.get("by_status") or {}
        pending += int(by_status.get("pending") or body.get("pending") or 0)
    return pending


def fetch_pending_quarantine(
    contract_ids: list[str],
    since: datetime,
    *,
    api_key: str,
    base_url: str,
) -> list[dict[str, Any]]:
    """Paginate GET /quarantine; keep status=pending and quarantined_at >= since.

    Fallback for rows that were written without a batch_id.
    """
    if since.tzinfo is None:
        since = since.replace(tzinfo=timezone.utc)
    headers = {"X-Api-Key": api_key, "Accept": "application/json"}
    pending: list[dict[str, Any]] = []

    for contract_id in contract_ids:
        offset = 0
        while True:
            resp = requests.get(
                f"{base_url}/quarantine",
                headers=headers,
                params={
                    "contract_id": contract_id,
                    "limit": PAGE_LIMIT,
                    "offset": offset,
                },
                timeout=60,
            )
            resp.raise_for_status()
            rows = resp.json()
            if not isinstance(rows, list):
                raise TypeError(f"Expected list from /quarantine, got {type(rows)}")
            if not rows:
                break
            for row in rows:
                if row.get("status") != "pending":
                    continue
                ts = row.get("quarantined_at")
                if not ts:
                    continue
                # API returns ISO-8601; normalize Z
                when = datetime.fromisoformat(ts.replace("Z", "+00:00"))
                if when >= since:
                    pending.append(row)
            if len(rows) < PAGE_LIMIT:
                break
            offset += PAGE_LIMIT
            # Oldest-first stop: responses are newest-first; once a full page
            # is entirely older than `since`, further pages are older too.
            oldest = rows[-1].get("quarantined_at")
            if oldest:
                oldest_dt = datetime.fromisoformat(oldest.replace("Z", "+00:00"))
                if oldest_dt < since:
                    break
    return pending


def _window_start(context: dict[str, Any]) -> datetime:
    since = context.get("data_interval_start")
    if since is None:
        from datetime import timedelta

        since = datetime.now(timezone.utc) - timedelta(hours=24)
    elif getattr(since, "tzinfo", None) is None:
        since = since.replace(tzinfo=timezone.utc)
    return since


def _use_time_window(run_id: str) -> bool:
    flag = Variable.get("dcg_quarantine_use_time_window", default_var="false")
    return str(flag).strip().lower() in {"1", "true", "yes"} or not run_id


def check_quarantine_gate(**context: Any) -> bool:
    """Return True to continue downstream; False to ShortCircuit."""
    key, base = _api_key_and_base()
    ids = _contract_ids()
    run_id = str(context.get("run_id") or "")
    since = _window_start(context)

    if _use_time_window(run_id):
        pending_rows = fetch_pending_quarantine(ids, since, api_key=key, base_url=base)
        count = len(pending_rows)
        sample_ids = [p.get("id") for p in pending_rows[:50]]
        label = f"since {since.isoformat()}"
    else:
        try:
            count = fetch_pending_count_by_batch(ids, run_id, api_key=key, base_url=base)
            sample_ids = []
            label = f"batch_id {run_id}"
        except requests.HTTPError as exc:
            status = exc.response.status_code if exc.response is not None else None
            if status != 404:
                raise
            print(
                "DataContractGate: GET /quarantine/summary returned 404; "
                "falling back to the time window"
            )
            pending_rows = fetch_pending_quarantine(ids, since, api_key=key, base_url=base)
            count = len(pending_rows)
            sample_ids = [p.get("id") for p in pending_rows[:50]]
            label = f"since {since.isoformat()}"

    context["ti"].xcom_push(key="dcg_pending_count", value=count)
    context["ti"].xcom_push(key="dcg_pending_ids", value=sample_ids)
    context["ti"].xcom_push(key="dcg_batch_id", value=run_id)
    if count:
        print(
            f"DataContractGate: {count} pending quarantine row(s) "
            f"{label} for contracts {ids}"
        )
        return False
    print(f"DataContractGate: no pending quarantine rows {label}")
    return True


def fail_if_pending(**context: Any) -> None:
    """Hard-fail variant (use instead of ShortCircuit when you want red runs)."""
    ok = check_quarantine_gate(**context)
    if not ok:
        count = context["ti"].xcom_pull(key="dcg_pending_count")
        raise RuntimeError(
            f"DataContractGate quarantine gate: {count} pending row(s)"
        )


with DAG(
    dag_id="dcg_load_quarantine_then_dbt",
    start_date=datetime(2026, 1, 1),
    schedule="@daily",
    catchup=False,
    tags=["datacontractgate", "quarantine"],
    doc_md=__doc__,
) as dag:
    load = EmptyOperator(task_id="load_raw")

    # Soft gate: skip dbt when quarantine has pending rows.
    quarantine_ok = ShortCircuitOperator(
        task_id="dcg_quarantine_check",
        python_callable=check_quarantine_gate,
    )

    # Swap for this if you prefer a failed task instead of skipping:
    # quarantine_ok = PythonOperator(
    #     task_id="dcg_quarantine_check",
    #     python_callable=fail_if_pending,
    # )

    dbt_build = EmptyOperator(task_id="dbt_build")
    # Replace EmptyOperator with your BashOperator / Dbt* operator.

    load >> quarantine_ok >> dbt_build
