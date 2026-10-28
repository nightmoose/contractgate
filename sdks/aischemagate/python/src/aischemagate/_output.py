"""Shared egress check used by the framework adapters.

Validation goes through :class:`contractgate.llm.LLMGate`, which posts the
object to ``POST /egress/{contract_id}``. A contract name is resolved with
``GET /contracts`` inside that client. This module does not import a
framework SDK.
"""

from __future__ import annotations

from enum import Enum
from typing import Any, Dict, List, Optional

from contractgate import Client, ContractGateError
from contractgate.llm import LLMGate, LLMResponseError
from contractgate.models import Violation

DEFAULT_BASE_URL = "https://api.datacontractgate.com"


class AISchemaGateRejected(ContractGateError, ValueError):
    """The output failed the AISchemaGate structural check.

    ``ValueError`` is in the bases so a Pydantic ``mode='before'`` validator
    reports this as a ``ValidationError`` before field validators run.
    ``violations`` is empty when the text could not be read as a JSON object
    (including a token stream, which this package does not validate).
    ``payload`` is the egress payload from a rejected call, which is not safe
    to forward. A passing check does not raise; the caller receives that
    payload instead.
    """

    def __init__(
        self,
        message: str,
        *,
        violations: Optional[List[Violation]] = None,
        payload: Optional[Dict[str, Any]] = None,
    ) -> None:
        super().__init__(message)
        self.violations = list(violations or [])
        self.payload = dict(payload or {})


def format_violations(violations: List[Violation]) -> str:
    """One block of text an output parser can show to a caller or a model."""
    if not violations:
        return "AISchemaGate rejected the output."
    lines = ["AISchemaGate rejected the output:"]
    for violation in violations:
        kind = violation.kind.value if isinstance(violation.kind, Enum) else str(violation.kind)
        lines.append(f"- {violation.field} ({kind}): {violation.message}")
    return "\n".join(lines)


def build_gate(
    *,
    api_key: str,
    contract_id: str,
    base_url: str = DEFAULT_BASE_URL,
    version: Optional[str] = None,
    disposition: Optional[str] = None,
    client: Optional[Client] = None,
) -> LLMGate:
    """Construct the :class:`LLMGate` the adapters call.

    ``client`` must be the synchronous :class:`contractgate.Client`. The
    adapters call ``validate_only``, which is synchronous.
    """
    if client is not None and not isinstance(client, Client):
        raise TypeError(
            "client must be the synchronous contractgate.Client used by LLMGate.validate_only."
        )
    return LLMGate(
        api_key=api_key,
        contract_id=contract_id,
        base_url=base_url,
        version=version,
        disposition=disposition,
        client=client,
    )


def check_output(
    gate: LLMGate,
    response: Any,
    *,
    dry_run: bool = False,
) -> Dict[str, Any]:
    """Return the egress payload for ``response``, or raise.

    The returned object is the gateway payload (declared transforms already
    applied), not the original text. HTTP failures from the gateway propagate
    as the SDK's own errors. A stream raises :class:`AISchemaGateRejected`
    with the same message ``LLMGate.validate_only`` uses; chunk validation is
    not implemented here.
    """
    try:
        result = gate.validate_only(response, dry_run=dry_run)
    except LLMResponseError as exc:
        raise AISchemaGateRejected(str(exc)) from exc
    if not result.ok:
        raise AISchemaGateRejected(
            format_violations(result.violations),
            violations=result.violations,
            payload=result.payload,
        )
    return dict(result.payload)


def missing_extra(extra: str, feature: str) -> ImportError:
    return ImportError(
        f"{feature} requires the optional extra 'aischemagate[{extra}]'. "
        f"Install it with: pip install 'aischemagate[{extra}]'"
    )
