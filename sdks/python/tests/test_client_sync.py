"""Synchronous HTTP client tests.

Uses ``httpx.MockTransport`` so the tests run without a live gateway.
The mock asserts on URL shape, headers, body, then returns a canned
response — matches the production wire shapes from
``src/main.rs`` and ``src/ingest.rs``.
"""

from __future__ import annotations

import json
from typing import Any, Dict

import httpx
import pytest

from contractgate import (
    AuthError,
    BadRequestError,
    Client,
    ConflictError,
    NotFoundError,
    ServerError,
    ValidationFailedError,
)
from contractgate.exceptions import ConnectionError as _CGConnError


def _ok_ingest_body(passed: int = 1, failed: int = 0) -> Dict[str, Any]:
    return {
        "total": passed + failed,
        "passed": passed,
        "failed": failed,
        "dry_run": False,
        "atomic": False,
        "resolved_version": "1.0",
        "version_pin_source": "default_stable",
        "results": [
            {
                "passed": True,
                "violations": [],
                "validation_us": 42,
                "forwarded": True,
                "contract_version": "1.0",
                "transformed_event": {"user_id": "x"},
            }
            for _ in range(passed)
        ]
        + [
            {
                "passed": False,
                "violations": [
                    {"field": "user_id", "message": "bad", "kind": "pattern_mismatch"}
                ],
                "validation_us": 12,
                "forwarded": False,
                "contract_version": "1.0",
                "transformed_event": {"user_id": "BAD!"},
            }
            for _ in range(failed)
        ],
    }


# ---------------------------------------------------------------------------
# Ingest happy path
# ---------------------------------------------------------------------------


def test_ingest_posts_to_correct_path_with_headers():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["headers"] = dict(request.headers)
        captured["body"] = json.loads(request.content.decode())
        return httpx.Response(200, json=_ok_ingest_body(passed=1))

    transport = httpx.MockTransport(handler)
    with Client(
        base_url="https://gw.example.com",
        api_key="cg_live_test",
        org_id="org-uuid",
        transport=transport,
    ) as cg:
        r = cg.ingest(
            contract_id="11111111-1111-1111-1111-111111111111",
            events=[{"user_id": "alice"}],
        )

    assert "/ingest/11111111-1111-1111-1111-111111111111" in captured["url"]
    assert captured["headers"]["x-api-key"] == "cg_live_test"
    assert captured["headers"]["x-org-id"] == "org-uuid"
    assert captured["headers"]["user-agent"].startswith("contractgate-python/")
    assert captured["body"] == [{"user_id": "alice"}]
    assert r.passed == 1
    assert r.results[0].forwarded is True


def test_ingest_version_kwarg_becomes_header():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["headers"] = dict(request.headers)
        return httpx.Response(200, json=_ok_ingest_body(passed=1))

    with Client(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(handler),
    ) as cg:
        cg.ingest(contract_id="abc", events=[{}], version="2.5")

    assert captured["headers"]["x-contract-version"] == "2.5"


def test_ingest_dry_run_and_atomic_become_query_params():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        return httpx.Response(200, json=_ok_ingest_body(passed=1))

    with Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        cg.ingest(contract_id="abc", events=[{}], dry_run=True, atomic=True)

    assert "dry_run=true" in captured["url"]
    assert "atomic=true" in captured["url"]


def test_ingest_207_does_not_raise():
    """207 Multi-Status carries per-event details — must surface, not raise."""

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(207, json=_ok_ingest_body(passed=1, failed=1))

    with Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        r = cg.ingest(contract_id="abc", events=[{}, {}])

    assert r.passed == 1
    assert r.failed == 1
    assert any(not e.passed for e in r.results)


# ---------------------------------------------------------------------------
# Error mapping
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("status", "exc"),
    [
        (400, BadRequestError),
        (401, AuthError),
        (404, NotFoundError),
        (409, ConflictError),
        (422, ValidationFailedError),
        (500, ServerError),
        (503, ServerError),
    ],
)
def test_status_codes_map_to_exceptions(status: int, exc: type):
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status, json={"error": "boom"})

    with Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        with pytest.raises(exc) as ei:
            cg.ingest(contract_id="abc", events=[{}])
    assert ei.value.status == status
    assert ei.value.body == {"error": "boom"}


def test_network_error_wraps_to_connection_error():
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("dns blew up")

    with Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        with pytest.raises(_CGConnError):
            cg.ingest(contract_id="abc", events=[{}])


# ---------------------------------------------------------------------------
# Audit / contract reads
# ---------------------------------------------------------------------------


def test_audit_paginates_via_query_params():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        return httpx.Response(
            200,
            json=[
                {
                    "id": "00000000-0000-0000-0000-000000000001",
                    "contract_id": "abc",
                    "contract_version": "1.0",
                    "passed": True,
                    "violation_count": 0,
                    "violation_details": [],
                    "raw_event": {"user_id": "x"},
                    "validation_us": 50,
                    "source_ip": None,
                    "created_at": "2026-04-26T00:00:00Z",
                }
            ],
        )

    with Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        rows = cg.audit(contract_id="abc", limit=10, offset=20)

    assert "/audit" in captured["url"]
    assert "limit=10" in captured["url"]
    assert "offset=20" in captured["url"]
    assert "contract_id=abc" in captured["url"]
    assert len(rows) == 1
    assert rows[0].passed is True
