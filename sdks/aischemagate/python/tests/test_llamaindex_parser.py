"""LlamaIndex AISchemaGateParser. HTTP is mocked."""

from __future__ import annotations

from typing import List

import pytest
from llama_index.core.output_parsers import BaseOutputParser

from aischemagate._output import AISchemaGateRejected
from aischemagate.llamaindex import AISchemaGateParser
from gateway import CONTRACT_UUID, fail_egress, ok_egress, recording_client


def test_parser_is_a_base_output_parser():
    assert issubclass(AISchemaGateParser, BaseOutputParser)


def test_parse_returns_the_egress_payload():
    calls: List[str] = []
    repaired = {"email": "hmac-sha256:abc"}
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress(repaired)),
    )
    try:
        got = parser.parse('{"email": "a@b.com"}')
    finally:
        parser.close()
    assert got == repaired
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


def test_structural_failure_raises():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(
            [],
            fail_egress("currency", "currency must be one of USD, EUR, GBP", "enum_violation"),
        ),
    )
    try:
        with pytest.raises(AISchemaGateRejected, match="enum_violation") as caught:
            parser.parse('{"currency": "usd"}')
    finally:
        parser.close()
    assert caught.value.violations[0].field == "currency"


def test_format_keeps_the_query_and_asks_for_json():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({})),
    )
    try:
        formatted = parser.format("Extract the invoice.")
    finally:
        parser.close()
    assert formatted.startswith("Extract the invoice.")
    assert "JSON object" in formatted


def test_stream_raises_and_does_not_call_egress():
    calls: List[str] = []

    class _Stream:
        def __iter__(self) -> "_Stream":
            return self

        def __next__(self) -> str:
            return ""

    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress({})),
    )
    try:
        with pytest.raises(AISchemaGateRejected, match="streaming responses are not supported"):
            parser.parse(_Stream())  # type: ignore[arg-type]
    finally:
        parser.close()
    assert calls == []
