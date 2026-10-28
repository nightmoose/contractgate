"""LangChain AISchemaGateParser. HTTP is mocked."""

from __future__ import annotations

from typing import List

import pytest
from langchain_core.exceptions import OutputParserException
from langchain_core.output_parsers import BaseOutputParser

from aischemagate._output import AISchemaGateRejected
from aischemagate.langchain import AISchemaGateParser
from gateway import CONTRACT_UUID, fail_egress, ok_egress, recording_client


def test_parser_is_a_base_output_parser():
    assert issubclass(AISchemaGateParser, BaseOutputParser)


def test_parse_returns_the_egress_payload():
    calls: List[str] = []
    repaired = {"email": "hmac-sha256:abc", "amount": 10}
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress(repaired)),
    )
    try:
        got = parser.parse('{"email": "a@b.com", "amount": 10}')
    finally:
        parser.close()
    assert got == repaired
    assert calls == [f"POST /egress/{CONTRACT_UUID}"]


def test_parse_accepts_a_markdown_fence():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({"amount": 10})),
    )
    try:
        assert parser.parse('```json\n{"amount": 10}\n```') == {"amount": 10}
    finally:
        parser.close()


def test_structural_failure_raises_output_parser_exception():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(
            [],
            fail_egress("amount", "Field 'amount' value -3 is below minimum 0", "range_violation"),
        ),
    )
    try:
        with pytest.raises(OutputParserException, match="range_violation") as caught:
            parser.parse('{"amount": -3}')
    finally:
        parser.close()
    cause = caught.value.__cause__
    assert isinstance(cause, AISchemaGateRejected)
    assert cause.violations[0].field == "amount"
    assert caught.value.llm_output == '{"amount": -3}'


def test_invalid_json_raises_output_parser_exception():
    calls: List[str] = []
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client(calls, ok_egress({})),
    )
    try:
        with pytest.raises(OutputParserException, match="not valid JSON"):
            parser.parse("nope")
    finally:
        parser.close()
    assert calls == []


def test_contract_name_resolves_through_the_parser():
    calls: List[str] = []
    from gateway import contract_row

    parser = AISchemaGateParser(
        api_key="k",
        contract_id="extract_invoice_v1",
        client=recording_client(
            calls,
            ok_egress({"amount": 1}),
            rows=[contract_row(CONTRACT_UUID, "extract_invoice_v1")],
        ),
    )
    try:
        parser.parse('{"amount": 1}')
        parser.parse('{"amount": 2}')
    finally:
        parser.close()
    assert calls == [
        "GET /contracts",
        f"POST /egress/{CONTRACT_UUID}",
        f"POST /egress/{CONTRACT_UUID}",
    ]


def test_repr_and_dump_omit_the_api_key():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({})),
    )
    try:
        assert "api_key" not in repr(parser)
        dumped = parser.model_dump()
        assert "api_key" not in dumped
        assert "client" not in dumped
    finally:
        parser.close()


def test_format_instructions_and_type_name():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({})),
    )
    try:
        instructions = parser.get_format_instructions()
        assert "JSON object" in instructions
        assert parser._type == "aischemagate"
    finally:
        parser.close()


def test_context_manager_closes():
    parser = AISchemaGateParser(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({"amount": 1})),
    )
    with parser:
        assert parser.parse('{"amount": 1}') == {"amount": 1}
