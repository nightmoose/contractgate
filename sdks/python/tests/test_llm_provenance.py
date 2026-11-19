"""Repair-loop provenance check (``contractgate.provenance`` + ``LLMGate``).

A repaired answer must not carry identifiers or amounts that are absent from
the source text. These tests run the real local validator through an
in-process egress stand-in, so ``null`` handling follows the contract.
"""

from __future__ import annotations

import asyncio
import json
from types import SimpleNamespace
from typing import Any, Dict, List

import pytest

from contractgate import Contract, EgressOutcome, EgressResponse, ViolationKind
from contractgate.llm import AsyncLLMGate, LLMGate
from contractgate.provenance import (
    ProvenanceCheck,
    coerce_config,
    is_checked_field,
    source_text_from_messages,
    unsupported_fields,
    value_in_source,
)
from contractgate.validator import validate

INVOICE_YAML = """
version: "1"
name: invoice_v1
ontology:
  entities:
    - name: invoice_id
      type: string
      required: false
      min_length: 1
      max_length: 32
    - name: vendor
      type: string
      required: true
    - name: total
      type: number
      required: true
      min: 0
    - name: status
      type: string
      required: true
      enum: [open, paid, overdue]
"""

NOTE = "Bill from Oscorp, no invoice number on it. Total 42.50 USD, still open."


class _LocalEgress:
    """Validates with the in-process engine; counts calls."""

    def __init__(self, yaml_text: str = INVOICE_YAML) -> None:
        self.compiled = Contract.from_yaml(yaml_text).compile()
        self.calls: List[Dict[str, Any]] = []

    def list_contracts(self):
        return [SimpleNamespace(id="invoice_v1", name="invoice_v1")]

    def egress(self, *, contract_id, events, version=None, disposition=None,
               dry_run=False, on_fail=None, timeout=None):
        event = events[0]
        self.calls.append(event)
        r = validate(self.compiled, event)
        ok = bool(r.passed)
        return EgressResponse(
            total=1, passed=int(ok), failed=int(not ok), dry_run=dry_run,
            disposition="block", resolved_version="1",
            payload=[event] if ok else [],
            outcomes=[EgressOutcome(index=0, passed=ok, violations=list(r.violations),
                                    validation_us=1, action="included" if ok else "blocked")],
        )

    def close(self) -> None:
        pass


class _AsyncEgress(_LocalEgress):
    async def list_contracts(self):  # type: ignore[override]
        return super().list_contracts()

    async def egress(self, **kw):  # type: ignore[override]
        return super().egress(**kw)

    async def aclose(self) -> None:
        pass


def _resp(obj: Dict[str, Any]) -> SimpleNamespace:
    return SimpleNamespace(
        choices=[SimpleNamespace(message=SimpleNamespace(content=json.dumps(obj), tool_calls=None))]
    )


class _LLM:
    def __init__(self, answers: List[Dict[str, Any]]) -> None:
        self._answers = list(answers)
        self.calls: List[List[Dict[str, Any]]] = []
        self.chat = SimpleNamespace(completions=SimpleNamespace(create=self._create))

    def _create(self, *, model, messages, **kw):
        self.calls.append(messages)
        return _resp(self._answers.pop(0))


MSGS = [
    {"role": "system", "content": "Extract the invoice. total range 0..100000."},
    {"role": "user", "content": NOTE},
]
BAD_FIRST = {"vendor": "Oscorp", "total": 42.5, "status": "pending"}  # enum violation


def _gate(llm, egress=None, **kw):
    return LLMGate(api_key="k", contract_id="invoice_v1", llm=llm,
                   client=egress or _LocalEgress(), **kw)


# --------------------------------------------------------------------------
# Pure helpers
# --------------------------------------------------------------------------


@pytest.mark.parametrize("name,expected", [
    ("invoice_id", True), ("ticketId", True), ("order_ref", True), ("id", True),
    ("refund_amount_usd", True), ("total", True), ("unit_price", True),
    ("vendor", False), ("status", False), ("summary", False), ("idle", False),
    ("country_code", False), ("", False),
])
def test_is_checked_field(name, expected):
    assert is_checked_field(name) is expected


