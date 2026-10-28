"""Adapters plugged into the installed LangChain, LlamaIndex, and Instructor.

HTTP never leaves the process. The gateway is a ``contractgate.Client`` on
``httpx.MockTransport``. Instructor's OpenAI client uses the same kind of
transport. These tests call the framework methods that actually invoke a
parser or response model: a LangChain runnable chain, a LlamaIndex
``PromptTemplate``, and ``instructor.from_openai`` ``create``.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

import httpx
import pytest
from pydantic import BaseModel, field_validator

from contractgate import Client

CONTRACT_UUID = "22222222-2222-4222-8222-222222222222"


def _ok(payload: Dict[str, Any]) -> Dict[str, Any]:
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
                "validation_us": 1,
                "action": "included",
            }
        ],
    }


def _fail(field: str, message: str, kind: str) -> Dict[str, Any]:
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
                "validation_us": 1,
                "action": "blocked",
            }
        ],
    }


def _row(contract_id: str, name: str) -> Dict[str, Any]:
    return {
        "id": contract_id,
        "name": name,
        "multi_stable_resolution": "strict",
        "latest_stable_version": "1.0.0",
        "version_count": 1,
    }


def _gateway(
    calls: List[str],
    body: Dict[str, Any],
    *,
    rows: Optional[List[Dict[str, Any]]] = None,
    posted: Optional[List[Any]] = None,
) -> Client:
    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(f"{request.method} {request.url.path}")
        if posted is not None and request.content:
            posted.append(json.loads(request.content))
        if request.method == "GET" and request.url.path == "/contracts":
            return httpx.Response(200, json=rows or [])
        status = 422 if body.get("failed") and not body.get("passed") else 200
        return httpx.Response(status, json=body)

    return Client(base_url="https://gw", api_key="k", transport=httpx.MockTransport(handler))


def _openai_completion(handler) -> Any:
    from openai import OpenAI

    return OpenAI(
        api_key="sk-test",
        http_client=httpx.Client(transport=httpx.MockTransport(handler)),
    )


def _chat_completion(message: Dict[str, Any], finish_reason: str) -> Dict[str, Any]:
    return {
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1,
        "model": "gpt-4o-mini",
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    }


# ---------------------------------------------------------------------------
# LangChain
# ---------------------------------------------------------------------------


def test_langchain_chain_returns_the_egress_payload():
    """LCEL calls ``BaseOutputParser.invoke``, which calls ``parse``."""
    from langchain_core.messages import AIMessage
    from langchain_core.output_parsers import BaseOutputParser
    from langchain_core.prompts import ChatPromptTemplate
    from langchain_core.runnables import RunnableLambda

    from aischemagate.langchain import AISchemaGateParser

    assert issubclass(AISchemaGateParser, BaseOutputParser)

    calls: List[str] = []
    posted: List[Any] = []
    repaired = {"email": "hmac-sha256:abc", "amount": 10}
    parser = AISchemaGateParser(
        api_key="k",
        contract_id="extract_invoice_v1",
        client=_gateway(
            calls,
            _ok(repaired),
            rows=[_row(CONTRACT_UUID, "extract_invoice_v1")],
            posted=posted,
        ),
    )
    prompt = ChatPromptTemplate.from_template("Extract {topic} as JSON.")

    def chat_model(_prompt: Any) -> AIMessage:
        return AIMessage(content='{"email": "a@b.com", "amount": 10}')

    chain = prompt | RunnableLambda(chat_model) | parser
    try:
        got = chain.invoke({"topic": "the invoice"})
    finally:
        parser.close()

    assert got == repaired
    assert calls == ["GET /contracts", f"POST /egress/{CONTRACT_UUID}"]
    assert posted[-1] == [{"email": "a@b.com", "amount": 10}]


def test_langchain_chain_raises_output_parser_exception():
    from langchain_core.exceptions import OutputParserException
    from langchain_core.messages import AIMessage
    from langchain_core.prompts import ChatPromptTemplate
    from langchain_core.runnables import RunnableLambda

    from aischemagate._output import AISchemaGateRejected
    from aischemagate.langchain import AISchemaGateParser

    calls: List[str] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(
            calls,
            _fail("amount", "Field 'amount' value -3 is below minimum 0", "range_violation"),
        ),
    )
    prompt = ChatPromptTemplate.from_template("Extract {topic} as JSON.")
    chain = prompt | RunnableLambda(lambda _prompt: AIMessage(content='{"amount": -3}')) | parser
    try:
        with pytest.raises(OutputParserException, match="range_violation") as caught:
            chain.invoke({"topic": "the invoice"})
    finally:
        parser.close()

    assert type(caught.value) is OutputParserException
    assert isinstance(caught.value.__cause__, AISchemaGateRejected)
    assert caught.value.llm_output == '{"amount": -3}'
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


def test_langchain_parse_rejects_a_stream_without_http():
    from langchain_core.exceptions import OutputParserException

    from aischemagate.langchain import AISchemaGateParser

    calls: List[str] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(calls, _ok({})),
    )

    def tokens():
        yield "{"
        yield "}"

    try:
        with pytest.raises(OutputParserException, match="streaming responses are not supported"):
            parser.parse(tokens())  # type: ignore[arg-type]
    finally:
        parser.close()
    assert calls == []


def test_langchain_invoke_accepts_a_plain_string():
    from aischemagate.langchain import AISchemaGateParser

    calls: List[str] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(calls, _ok({"amount": 1})),
    )
    try:
        assert parser.invoke('{"amount": 1}') == {"amount": 1}
    finally:
        parser.close()
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


# ---------------------------------------------------------------------------
# LlamaIndex
# ---------------------------------------------------------------------------


def test_llamaindex_prompt_template_formats_and_parse_posts_egress():
    from llama_index.core.output_parsers import BaseOutputParser
    from llama_index.core.prompts import PromptTemplate

    from aischemagate.llamaindex import AISchemaGateParser

    assert issubclass(AISchemaGateParser, BaseOutputParser)

    calls: List[str] = []
    posted: List[Any] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id="extract_invoice_v1",
        client=_gateway(
            calls,
            _ok({"email": "hmac-sha256:abc"}),
            rows=[_row(CONTRACT_UUID, "extract_invoice_v1")],
            posted=posted,
        ),
    )
    prompt = PromptTemplate("Extract {q} as JSON.", output_parser=parser)
    try:
        formatted = prompt.format(q="the invoice")
        messages = prompt.format_messages(q="the invoice")
        got = parser.parse('{"email": "a@b.com"}')
    finally:
        parser.close()

    assert formatted.startswith("Extract the invoice as JSON.")
    assert "JSON object" in formatted
    assert messages[0].content is not None
    assert "Extract the invoice as JSON." in messages[0].content
    assert "JSON object" in messages[0].content
    assert got == {"email": "hmac-sha256:abc"}
    assert calls == [
        "GET /contracts",
        f"POST /egress/{CONTRACT_UUID}",
    ]
    assert posted[-1] == [{"email": "a@b.com"}]


def test_llamaindex_parse_raises_on_reject():
    from aischemagate._output import AISchemaGateRejected
    from aischemagate.llamaindex import AISchemaGateParser

    calls: List[str] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(
            calls,
            _fail("currency", "currency must be one of USD, EUR, GBP", "enum_violation"),
        ),
    )
    try:
        with pytest.raises(AISchemaGateRejected, match="enum_violation") as caught:
            parser.parse('{"currency": "usd"}')
    finally:
        parser.close()
    assert caught.value.violations[0].field == "currency"
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


# ---------------------------------------------------------------------------
# Instructor
# ---------------------------------------------------------------------------


def test_instructor_json_mode_validates_the_repaired_payload():
    """``create`` parses with ``model_validate_json``; the hook runs first."""
    import instructor

    from aischemagate.instructor import AISchemaGateHook

    calls: List[str] = []
    posted: List[Any] = []
    seen: List[str] = []
    hook = AISchemaGateHook(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(
            calls,
            _ok({"total": 9.0, "currency": "EUR"}),
            posted=posted,
        ),
    )

    @hook.guard
    class Invoice(BaseModel):
        total: float
        currency: str

        @field_validator("currency")
        @classmethod
        def _track(cls, value: str) -> str:
            seen.append(value)
            return value

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        assert body["response_format"] == {"type": "json_object"}
        return httpx.Response(
            200,
            json=_chat_completion(
                {"role": "assistant", "content": '{"total": 1, "currency": "USD"}'},
                "stop",
            ),
        )

    llm = instructor.from_openai(_openai_completion(handler), mode=instructor.Mode.JSON)
    try:
        invoice = llm.chat.completions.create(
            model="gpt-4o-mini",
            response_model=Invoice,
            messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
            max_retries=0,
        )
    finally:
        hook.close()

    assert invoice.total == 9.0
    assert invoice.currency == "EUR"
    assert seen == ["EUR"]
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]
    assert posted[-1] == [{"total": 1, "currency": "USD"}]


def test_instructor_tools_mode_checks_structure_before_field_validation():
    import instructor
    from instructor.core import InstructorRetryException

    from aischemagate.instructor import AISchemaGateHook

    calls: List[str] = []
    seen: List[Any] = []
    hook = AISchemaGateHook(
        api_key="k",
        contract_id="invoice_extract",
        client=_gateway(
            calls,
            _fail("total", "total must be >= 0", "range_violation"),
            rows=[_row(CONTRACT_UUID, "invoice_extract")],
        ),
    )

    @hook.guard
    class Invoice(BaseModel):
        total: float
        currency: str

        @field_validator("total", mode="before")
        @classmethod
        def _track(cls, value: Any) -> Any:
            seen.append(value)
            return value

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        name = body["tools"][0]["function"]["name"]
        assert name == "Invoice"
        arguments = '{"total": -1, "currency": "usd"}'
        message = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                }
            ],
        }
        return httpx.Response(200, json=_chat_completion(message, "tool_calls"))

    llm = instructor.from_openai(_openai_completion(handler))
    try:
        with pytest.raises(InstructorRetryException, match="range_violation") as caught:
            llm.chat.completions.create(
                model="gpt-4o-mini",
                response_model=Invoice,
                messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
                max_retries=0,
            )
    finally:
        hook.close()

    assert seen == []
    assert "valid float" not in str(caught.value).lower()
    assert calls == ["GET /contracts", f"POST /egress/{CONTRACT_UUID}"]
    attempt = caught.value.failed_attempts[0].exception
    assert type(attempt).__name__ == "ValidationError"


def test_instructor_tools_mode_returns_the_repaired_model():
    import instructor

    from aischemagate.instructor import AISchemaGateHook

    calls: List[str] = []
    hook = AISchemaGateHook(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=_gateway(calls, _ok({"total": 4.5, "currency": "GBP"})),
    )

    @hook.guard
    class Invoice(BaseModel):
        total: float
        currency: str

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        name = body["tools"][0]["function"]["name"]
        message = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": '{"total": 1, "currency": "USD"}',
                    },
                }
            ],
        }
        return httpx.Response(200, json=_chat_completion(message, "tool_calls"))

    llm = instructor.from_openai(_openai_completion(handler))
    try:
        invoice = llm.chat.completions.create(
            model="gpt-4o-mini",
            response_model=Invoice,
            messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
            max_retries=0,
        )
    finally:
        hook.close()

    assert invoice.total == 4.5
    assert invoice.currency == "GBP"
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]
