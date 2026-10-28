"""Egress check shared by the framework adapters. HTTP is mocked."""

from __future__ import annotations

import json
from typing import Any, Dict, List

import httpx
import pytest

from contractgate import AuthError, ContractGateError

from aischemagate._output import AISchemaGateRejected, build_gate, check_output, missing_extra
from gateway import (
    CONTRACT_UUID,
    client_for,
    contract_row,
    fail_egress,
    ok_egress,
    recording_client,
    status_for,
)


class _Stream:
    def __iter__(self) -> "_Stream":
        return self

    def __next__(self) -> Any:
        raise StopIteration


def test_check_output_returns_egress_payload_not_the_submitted_object():
    submitted = {"email": "a@b.com", "amount": 10}
    repaired = {"email": "hmac-sha256:abc", "amount": 10}
    seen: List[bytes] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request.content)
        return httpx.Response(200, json=ok_egress(repaired))

    gate = build_gate(api_key="k", contract_id=CONTRACT_UUID, client=client_for(handler))
    try:
        got = check_output(gate, submitted)
    finally:
        gate.close()
    assert got == repaired
    assert got is not submitted
    assert json.loads(seen[0]) == [submitted]


def test_check_output_posts_to_the_egress_uuid_path():
    calls: List[str] = []
    body = ok_egress({"amount": 1})
    gate = build_gate(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, body),
    )
    try:
        assert check_output(gate, {"amount": 1}) == {"amount": 1}
    finally:
        gate.close()
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


def test_contract_name_is_resolved_once_via_list_contracts():
    calls: List[str] = []
    rows = [
        contract_row("33333333-3333-4333-8333-333333333333", "other"),
        contract_row(CONTRACT_UUID, "extract_invoice_v1"),
    ]
    gate = build_gate(
        api_key="k",
        contract_id="extract_invoice_v1",
        client=recording_client(calls, ok_egress({"amount": 1}), rows=rows),
    )
    try:
        check_output(gate, {"amount": 1})
        check_output(gate, {"amount": 1})
    finally:
        gate.close()
    assert calls == [
        "GET /contracts",
        f"POST /egress/{CONTRACT_UUID}",
        f"POST /egress/{CONTRACT_UUID}",
    ]


def test_unknown_contract_name_raises_the_sdk_error():
    gate = build_gate(
        api_key="k",
        contract_id="missing",
        client=recording_client([], ok_egress({}), rows=[contract_row(CONTRACT_UUID, "extract_invoice_v1")]),
    )
    try:
        with pytest.raises(ContractGateError, match="No contract named 'missing'"):
            check_output(gate, {"amount": 1})
    finally:
        gate.close()


def test_structural_failure_raises_with_violations():
    body = fail_egress("amount", "Field 'amount' value -3 is below minimum 0", "range_violation")
    calls: List[str] = []
    gate = build_gate(api_key="k", contract_id=CONTRACT_UUID, client=recording_client(calls, body))
    try:
        with pytest.raises(AISchemaGateRejected) as caught:
            check_output(gate, {"amount": -3})
    finally:
        gate.close()
    err = caught.value
    assert err.violations[0].field == "amount"
    assert err.violations[0].kind.value == "range_violation"
    assert "range_violation" in str(err)
    assert isinstance(err, ValueError)
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]
    assert status_for(body) == 422


def test_invalid_json_does_not_call_egress():
    calls: List[str] = []
    gate = build_gate(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress({})),
    )
    try:
        with pytest.raises(AISchemaGateRejected, match="not valid JSON"):
            check_output(gate, "not json")
    finally:
        gate.close()
    assert calls == []


def test_stream_is_rejected_without_an_http_call():
    calls: List[str] = []
    gate = build_gate(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress({})),
    )
    try:
        with pytest.raises(AISchemaGateRejected, match="streaming responses are not supported"):
            check_output(gate, _Stream())
    finally:
        gate.close()
    assert calls == []


def test_version_disposition_and_dry_run_are_forwarded():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["header"] = request.headers.get("x-contract-version")
        captured["disposition"] = request.url.params.get("disposition")
        captured["dry_run"] = request.url.params.get("dry_run")
        return httpx.Response(200, json=ok_egress({"amount": 1}))

    gate = build_gate(
        api_key="k",
        contract_id=CONTRACT_UUID,
        version="1.2.0",
        disposition="tag",
        client=client_for(handler),
    )
    try:
        check_output(gate, {"amount": 1}, dry_run=True)
    finally:
        gate.close()
    assert captured == {"header": "1.2.0", "disposition": "tag", "dry_run": "true"}


def test_auth_error_propagates():
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(401, json={"error": "unauthorized"})

    gate = build_gate(api_key="k", contract_id=CONTRACT_UUID, client=client_for(handler))
    try:
        with pytest.raises(AuthError):
            check_output(gate, {"amount": 1})
    finally:
        gate.close()


def test_client_must_be_the_synchronous_client():
    with pytest.raises(TypeError, match="synchronous contractgate.Client"):
        build_gate(api_key="k", contract_id=CONTRACT_UUID, client=object())


def test_missing_extra_message_names_the_extra():
    err = missing_extra("langchain", "AISchemaGateParser")
    assert "aischemagate[langchain]" in str(err)
    assert "pip install" in str(err)


def test_markdown_fence_is_accepted():
    gate = build_gate(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({"amount": 10})),
    )
    try:
        got = check_output(gate, '```json\n{"amount": 10}\n```')
    finally:
        gate.close()
    assert got == {"amount": 10}


def test_build_gate_rejects_async_client():
    import asyncio

    from contractgate import AsyncClient

    async_client = AsyncClient(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(lambda request: httpx.Response(200)),
    )
    try:
        with pytest.raises(TypeError, match="synchronous contractgate.Client"):
            build_gate(api_key="k", contract_id=CONTRACT_UUID, client=async_client)
    finally:
        asyncio.run(async_client.aclose())