@pytest.mark.parametrize("value,source,expected", [
    ("INV-1001", "Invoice INV-1001 from Northwind", True),
    ("inv-1001", "Invoice INV-1001 from Northwind", True),
    ("INV 1001", "Invoice INV-1001 from Northwind", True),
    ("INV001", "Number: (blank). Total: TBD", False),
    ("1234567890", "no ticket id. user says broken", False),
    (42.5, "Total 42.50 USD", True),
    (1250, "it's $1,250.00 and unpaid", True),
    (980.4, "über 980,40 €", True),
    (1250.0, "Betrag 1.250,00 EUR", True),
    (3200, "Amount USD 3.2k.", True),
    (-12, "Charged -12, currency usd", True),
    (12, "Charged -12, currency usd", False),
    (5, "items 3,5 and 9", True),
    (0.0, "Total: TBD", False),
    (0, "INV-551 total: 0 USD", True),
    (551, "INV-551 / Stark", True),
    (7731, "Ticket 7731 is open", True),
    (None, "anything", True),
    ("", "anything", True),
    (True, "anything", True),
    (12.5, "twelve dollars and fifty cents", False),
])
def test_value_in_source(value, source, expected):
    assert value_in_source(value, source) is expected


def test_unsupported_fields_respects_explicit_fields():
    payload = {"invoice_id": "INV001", "vendor": "Nobody Inc", "total": 1}
    src = "Vendor Oscorp, total 1"
    assert unsupported_fields(payload, src, ProvenanceCheck()) == [("invoice_id", "INV001")]
    cfg = ProvenanceCheck(fields=["vendor"])
    assert unsupported_fields(payload, src, cfg) == [("vendor", "Nobody Inc")]
    assert unsupported_fields(payload, src, ProvenanceCheck(enabled=False)) == []


def test_source_text_uses_user_messages_only():
    msgs = [
        {"role": "system", "content": "range 0..100000"},
        {"role": "user", "content": [{"type": "text", "text": "Total 5"}, "and 6"]},
        {"role": "assistant", "content": "INV-9"},
    ]
    assert source_text_from_messages(msgs) == "Total 5\nand 6"


def test_config_validation():
    assert coerce_config(True).enabled is True
    assert coerce_config(False).enabled is False
    assert coerce_config(None).enabled is False
    with pytest.raises(ValueError):
        ProvenanceCheck(on_unsupported="drop")
    with pytest.raises(ValueError):
        ProvenanceCheck(scope="sometimes")
    with pytest.raises(TypeError):
        ProvenanceCheck(fields="invoice_id")
    with pytest.raises(TypeError):
        coerce_config("yes")


def test_digit_scan_is_linear_on_hostile_input():
    """Long digit/separator runs must not blow up (ReDoS guard)."""
    hostile = "1," * 20000 + "x"
    assert value_in_source(5, hostile) is False


# --------------------------------------------------------------------------
# LLMGate repair loop
# --------------------------------------------------------------------------


# ``any`` accepts an explicit null, so nulling is enough for this contract.
NULLABLE_YAML = INVOICE_YAML.replace(
    "    - name: invoice_id\n      type: string", "    - name: invoice_id\n      type: any", 1
)
assert NULLABLE_YAML != INVOICE_YAML


def test_repair_invented_id_is_nulled_when_contract_allows_null():
    llm = _LLM([BAD_FIRST, {"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5,
                            "status": "open"}])
    egress = _LocalEgress(NULLABLE_YAML)
    result = _gate(llm, egress).completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is True
    assert result.attempts == 2
    assert result.payload["invoice_id"] is None
    assert [(f.field, f.value, f.action) for f in result.provenance] == [
        ("invoice_id", "INV001", "nulled")
    ]
    assert len(llm.calls) == 2  # clearing needs no extra model call
    assert egress.calls[-1]["invoice_id"] is None


def test_repair_invented_optional_id_is_removed_when_null_rejected():
    llm = _LLM([BAD_FIRST, {"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5,
                            "status": "open"}])
    egress = _LocalEgress()  # optional, but explicit null is a type mismatch
    result = _gate(llm, egress).completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is True
    assert "invoice_id" not in result.payload
    assert [f.action for f in result.provenance] == ["removed"]
    assert len(llm.calls) == 2
    assert egress.calls[-2]["invoice_id"] is None  # null tried first
    assert "invoice_id" not in egress.calls[-1]


