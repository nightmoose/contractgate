"""Tests for ``contractgate.llm`` — LLMGate / AsyncLLMGate / extract_json.

RFC-094. Covers extractor shape matrix, happy-path validate, failure path,
auto-repair three-arm matrix, max_retries clamp, streaming rejection, and
mask_prompt round-trip. One sanity test for the async mirror.
"""

from __future__ import annotations

import json
from types import SimpleNamespace
from typing import Any, Dict, List
from unittest.mock import MagicMock

import httpx
import pytest

from contractgate import AsyncClient, Client, Violation, ViolationKind
from contractgate.llm import (
    AsyncLLMGate,
    LLMGate,
    LLMGateResult,
    LLMResponseError,
    MAX_RETRIES_CAP,
    extract_json,
)


# ---------------------------------------------------------------------------
# Mock builders — LLM responses
# ---------------------------------------------------------------------------


def _openai_json_mode(obj: Dict[str, Any]) -> SimpleNamespace:
    """OpenAI ChatCompletion with ``response_format: json_object``."""
    return SimpleNamespace(
        choices=[
            SimpleNamespace(
                message=SimpleNamespace(content=json.dumps(obj), tool_calls=None)
            )
        ]
    )


def _openai_tool_call(obj: Dict[str, Any]) -> SimpleNamespace:
    """OpenAI ChatCompletion with a tool call."""
    return SimpleNamespace(
        choices=[
            SimpleNamespace(
                message=SimpleNamespace(
                    content=None,
                    tool_calls=[
                        SimpleNamespace(
                            function=SimpleNamespace(
                                name="extract", arguments=json.dumps(obj)
                            )
                        )
                    ],
                )
            )
        ]
    )


def _anthropic_tool_use(obj: Dict[str, Any]) -> SimpleNamespace:
    return SimpleNamespace(
        content=[SimpleNamespace(type="tool_use", name="extract", input=obj)]
    )


def _anthropic_text_json(obj: Dict[str, Any]) -> SimpleNamespace:
    return SimpleNamespace(
        content=[SimpleNamespace(type="text", text=json.dumps(obj))]
    )


class _StreamStub:
    """Minimal stand-in for a streaming response iterator."""

    def __iter__(self):  # pragma: no cover — never advanced
        return self

    def __next__(self):  # pragma: no cover
        raise StopIteration


# ---------------------------------------------------------------------------
# Mock builders — gateway egress / ingest responses
# ---------------------------------------------------------------------------


def _ok_egress_body(payload: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 1,
        "failed": 0,
        "dry_run": False,
        "disposition": "block",
        "resolved_version": "1.0",
        "payload": [payload],
        "outcomes": [
            {
                "index": 0,
                "passed": True,
                "violations": [],
                "validation_us": 42,
                "action": "included",
            }
        ],
    }


def _fail_egress_body(payload: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 0,
        "failed": 1,
        "dry_run": False,
        "disposition": "block",
        "resolved_version": "1.0",
        "payload": [],
        "outcomes": [
            {
                "index": 0,
                "passed": False,
                "violations": [
                    {
                        "field": "amount",
                        "message": "Field 'amount' value -3 is below minimum 0",
                        "kind": "range_violation",
                    }
                ],
                "validation_us": 42,
                "action": "blocked",
            }
        ],
    }


def _ingest_body_with_transform(transformed: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "total": 1,
        "passed": 1,
        "failed": 0,
        "dry_run": True,
        "atomic": False,
        "resolved_version": "1.0",
        "version_pin_source": "default_stable",
        "results": [
            {
                "passed": True,
                "violations": [],
                "validation_us": 10,
                "forwarded": False,
                "contract_version": "1.0",
                "transformed_event": transformed,
            }
        ],
    }


# ---------------------------------------------------------------------------
# extract_json — shape matrix
# ---------------------------------------------------------------------------


def test_extract_json_openai_json_mode():
    resp = _openai_json_mode({"user_id": "alice", "amount": 10})
    assert extract_json(resp) == {"user_id": "alice", "amount": 10}


