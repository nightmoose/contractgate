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
