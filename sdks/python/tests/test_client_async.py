"""Asynchronous HTTP client tests.

Mirrors ``test_client_sync.py`` against ``AsyncClient``. ``pytest-asyncio``
is configured in ``auto`` mode (see ``pyproject.toml``) so plain
``async def`` test functions are picked up.
"""

from __future__ import annotations

import json
from typing import Any, Dict

import httpx
import pytest

from contractgate import (
    AsyncClient,
    AuthError,
    BadRequestError,
    ConflictError,
    NotFoundError,
    ValidationFailedError,
)


def _ok_body() -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 1,
        "failed": 0,
        "dry_run": False,
        "atomic": False,
        "resolved_version": "1.0",
        "version_pin_source": "default_stable",
        "results": [
            {
                "passed": True,
                "violations": [],
                "validation_us": 1,
                "forwarded": True,
                "contract_version": "1.0",
                "transformed_event": {},
            }
        ],
    }


async def test_async_ingest_basic():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content.decode())
        return httpx.Response(200, json=_ok_body())

    transport = httpx.MockTransport(handler)
    async with AsyncClient(
        base_url="https://gw", api_key="k", transport=transport
    ) as cg:
        r = await cg.ingest(contract_id="abc", events=[{"user_id": "x"}])

    assert r.passed == 1
    assert "/ingest/abc" in captured["url"]
    assert captured["body"] == [{"user_id": "x"}]


@pytest.mark.parametrize(
    ("status", "exc"),
    [
        (400, BadRequestError),
        (401, AuthError),
        (404, NotFoundError),
        (409, ConflictError),
        (422, ValidationFailedError),
    ],
)
async def test_async_error_mapping(status: int, exc: type):
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status, json={"error": "x"})

    transport = httpx.MockTransport(handler)
    async with AsyncClient(base_url="https://gw", api_key="k", transport=transport) as cg:
        with pytest.raises(exc):
            await cg.ingest(contract_id="abc", events=[{}])


async def test_async_context_manager_closes_underlying_client():
    transport = httpx.MockTransport(lambda r: httpx.Response(200, json=_ok_body()))
    cg = AsyncClient(base_url="https://gw", api_key="k", transport=transport)
    async with cg:
        await cg.ingest(contract_id="abc", events=[{}])
    # After exit the underlying httpx.AsyncClient is closed.
    assert cg._http.is_closed


# ---------------------------------------------------------------------------
# Egress
# ---------------------------------------------------------------------------


def _ok_egress_body(passed: int = 1, failed: int = 0) -> Dict[str, Any]:
    outcomes = []
    for i in range(passed):
        outcomes.append({
            "index": i,
            "passed": True,
            "violations": [],
            "validation_us": 10,
            "action": "included",
        })
    for i in range(failed):
        outcomes.append({
            "index": passed + i,
            "passed": False,
            "violations": [{"field": "amount", "message": "must be >= 0", "kind": "range_violation"}],
            "validation_us": 8,
            "action": "blocked",
        })
    return {
        "total": passed + failed,
        "passed": passed,
        "failed": failed,
        "dry_run": True,
        "disposition": "block",
        "resolved_version": "1.0",
        "payload": [{"user_id": "alice"}] * passed,
        "outcomes": outcomes,
    }


async def test_async_egress_posts_to_correct_path():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content.decode())
        return httpx.Response(200, json=_ok_egress_body(passed=1))

    async with AsyncClient(
        base_url="https://gw.example.com",
        api_key="cg_live_test",
        transport=httpx.MockTransport(handler),
    ) as cg:
        r = await cg.egress(
            contract_id="22222222-2222-2222-2222-222222222222",
            events=[{"user_id": "alice"}],
        )

    assert "/egress/22222222-2222-2222-2222-222222222222" in captured["url"]
    assert captured["body"] == [{"user_id": "alice"}]
    assert r.passed == 1
    assert r.outcomes[0].action == "included"


async def test_async_egress_dry_run_becomes_query_param():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        return httpx.Response(200, json=_ok_egress_body(passed=1))

    async with AsyncClient(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        await cg.egress(contract_id="abc", events=[{}], dry_run=True)

    assert "dry_run=true" in captured["url"]


async def test_async_egress_disposition_becomes_query_param():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        return httpx.Response(200, json=_ok_egress_body(passed=1))

    async with AsyncClient(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        await cg.egress(contract_id="abc", events=[{}], disposition="fail")

    assert "disposition=fail" in captured["url"]


async def test_async_egress_207_does_not_raise():
    """207 with partial failures must surface in outcomes, not raise."""

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(207, json=_ok_egress_body(passed=1, failed=1))

    async with AsyncClient(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler)) as cg:
        r = await cg.egress(contract_id="abc", events=[{}, {}])

    assert r.passed == 1
    assert r.failed == 1
    assert any(o.action == "blocked" for o in r.outcomes)