def test_extract_json_openai_tool_call_takes_precedence_over_content():
    """When both content and tool_calls are set, tool_calls wins."""
    resp = SimpleNamespace(
        choices=[
            SimpleNamespace(
                message=SimpleNamespace(
                    content=json.dumps({"wrong": "value"}),
                    tool_calls=[
                        SimpleNamespace(
                            function=SimpleNamespace(
                                name="x", arguments=json.dumps({"right": "value"})
                            )
                        )
                    ],
                )
            )
        ]
    )
    assert extract_json(resp) == {"right": "value"}


def test_extract_json_anthropic_tool_use():
    resp = _anthropic_tool_use({"invoice_total": 42.5})
    assert extract_json(resp) == {"invoice_total": 42.5}


def test_extract_json_anthropic_text_json_fallback():
    resp = _anthropic_text_json({"status": "ok"})
    assert extract_json(resp) == {"status": "ok"}


def test_extract_json_openai_responses_output_parsed():
    resp = SimpleNamespace(output_parsed={"schema_validated": True})
    assert extract_json(resp) == {"schema_validated": True}


def test_extract_json_string_passthrough():
    assert extract_json('{"a": 1}') == {"a": 1}


def test_extract_json_string_with_markdown_fence():
    s = "```json\n{\"a\": 1, \"b\": [1, 2]}\n```"
    assert extract_json(s) == {"a": 1, "b": [1, 2]}


def test_extract_json_string_fence_no_language_tag():
    s = "```\n{\"a\": 1}\n```"
    assert extract_json(s) == {"a": 1}


def test_extract_json_dict_passthrough():
    assert extract_json({"a": 1}) == {"a": 1}


def test_extract_json_unknown_shape_raises():
    with pytest.raises(LLMResponseError, match="unrecognized response shape"):
        extract_json(object())


def test_extract_json_invalid_json_string_raises():
    with pytest.raises(LLMResponseError, match="not valid JSON"):
        extract_json("not json at all {{{")


def test_extract_json_json_array_not_object_raises():
    with pytest.raises(LLMResponseError, match="not an object"):
        extract_json("[1, 2, 3]")


def test_extract_json_streaming_rejected():
    with pytest.raises(LLMResponseError, match="streaming responses are not supported"):
        extract_json(_StreamStub())


# ---------------------------------------------------------------------------
# LLMGate — construction
# ---------------------------------------------------------------------------


def test_llmgate_requires_llm():
    with pytest.raises(ValueError, match="requires an `llm` client"):
        LLMGate(api_key="k", contract_id="c", llm=None)


def test_llmgate_max_retries_above_cap_warns_and_clamps():
    with pytest.warns(UserWarning, match="exceeds hard cap"):
        gate = LLMGate(
            api_key="k", contract_id="c", llm=MagicMock(), max_retries=99
        )
    assert gate._max_retries == MAX_RETRIES_CAP


def test_llmgate_negative_max_retries_clamps_to_zero():
    gate = LLMGate(api_key="k", contract_id="c", llm=MagicMock(), max_retries=-5)
    assert gate._max_retries == 0


# ---------------------------------------------------------------------------
# validate_only — happy + failure paths
# ---------------------------------------------------------------------------


def _client_with_egress(response_body: Dict[str, Any]) -> Client:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json=response_body)

    return Client(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(handler),
    )


def test_validate_only_happy_path():
    payload = {"user_id": "alice", "amount": 10}
    client = _client_with_egress(_ok_egress_body(payload))
    gate = LLMGate(api_key="k", contract_id="c", llm=MagicMock(), client=client)
    result = gate.validate_only(payload)
    assert result.ok is True
    assert result.payload == payload
    assert result.violations == []
    assert result.attempts == 1


