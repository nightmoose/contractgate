"""Mock gateway responses for adapter tests. No live network and no live keys."""

from __future__ import annotations

from typing import Any, Callable, Dict, List

import httpx

from contractgate import Client

CONTRACT_UUID = "11111111-1111-4111-8111-111111111111"


def ok_egress(payload: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 1,
        "failed": 0,
        "dry_run": False,
        "disposition": "block",
        "resolved_version": "1.0.0",
        "payload": [payload],
        "outcomes": [
            {
                "index": 0,
                "passed": True,
                "violations": [],
                "validation_us": 10,
                "action": "included",
            }
        ],
    }


def fail_egress(field: str, message: str, kind: str) -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 0,
        "failed": 1,
        "dry_run": False,
        "disposition": "block",
        "resolved_version": "1.0.0",
        "payload": [],
        "outcomes": [
            {
                "index": 0,
                "passed": False,
                "violations": [{"field": field, "message": message, "kind": kind}],
                "validation_us": 10,
                "action": "blocked",
            }
        ],
    }


def contract_row(contract_id: str, name: str) -> Dict[str, Any]:
    return {
        "id": contract_id,
        "name": name,
        "multi_stable_resolution": "strict",
        "latest_stable_version": "1.0.0",
        "version_count": 1,
    }


def status_for(body: Dict[str, Any]) -> int:
    if body.get("failed", 0) > 0 and body.get("passed", 0) == 0:
        return 422
    return 200


def client_for(handler: Callable[[httpx.Request], httpx.Response]) -> Client:
    return Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler))


def recording_client(
    calls: List[str],
    body: Dict[str, Any],
    *,
    rows: List[Dict[str, Any]] | None = None,
) -> Client:
    """Record ``METHOD path`` and serve ``body`` for egress."""

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(f"{request.method} {request.url.path}")
        if request.method == "GET" and request.url.path == "/contracts":
            return httpx.Response(200, json=rows or [])
        return httpx.Response(status_for(body), json=body)

    return client_for(handler)
