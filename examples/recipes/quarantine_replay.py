#!/usr/bin/env python3
"""Inspect quarantine groups and replay them on DataContractGate.

``GET /quarantine`` returns source rows for the caller's org. Group them by
field and violation kind, fix the contract or the producer, then
``POST /contracts/{id}/quarantine/replay`` with the ids in one group.

Offline check:

    python quarantine_replay.py --self-check

Live, after the contract or the upstream mapper has been fixed::

    export CONTRACTGATE_API_KEY=cg_live_...
    export CONTRACTGATE_CONTRACT_ID=<uuid>
    python quarantine_replay.py --group status enum_violation --target-version 1.1.0
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Mapping, Sequence

DEFAULT_BASE_URL = "https://api.datacontractgate.com"


def violation_key(detail: Mapping[str, Any]) -> tuple[str, str]:
    field = str(detail.get("field") or "")
    kind = str(detail.get("kind") or detail.get("rule") or "")
    return field, kind


def group_violations(rows: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """Group source quarantine rows by ``(field, kind)``.

    One row can sit in more than one group when it has more than one
    violation. Ids are listed once per group, in first-seen order.
    """
    groups: dict[tuple[str, str], dict[str, Any]] = {}
    for row in rows:
        row_id = str(row["id"])
        details = row.get("violation_details") or []
        if not details:
            details = [{"field": "", "kind": ""}]
        for detail in details:
            key = violation_key(detail)
            bucket = groups.setdefault(
                key,
                {"field": key[0], "kind": key[1], "count": 0, "ids": []},
            )
            if row_id not in bucket["ids"]:
                bucket["ids"].append(row_id)
                bucket["count"] += 1
    return sorted(groups.values(), key=lambda g: (-g["count"], g["field"], g["kind"]))


def replay_body(ids: Sequence[str], target_version: str | None = None) -> dict[str, Any]:
    """Body for ``POST /contracts/{id}/quarantine/replay``."""
    body: dict[str, Any] = {"ids": list(ids)}
    if target_version:
        body["target_version"] = target_version
    return body


def select_group(
    groups: Sequence[Mapping[str, Any]],
    field: str | None,
    kind: str | None,
) -> dict[str, Any]:
    matches = [
        g
        for g in groups
        if (field is None or g["field"] == field) and (kind is None or g["kind"] == kind)
    ]
    if not matches:
        raise SystemExit(f"no quarantine group for field={field!r} kind={kind!r}")
    return dict(matches[0])


def request_json(
    *,
    method: str,
    url: str,
    api_key: str,
    body: Any = None,
    opener: Any = urllib.request.urlopen,
) -> tuple[int, Any]:
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Accept": "application/json",
            "X-Api-Key": api_key,
            **({"Content-Type": "application/json"} if data is not None else {}),
        },
    )
    try:
        with opener(req, timeout=30) as resp:
            status = resp.status
            raw = resp.read()
    except urllib.error.HTTPError as err:
        status = err.code
        raw = err.read()
    return status, json.loads(raw.decode("utf-8"))


def inspect_and_replay(
    *,
    base_url: str,
    api_key: str,
    contract_id: str,
    field: str | None = None,
    kind: str | None = None,
    target_version: str | None = None,
    opener: Any = urllib.request.urlopen,
) -> dict[str, Any]:
    """List quarantine rows, pick one violation group, and replay its ids."""
    status, rows = request_json(
        method="GET",
        url=f"{base_url.rstrip('/')}/quarantine?contract_id={contract_id}&limit=500",
        api_key=api_key,
        opener=opener,
    )
    if status != 200 or not isinstance(rows, list):
        raise RuntimeError(f"GET /quarantine HTTP {status}: {rows}")
    groups = group_violations(rows)
    chosen = select_group(groups, field, kind)
    payload = replay_body(chosen["ids"], target_version)
    replay_status, replay = request_json(
        method="POST",
        url=f"{base_url.rstrip('/')}/contracts/{contract_id}/quarantine/replay",
        api_key=api_key,
        body=payload,
        opener=opener,
    )
    return {
        "groups": groups,
        "chosen": chosen,
        "replay_http_status": replay_status,
        "replay": replay,
    }


def _self_check() -> None:
    rows = [
        {
            "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "violation_details": [
                {"field": "status", "kind": "enum_violation", "message": "not in enum"},
                {"field": "total", "rule": "range", "message": "below min"},
            ],
        },
        {
            "id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            "violation_details": [
                {"field": "status", "kind": "enum_violation", "message": "not in enum"},
            ],
        },
        {
            "id": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            "violation_details": [
                {"field": "email", "kind": "pattern_mismatch", "message": "bad email"},
            ],
        },
    ]
    groups = group_violations(rows)
    status_group = select_group(groups, "status", "enum_violation")
    assert status_group["count"] == 2
    assert status_group["ids"] == [
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    ]
    assert select_group(groups, "total", None)["kind"] == "range"
    body = replay_body(status_group["ids"], "1.1.0")
    assert body == {"ids": status_group["ids"], "target_version": "1.1.0"}
    assert "target_version" not in replay_body(status_group["ids"])

    captured: dict[str, Any] = {}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            captured["get_path"] = self.path
            captured["api_key"] = self.headers.get("X-Api-Key")
            self._send(200, rows)

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            captured["post_path"] = self.path
            captured["post_body"] = json.loads(self.rfile.read(length).decode("utf-8"))
            self._send(
                200,
                {
                    "total": 2,
                    "replayed": 2,
                    "still_quarantined": 0,
                    "target_version": "1.1.0",
                    "results": [
                        {"quarantine_id": i, "outcome": "replayed"} for i in captured["post_body"]["ids"]
                    ],
                },
            )

        def _send(self, status: int, payload: Any) -> None:
            encoded = json.dumps(payload).encode("utf-8")
            self.send_response(status)
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
        contract_id = "11111111-1111-4111-8111-111111111111"
        report = inspect_and_replay(
            base_url=f"http://127.0.0.1:{port}",
            api_key="cg_live_test",
            contract_id=contract_id,
            field="status",
            kind="enum_violation",
            target_version="1.1.0",
        )
        assert captured["get_path"].startswith(f"/quarantine?contract_id={contract_id}")
        assert captured["api_key"] == "cg_live_test"
        assert captured["post_path"] == f"/contracts/{contract_id}/quarantine/replay"
        assert captured["post_body"]["target_version"] == "1.1.0"
        assert report["replay"]["replayed"] == 2
        assert report["replay_http_status"] == 200
    finally:
        server.shutdown()
    print("quarantine replay self-check passed")


def main(argv: list[str]) -> None:
    if "--self-check" in argv:
        _self_check()
        return
    field = None
    kind = None
    target = None
    args = argv[1:]
    while args:
        flag = args.pop(0)
        if flag == "--group":
            field = args.pop(0)
            kind = args.pop(0)
        elif flag == "--target-version":
            target = args.pop(0)
        else:
            raise SystemExit(f"unknown argument {flag}")
    report = inspect_and_replay(
        base_url=os.environ.get("CONTRACTGATE_BASE_URL", DEFAULT_BASE_URL),
        api_key=os.environ["CONTRACTGATE_API_KEY"],
        contract_id=os.environ["CONTRACTGATE_CONTRACT_ID"],
        field=field,
        kind=kind,
        target_version=target,
    )
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main(sys.argv)