def test_validate_only_failure_returns_sanitized_result():
    payload = {"user_id": "alice", "amount": -3}
    client = _client_with_egress(_fail_egress_body(payload))
    gate = LLMGate(api_key="k", contract_id="c", llm=MagicMock(), client=client)
    result = gate.validate_only(payload)
    assert result.ok is False
    assert result.payload == {}  # sanitized form — nothing passed through
    assert len(result.violations) == 1
    assert result.violations[0].field == "amount"
    assert result.violations[0].kind == ViolationKind.RANGE_VIOLATION


def test_validate_only_asserts_request_shape():
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content.decode())
        return httpx.Response(200, json=_ok_egress_body({"user_id": "a"}))

    client = Client(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(handler),
    )
    gate = LLMGate(api_key="k", contract_id="cid-xyz", llm=MagicMock(), client=client)
    gate.validate_only(_openai_json_mode({"user_id": "a"}))

    assert "/egress/cid-xyz" in captured["url"]
    assert captured["body"] == [{"user_id": "a"}]


# ---------------------------------------------------------------------------
# completion — happy path and auto-repair matrix
# ---------------------------------------------------------------------------


def test_completion_happy_path_no_repair():
    payload = {"user_id": "alice", "amount": 10}
    client = _client_with_egress(_ok_egress_body(payload))
    llm = MagicMock()
    llm.chat.completions.create = MagicMock(return_value=_openai_json_mode(payload))

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client)
    result = gate.completion(model="gpt-4o-mini", messages=[{"role": "user", "content": "x"}])

    assert result.ok is True
    assert result.attempts == 1
    assert llm.chat.completions.create.call_count == 1
    assert result.repair_hints == []


def test_completion_passes_model_and_messages_to_provider():
    payload = {"a": 1}
    client = _client_with_egress(_ok_egress_body(payload))
    llm = MagicMock()
    llm.chat.completions.create = MagicMock(return_value=_openai_json_mode(payload))

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client)
    gate.completion(
        model="gpt-4o",
        messages=[{"role": "user", "content": "hi"}],
        temperature=0.1,
    )

    call = llm.chat.completions.create.call_args
    assert call.kwargs["model"] == "gpt-4o"
    assert call.kwargs["messages"] == [{"role": "user", "content": "hi"}]
    assert call.kwargs["temperature"] == 0.1


class _ToggleTransport(httpx.MockTransport):
    """Returns a different body per request, cycling through ``bodies``."""

    def __init__(self, bodies: List[Dict[str, Any]]):
        self._bodies = iter(bodies)
        self.hits = 0
        super().__init__(self._handler)

    def _handler(self, request: httpx.Request) -> httpx.Response:
        self.hits += 1
        body = next(self._bodies)
        return httpx.Response(200, json=body)


def test_completion_auto_repair_passes_on_second_try():
    bad_payload = {"amount": -3}
    good_payload = {"amount": 10}
    transport = _ToggleTransport(
        bodies=[_fail_egress_body(bad_payload), _ok_egress_body(good_payload)]
    )
    client = Client(base_url="https://gw", api_key="k", transport=transport)

    llm = MagicMock()
    llm.chat.completions.create = MagicMock(
        side_effect=[_openai_json_mode(bad_payload), _openai_json_mode(good_payload)]
    )

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client, max_retries=2)
    result = gate.completion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "extract"}],
        auto_repair=True,
    )

    assert result.ok is True
    assert result.attempts == 2
    assert llm.chat.completions.create.call_count == 2
    assert transport.hits == 2
    assert len(result.repair_hints) == 1
    # Second call must have received the hint appended as a user message.
    second_call_messages = llm.chat.completions.create.call_args_list[1].kwargs["messages"]
    assert len(second_call_messages) == 2
    assert second_call_messages[-1]["role"] == "user"
    assert "amount" in second_call_messages[-1]["content"]


