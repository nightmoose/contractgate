"""
DataContractGate quarantine gate for Airflow.

Pattern: load → quarantine check → dbt build.
The check calls GET /quarantine (paginated), keeps rows with status=pending
and quarantined_at >= data_interval_start, and ShortCircuits (or fails) when
pending_count > 0.

Requires:
  - Airflow connection `datacontractgate_api` (HTTP) with host =
    https://api.datacontractgate.com  (or your self-host gateway)
    and password = API key  (or set Variable datacontractgate_api_key)
  - Variable `dcg_quarantine_contract_ids` = JSON list of contract UUIDs
    e.g. ["11111111-1111-1111-1111-111111111111"]

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


def fetch_pending_quarantine(
    contract_ids: list[str],
    since: datetime,
    *,
    api_key: str,
    base_url: str,
) -> list[dict[str, Any]]:
    """Paginate GET /quarantine; keep status=pending and quarantined_at >= since."""
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


def check_quarantine_gate(**context: Any) -> bool:
    """Return True to continue downstream; False to ShortCircuit."""
    key, base = _api_key_and_base()
    ids = _contract_ids()
    # Prefer the scheduled interval start; fall back to 24h lookback.
    since = context.get("data_interval_start")
    if since is None:
        from datetime import timedelta

        since = datetime.now(timezone.utc) - timedelta(hours=24)
    elif getattr(since, "tzinfo", None) is None:
        since = since.replace(tzinfo=timezone.utc)

    pending = fetch_pending_quarantine(ids, since, api_key=key, base_url=base)
    context["ti"].xcom_push(key="dcg_pending_count", value=len(pending))
    context["ti"].xcom_push(
        key="dcg_pending_ids",
        value=[p.get("id") for p in pending[:50]],
    )
    if pending:
        print(
            f"DataContractGate: {len(pending)} pending quarantine row(s) "
            f"since {since.isoformat()} for contracts {ids}"
        )
        return False
    print("DataContractGate: no pending quarantine rows in window")
    return True


def fail_if_pending(**context: Any) -> None:
    """Hard-fail variant (use instead of ShortCircuit when you want red runs)."""
    ok = check_quarantine_gate(**context)
    if not ok:
        count = context["ti"].xcom_pull(key="dcg_pending_count")
        raise RuntimeError(
            f"DataContractGate quarantine gate: {count} pending row(s) in window"
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
