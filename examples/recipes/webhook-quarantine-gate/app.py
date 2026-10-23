"""Validate inbound webhooks with DataContractGate before forwarding them.

POST /v1/ingest/{contract_id} with an Idempotency-Key. On pass, forward the
PII-masked transformed_event. On failure, leave the event in DataContractGate
quarantine and do not call downstream. Replay with POST /quarantine/replay.

    python app.py --self-check
"""

from __future__ import annotations

import hashlib
import json
import os
import urllib.error
import urllib.request
from typing import Any, Callable, Mapping, Optional

BASE_URL = os.environ.get("CONTRACTGATE_BASE_URL", "https://api.datacontractgate.com")
API_KEY = os.environ.get("CONTRACTGATE_API_KEY", "")
CONTRACT_ID = os.environ.get("CONTRACTGATE_CONTRACT_ID", "")
DOWNSTREAM_URL = os.environ.get("DOWNSTREAM_URL", "")

Post = Callable[[str, Mapping[str, str], bytes], tuple[int, dict]]


def idempotency_key(headers: Mapping[str, str], body: Any) -> str:
    lowered = {k.lower(): v for k, v in headers.items()}
    for name in ("svix-id", "x-hookdeck-eventid", "x-request-id"):
        value = lowered.get(name)
        if value:
            return value
    if isinstance(body, dict) and isinstance(body.get("id"), str) and body["id"]:
        return body["id"]
    raw = json.dumps(body, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(raw).hexdigest()


def parse_ingest(status: int, payload: dict) -> dict:
    """Normalize a 200 / 207 / 422 ingest body. Other statuses raise."""
    if status not in (200, 207, 422) or "results" not in payload:
        raise RuntimeError(f"DataContractGate ingest failed: HTTP {status}")
    results = payload.get("results") or []
    first = results[0] if results else {}
    return {
        "passed": bool(first.get("passed")),
        "transformed_event": first.get("transformed_event"),
        "quarantine_id": first.get("quarantine_id"),
        "violations": first.get("violations") or [],
    }


def handle_webhook(
    body: Any,
    headers: Mapping[str, str],
    *,
    post: Post,
    ingest_url: str,
    api_key: str,
    downstream_url: str,
) -> dict:
    """Validate one webhook. Forward only when ingest passes.

    `post(url, headers, raw_body) -> (status, json)` is the HTTP call. Tests
    pass a fake. The live server uses `urllib_post`.
    """
    key = idempotency_key(headers, body)
    status, payload = post(
        ingest_url,
        {
            "Content-Type": "application/json",
            "X-Api-Key": api_key,
            "Idempotency-Key": key,
        },
        json.dumps(body).encode(),
    )
    decision = parse_ingest(status, payload)
    decision["idempotency_key"] = key
    if not decision["passed"]:
        decision["forwarded"] = False
        return decision

    masked = decision["transformed_event"]
    if not isinstance(masked, dict):
        # A pass with no transformed object has nothing safe to forward.
        decision["forwarded"] = False
        decision["passed"] = False
        return decision

    down_status, _down_body = post(
        downstream_url,
        {"Content-Type": "application/json"},
        json.dumps(masked).encode(),
    )
    if down_status < 200 or down_status >= 300:
        raise RuntimeError(f"downstream rejected the masked payload: HTTP {down_status}")
    decision["forwarded"] = True
    decision["forwarded_event"] = masked
    return decision


def urllib_post(url: str, headers: Mapping[str, str], raw: bytes) -> tuple[int, dict]:
    request = urllib.request.Request(url, data=raw, headers=dict(headers), method="POST")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            text = response.read().decode()
            return response.status, json.loads(text) if text else {}
    except urllib.error.HTTPError as err:
        text = err.read().decode()
        try:
            parsed = json.loads(text) if text else {}
        except json.JSONDecodeError:
            parsed = {"error": text}
        return err.code, parsed


def build_app():
    from fastapi import FastAPI, Request
    from fastapi.responses import JSONResponse

    app = FastAPI(title="DataContractGate webhook gate")

    @app.post("/webhooks/{source}")
    async def receive(source: str, request: Request):
        del source  # the contract, not the URL label, decides what is valid
        body = await request.json()
        if not API_KEY or not CONTRACT_ID or not DOWNSTREAM_URL:
            return JSONResponse(
                {"error": "set CONTRACTGATE_API_KEY, CONTRACTGATE_CONTRACT_ID, and DOWNSTREAM_URL"},
                status_code=500,
            )
        decision = handle_webhook(
            body,
            dict(request.headers),
            post=urllib_post,
            ingest_url=f"{BASE_URL.rstrip('/')}/v1/ingest/{CONTRACT_ID}",
            api_key=API_KEY,
            downstream_url=DOWNSTREAM_URL,
        )
        if decision["forwarded"]:
            return {"ok": True, "idempotency_key": decision["idempotency_key"]}
        # 202: stored in quarantine. Replay with POST /quarantine/replay
        # {"event_ids": [quarantine_id], "contract_id": "<uuid>"} once the
        # contract or the producer is fixed. Do not forward the raw body.
        return JSONResponse(
            {
                "ok": False,
                "quarantine_id": decision["quarantine_id"],
                "violations": decision["violations"],
            },
            status_code=202,
        )

    return app


def _maybe_app():
    try:
        return build_app()
    except ImportError:
        return None


app = _maybe_app()


def self_check() -> None:
    calls: list[tuple[str, Mapping[str, str], bytes]] = []

    def post(url: str, headers: Mapping[str, str], raw: bytes) -> tuple[int, dict]:
        calls.append((url, headers, raw))
        event = json.loads(raw)
        if url.endswith("/v1/ingest/contract-1"):
            if event.get("type") == "nope":
                return 422, {
                    "total": 1,
                    "passed": 0,
                    "failed": 1,
                    "results": [
                        {
                            "index": 0,
                            "passed": False,
                            "violations": [
                                {
                                    "field": "type",
                                    "kind": "enum_violation",
                                    "message": "type must be one of invoice.paid, customer.updated",
                                }
                            ],
                            "transformed_event": None,
                            "quarantine_id": "q-1",
                        }
                    ],
                }
            masked = dict(event)
            masked["customer_email"] = "****"
            return 200, {
                "total": 1,
                "passed": 1,
                "failed": 0,
                "results": [
                    {
                        "index": 0,
                        "passed": True,
                        "violations": [],
                        "transformed_event": masked,
                        "quarantine_id": None,
                    }
                ],
            }
        return 204, {}

    good = handle_webhook(
        {"id": "evt_1", "type": "invoice.paid", "customer_email": "a@b.co"},
        {"svix-id": "msg_1"},
        post=post,
        ingest_url="https://api.datacontractgate.com/v1/ingest/contract-1",
        api_key="cg_live_example",
        downstream_url="https://downstream.example/hooks",
    )
    assert good["forwarded"] is True
    assert good["idempotency_key"] == "msg_1"
    assert calls[0][1]["Idempotency-Key"] == "msg_1"
    assert calls[0][1]["X-Api-Key"] == "cg_live_example"
    forwarded = json.loads(calls[1][2])
    assert forwarded["customer_email"] == "****"
    assert "a@b.co" not in calls[1][2].decode()

    calls.clear()
    bad = handle_webhook(
        {"id": "evt_2", "type": "nope"},
        {},
        post=post,
        ingest_url="https://api.datacontractgate.com/v1/ingest/contract-1",
        api_key="cg_live_example",
        downstream_url="https://downstream.example/hooks",
    )
    assert bad["forwarded"] is False
    assert bad["quarantine_id"] == "q-1"
    assert bad["idempotency_key"] == "evt_2"
    assert len(calls) == 1  # ingest only; downstream was not called

    stripe_key = idempotency_key({}, {"id": "evt_stripe"})
    assert stripe_key == "evt_stripe"
    print("webhook quarantine gate self-check ok")


if __name__ == "__main__":
    import sys

    if "--self-check" in sys.argv:
        self_check()
    else:
        raise SystemExit("run: python app.py --self-check   or   uvicorn app:app")