def test_completion_auto_repair_exhausts_budget():
    bad_payload = {"amount": -3}
    transport = _ToggleTransport(
        bodies=[_fail_egress_body(bad_payload)] * 3  # all failures
    )
    client = Client(base_url="https://gw", api_key="k", transport=transport)

    llm = MagicMock()
    llm.chat.completions.create = MagicMock(
        side_effect=[_openai_json_mode(bad_payload)] * 3
    )

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client, max_retries=2)
    result = gate.completion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "extract"}],
        auto_repair=True,
    )

    assert result.ok is False
    assert result.attempts == 3  # 1 initial + 2 retries
    assert llm.chat.completions.create.call_count == 3
    assert len(result.repair_hints) == 2


def test_completion_auto_repair_false_skips_retries():
    bad_payload = {"amount": -3}
    client = _client_with_egress(_fail_egress_body(bad_payload))
    llm = MagicMock()
    llm.chat.completions.create = MagicMock(return_value=_openai_json_mode(bad_payload))

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client, max_retries=2)
    result = gate.completion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "extract"}],
        auto_repair=False,
    )

    assert result.ok is False
    assert result.attempts == 1
    assert llm.chat.completions.create.call_count == 1


def test_completion_stream_kwarg_is_rejected():
    gate = LLMGate(api_key="k", contract_id="c", llm=MagicMock(), client=_client_with_egress(_ok_egress_body({})))
    with pytest.raises(LLMResponseError, match="streaming responses are not supported"):
        gate.completion(model="x", messages=[], stream=True)


# ---------------------------------------------------------------------------
# message() — Anthropic protocol
# ---------------------------------------------------------------------------


def test_message_calls_anthropic_messages_create():
    payload = {"invoice_total": 42}
    client = _client_with_egress(_ok_egress_body(payload))
    llm = MagicMock()
    llm.messages.create = MagicMock(return_value=_anthropic_tool_use(payload))

    gate = LLMGate(api_key="k", contract_id="c", llm=llm, client=client)
    result = gate.message(
        model="claude-sonnet-4-5",
        messages=[{"role": "user", "content": "extract invoice"}],
        max_tokens=512,
    )

    assert result.ok is True
    assert result.payload == payload
    call = llm.messages.create.call_args
    assert call.kwargs["model"] == "claude-sonnet-4-5"
    assert call.kwargs["max_tokens"] == 512


# ---------------------------------------------------------------------------
# mask_prompt — RFC-004 ingest(dry_run=True) wrapper
# ---------------------------------------------------------------------------


def test_mask_prompt_returns_transformed_event():
    transformed = {"email": "hmac-sha256:abc123", "user_id": "alice"}
    captured: Dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content.decode())
        return httpx.Response(200, json=_ingest_body_with_transform(transformed))

    client = Client(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(handler),
    )
    gate = LLMGate(api_key="k", contract_id="output-c", llm=MagicMock(), client=client)
    result = gate.mask_prompt(
        {"email": "alice@example.com", "user_id": "alice"},
        prompt_contract_id="prompt-c",
    )

    assert result == transformed
    assert "/ingest/prompt-c" in captured["url"]
    assert "dry_run=true" in captured["url"]


# ---------------------------------------------------------------------------
# Async mirror sanity
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_async_llmgate_validate_only_happy_path():
    payload = {"user_id": "alice"}

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json=_ok_egress_body(payload))

    async_client = AsyncClient(
        base_url="https://gw",
        api_key="k",
        transport=httpx.MockTransport(handler),
    )
    gate = AsyncLLMGate(
        api_key="k", contract_id="c", llm=MagicMock(), client=async_client
    )
    result = await gate.validate_only(payload)
    await async_client.aclose()
    assert result.ok is True
    assert result.payload == payload


@pytest.mark.asyncio
async def test_async_llmgate_requires_llm():
    with pytest.raises(ValueError, match="requires an `llm` client"):
        AsyncLLMGate(api_key="k", contract_id="c", llm=None)


# ---------------------------------------------------------------------------
# LLMGateResult shape
# ---------------------------------------------------------------------------


def test_result_is_dataclass_with_expected_fields():
    r = LLMGateResult(
        ok=True,
        payload={"a": 1},
        violations=[],
        attempts=1,
        raw_response=None,
    )
    assert r.repair_hints == []
    assert r.ok is True