def test_repair_invented_required_amount_fails_and_reprompts():
    invented = {"vendor": "Oscorp", "total": 0.0, "status": "open"}
    note = "Vendor Oscorp invoiced us. Total: TBD. Status: open."
    msgs = [{"role": "user", "content": note}]
    llm = _LLM([BAD_FIRST, invented, invented])
    result = _gate(llm).completion(model="m", messages=msgs, auto_repair=True)
    assert result.ok is False
    assert result.attempts == 3
    kinds = [v.kind for v in result.violations]
    assert ViolationKind.PROVENANCE_VIOLATION in kinds
    assert [f.action for f in result.provenance] == ["failed", "failed"]
    # The provenance hint went to the model before the last attempt.
    assert "not found in the source text" in llm.calls[2][-1]["content"]
    assert "0.0" not in llm.calls[2][-1]["content"]  # value is not echoed back


def test_repair_reprompt_can_recover_from_provenance_failure():
    note = "Vendor Oscorp invoiced us. Total 42.50. Status: open."
    msgs = [{"role": "user", "content": note}]
    llm = _LLM([BAD_FIRST, {"vendor": "Oscorp", "total": 0.0, "status": "open"},
                {"vendor": "Oscorp", "total": 42.5, "status": "open"}])
    result = _gate(llm).completion(model="m", messages=msgs, auto_repair=True)
    assert result.ok is True
    assert result.attempts == 3
    assert [f.action for f in result.provenance] == ["failed"]


def test_grounded_repair_passes_untouched():
    llm = _LLM([BAD_FIRST, {"vendor": "Oscorp", "total": 42.5, "status": "open"}])
    result = _gate(llm).completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is True and result.provenance == []


def test_first_answer_not_checked_by_default_scope():
    llm = _LLM([{"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5, "status": "open"}])
    result = _gate(llm).completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is True
    assert result.payload["invoice_id"] == "INV001"


def test_scope_all_checks_first_answer():
    llm = _LLM([{"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5, "status": "open"}])
    gate = _gate(llm, provenance=ProvenanceCheck(scope="all"))
    result = gate.completion(model="m", messages=MSGS, auto_repair=False)
    assert result.ok is True
    assert "invoice_id" not in result.payload
    assert result.provenance[0].attempt == 1


def test_fail_mode_never_nulls():
    llm = _LLM([BAD_FIRST] + [{"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5,
                               "status": "open"}] * 2)
    gate = _gate(llm, provenance=ProvenanceCheck(on_unsupported="fail"))
    result = gate.completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is False
    assert all(f.action == "failed" for f in result.provenance)


def test_disabled_keeps_old_behavior():
    llm = _LLM([BAD_FIRST, {"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5,
                            "status": "open"}])
    result = _gate(llm, provenance=False).completion(model="m", messages=MSGS, auto_repair=True)
    assert result.ok is True
    assert result.payload["invoice_id"] == "INV001"
    assert result.provenance == []


def test_explicit_source_text_overrides_messages():
    llm = _LLM([BAD_FIRST, {"invoice_id": "INV-77", "vendor": "Oscorp", "total": 42.5,
                            "status": "open"}])
    result = _gate(llm).completion(
        model="m", messages=MSGS, auto_repair=True,
        source_text="Invoice INV-77 from Oscorp, 42.50, open",
    )
    assert result.payload["invoice_id"] == "INV-77"
    assert result.provenance == []


def test_async_gate_mirrors_sync():
    class _AsyncLLM(_LLM):
        def __init__(self, answers):
            super().__init__(answers)

            async def create(**kw):
                return self._create(**kw)

            self.chat = SimpleNamespace(completions=SimpleNamespace(create=create))

    llm = _AsyncLLM([BAD_FIRST, {"invoice_id": "INV001", "vendor": "Oscorp", "total": 42.5,
                                 "status": "open"}])
    gate = AsyncLLMGate(api_key="k", contract_id="invoice_v1", llm=llm, client=_AsyncEgress())
    result = asyncio.run(gate.completion(model="m", messages=MSGS, auto_repair=True))
    assert result.ok is True
    assert "invoice_id" not in result.payload
    assert result.provenance[0].action == "removed"
