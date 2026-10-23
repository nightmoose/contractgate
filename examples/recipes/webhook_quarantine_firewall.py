#!/usr/bin/env python3
"""Webhook quarantine firewall for DataContractGate.

A FastAPI app or a plain worker accepts a Hookdeck or Svix delivery, posts
the JSON body to ``POST /v1/ingest/{contract_id}`` with an ``Idempotency-Key``,
and forwards only ``transformed_event``. Rows that fail validation are
already in quarantine; this process does not forward them and it answers
2xx so the webhook vendor does not retry a payload that was decided.

Offline check (no gateway, no FastAPI):

    python webhook_quarantine_firewall.py --self-check

Live::

    export CONTRACTGATE_API_KEY=cg_live_...
    export CONTRACTGATE_CONTRACT_ID=<uuid>
    python webhook_quarantine_firewall.py path/to/event.json
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Mapping

DEFAULT_BASE_URL = "https://api.datacontractgate.com"


def idempotency_key(headers: Mapping[str, str]) -> str:
    """Stable key for one webhook delivery.

    Svix sends ``svix-id``. Hookdeck sends ``x-hookdeck-eventid``.
    Both are unique per delivery and short enough for the 255-character
    ``Idempotency-Key`` limit. Fall back to a caller-supplied key.
    """
    lowered = {str(k).lower(): str(v) for k, v in headers.items()}
    for name in ("svix-id", "x-hookdeck-eventid", "idempotency-key"):
        value = lowered.get(name, "").strip()
        if value:
            return value[:255]
    raise ValueError("webhook delivery has no svix-id, x-hookdeck-eventid, or Idempotency-Key")


def decide(ingest_body: Mapping[str, Any]) -> dict[str, Any]:
    """Split a ``/v1/ingest`` body into forwardable payloads and quarantine rows.

    ``transformed_event`` is the post-transform payload (PII masking included).
    The original webhook body is not returned.
    """
    forward: list[Any] = []
    quarantined: list[dict[str, Any]] = []
    for row in ingest_body.get("results") or []:
        if row.get("passed"):
            forward.append(row.get("transformed_event"))
        else:
            quarantined.append(
                {
                    "index": row.get("index"),
                    "quarantine_id": row.get("quarantine_id"),
                    "violations": row.get("violations") or [],
                }
            )
    return {"forward": forward, "quarantined": quarantined}


def post_ingest(
    *,
    base_url: str,
    api_key: str,
    contract_id: str,
    payload: Any,
    idempotency_key: str,
    opener: Any = urllib.request.urlopen,
) -> tuple[int, dict[str, Any]]:
    """POST one webhook JSON value. 200, 207, and 422 all return a batch body."""
    target = f"{base_url.rstrip('/')}/v1/ingest/{contract_id}"
    req = urllib.request.Request(
        target,
        data=json.dumps(payload).encode("utf-8"),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "Accept": "application/json",
            "X-Api-Key": api_key,
            "Idempotency-Key": idempotency_key[:255],
        },
    )
    try:
        with opener(req, timeout=30) as resp:
            status = resp.status
            raw = resp.read()
    except urllib.error.HTTPError as err:
        status = err.code
        raw = err.read()
    body = json.loads(raw.decode("utf-8"))
    if "results" not in body:
        raise RuntimeError(f"ingest HTTP {status} did not return results: {body}")
    return status, body


def create_app():
    """FastAPI app. Imported lazily so ``--self-check`` does not need FastAPI."""
    from fastapi import FastAPI, Request

    app = FastAPI(title="DataContractGate webhook firewall")
    base_url = os.environ.get("CONTRACTGATE_BASE_URL", DEFAULT_BASE_URL)
    api_key = os.environ["CONTRACTGATE_API_KEY"]
    contract_id = os.environ["CONTRACTGATE_CONTRACT_ID"]

    @app.post("/webhooks/orders")
    async def orders(request: Request) -> dict[str, Any]:
        # Verify the vendor signature before this point, e.g. Svix
        # ``Webhook(secret).verify(body, headers)`` or Hookdeck's signing secret.
        payload = json.loads(await request.body())
        _status, body = post_ingest(
            base_url=base_url,
            api_key=api_key,
            contract_id=contract_id,
            payload=payload,
            idempotency_key=idempotency_key(request.headers),
        )
        decision = decide(body)
        # 200: the delivery is decided. A 5xx would make Svix or Hookdeck retry
        # a payload that is already quarantined.
        return {"status": "accepted", **decision}

    return app


def _self_check() -> None:
    captured: dict[str, Any] = {}

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length)
            captured["path"] = self.path
            captured["idempotency"] = self.headers.get("Idempotency-Key")
            captured["api_key"] = self.headers.get("X-Api-Key")
            captured["body"] = json.loads(raw.decode("utf-8"))
            response = {
                "total": 1,
                "passed": 1,
                "failed": 0,
                "dry_run": False,
                "results": [
                    {
                        "index": 0,
                        "passed": True,
                        "violations": [],
                        "quarantine_id": None,
                        "transformed_event": {
                            "order_id": "ord_1",
                            "email": "****",
                            "status": "paid",
                        },
                    }
                ],
            }
            encoded = json.dumps(response).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def log_message(self, fmt: str, *args: Any) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    import threading

    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        key = idempotency_key({"svix-id": "msg_123", "X-Hookdeck-Eventid": "ignored"})
        assert key == "msg_123"
        assert idempotency_key({"X-Hookdeck-Eventid": "evt_9"}) == "evt_9"
        status, body = post_ingest(
            base_url=f"http://127.0.0.1:{port}",
            api_key="cg_live_test",
            contract_id="11111111-1111-4111-8111-111111111111",
            payload={"order_id": "ord_1", "email": "ada@example.com", "status": "paid"},
            idempotency_key=key,
        )
        assert status == 200, status
        assert captured["path"] == "/v1/ingest/11111111-1111-4111-8111-111111111111"
        assert captured["idempotency"] == "msg_123"
        assert captured["api_key"] == "cg_live_test"
        assert captured["body"]["email"] == "ada@example.com"
        decision = decide(body)
        assert decision["forward"] == [
            {"order_id": "ord_1", "email": "****", "status": "paid"}
        ]
        assert decision["quarantined"] == []
        assert "ada@example.com" not in json.dumps(decision["forward"])

        held = decide(
            {
                "results": [
                    {
                        "index": 0,
                        "passed": False,
                        "quarantine_id": "22222222-2222-4222-8222-222222222222",
                        "transformed_event": {"email": "****"},
                        "violations": [{"field": "status", "kind": "enum_violation"}],
                    }
                ]
            }
        )
        assert held["forward"] == []
        assert held["quarantined"][0]["quarantine_id"].startswith("22222222")
    finally:
        server.shutdown()
    print("webhook quarantine firewall self-check passed")


def main(argv: list[str]) -> None:
    if "--self-check" in argv:
        _self_check()
        return
    if len(argv) != 2:
        raise SystemExit("usage: webhook_quarantine_firewall.py --self-check | <event.json>")
    payload = json.loads(open(argv[1], encoding="utf-8").read())
    status, body = post_ingest(
        base_url=os.environ.get("CONTRACTGATE_BASE_URL", DEFAULT_BASE_URL),
        api_key=os.environ["CONTRACTGATE_API_KEY"],
        contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
        payload=payload,
        idempotency_key=os.environ.get("IDEMPOTENCY_KEY", "manual-" + os.path.basename(argv[1])),
    )
    decision = decide(body)
    print(json.dumps({"http_status": status, **decision}, indent=2))


if __name__ == "__main__":
    main(sys.argv)
