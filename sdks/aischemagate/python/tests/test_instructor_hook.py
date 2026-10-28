"""Instructor hook: structural check runs before Pydantic field validation."""

from __future__ import annotations

from typing import Any, List

import httpx
import pytest
from pydantic import BaseModel, ValidationError, field_validator

import instructor
from aischemagate.instructor import AISchemaGateHook
from gateway import CONTRACT_UUID, client_for, contract_row, fail_egress, ok_egress, recording_client, status_for


def test_instructor_extra_imports():
    assert instructor.__name__ == "instructor"


def test_guard_preserves_the_model_name_and_fields():
    hook = AISchemaGateHook(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({})),
    )

    @hook.guard
    class Invoice(BaseModel):
        amount: int
        currency: str

    try:
        assert Invoice.__name__ == "Invoice"
        schema = Invoice.model_json_schema()
        assert schema["title"] == "Invoice"
        assert set(schema["properties"]) == {"amount", "currency"}
    finally:
        hook.close()


def test_structural_failure_is_reported_before_field_validation():
    """A string in an int field would also fail Pydantic. The contract error wins."""
    seen: List[str] = []
    body = fail_egress("amount", "amount must be an integer", "type_mismatch")

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(f"{request.method} {request.url.path}")
        if request.url.path == "/contracts":
            return httpx.Response(200, json=[contract_row(CONTRACT_UUID, "invoice_extract")])
        return httpx.Response(status_for(body), json=body)

    hook = AISchemaGateHook(
        api_key="k",
        contract_id="invoice_extract",
        client=client_for(handler),
    )
    field_values: List[Any] = []

    @hook.guard
    class Invoice(BaseModel):
        amount: int

        @field_validator("amount", mode="before")
        @classmethod
        def _track(cls, value: Any) -> Any:
            field_values.append(value)
            return value

    try:
        with pytest.raises(ValidationError, match="type_mismatch") as caught:
            Invoice.model_validate({"amount": "nope"})
    finally:
        hook.close()
    assert field_values == []
    assert "valid integer" not in str(caught.value)
    assert seen == ["GET /contracts", f"POST /egress/{CONTRACT_UUID}"]


def test_passing_check_validates_the_repaired_payload():
    order: List[str] = []
    repaired = {"email": "hmac-sha256:abc", "amount": 3}

    def handler(request: httpx.Request) -> httpx.Response:
        order.append("egress")
        return httpx.Response(200, json=ok_egress(repaired))

    hook = AISchemaGateHook(api_key="k", contract_id=CONTRACT_UUID, client=client_for(handler))

    @hook.guard
    class Invoice(BaseModel):
        email: str
        amount: int

        @field_validator("email")
        @classmethod
        def _track(cls, value: str) -> str:
            order.append(f"pydantic:{value}")
            return value

    try:
        invoice = Invoice.model_validate({"email": "a@b.com", "amount": 3})
    finally:
        hook.close()
    assert invoice.email == "hmac-sha256:abc"
    assert invoice.amount == 3
    assert order == ["egress", "pydantic:hmac-sha256:abc"]


def test_pydantic_failure_still_runs_after_a_passing_structural_check():
    hook = AISchemaGateHook(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({"amount": 3})),
    )

    @hook.guard
    class Invoice(BaseModel):
        amount: int

        @field_validator("amount")
        @classmethod
        def _nonzero(cls, value: int) -> int:
            raise ValueError("amount must be zero in this model")

    try:
        with pytest.raises(ValidationError, match="amount must be zero in this model"):
            Invoice.model_validate({"amount": 3})
    finally:
        hook.close()


def test_guard_rejects_a_non_model():
    hook = AISchemaGateHook(
        api_key="k",
        contract_id=CONTRACT_UUID,
        client=recording_client([], ok_egress({})),
    )
    try:
        with pytest.raises(TypeError, match="BaseModel"):
            hook.guard(dict)  # type: ignore[arg-type]
    finally:
        hook.close()
