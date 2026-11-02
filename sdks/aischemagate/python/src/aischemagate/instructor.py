"""Instructor hook that runs the AISchemaGate structural check first.

Install with ``pip install 'aischemagate[instructor]'``. Importing this module
does not happen from ``import aischemagate``.

Instructor emits ``completion:response`` before Pydantic validation, and its
hook runner swallows exceptions from those handlers. A handler registered
with ``client.on`` therefore cannot stop field validation or become the
error Instructor reports. ``AISchemaGateHook.guard`` subclasses the response
model with a Pydantic ``mode='before'`` validator. Instructor's response
parsers call ``model_validate_json`` (OpenAI JSON, JSON schema, and tool
calls) or ``model_validate``. Both run that validator before field
validators, so a structural failure is the validation error and field
validators do not run.
"""

from __future__ import annotations

from typing import Any, Optional, Type, TypeVar

from aischemagate._output import (
    AISchemaGateRejected,
    DEFAULT_BASE_URL,
    build_gate,
    check_output,
    missing_extra,
)

try:
    import instructor as _instructor
    from pydantic import BaseModel, model_validator
except ImportError as exc:  # pragma: no cover - exercised when the extra is absent
    raise missing_extra("instructor", "AISchemaGateHook") from exc

from contractgate.llm import LLMGate

ModelT = TypeVar("ModelT", bound=BaseModel)


class AISchemaGateHook:
    """Bind a contract, then guard the Pydantic model Instructor validates.

    ``contract_id`` is a contract UUID, or a contract name resolved with
    ``GET /contracts``. The guarded model calls ``POST /egress/{contract_id}``
    and, on success, hands the egress payload to the rest of Pydantic
    validation. Streaming responses are not supported.
    """

    def __init__(
        self,
        *,
        api_key: str,
        contract_id: str,
        base_url: str = DEFAULT_BASE_URL,
        version: Optional[str] = None,
        disposition: Optional[str] = None,
        on_fail: Optional[str] = None,
        dry_run: bool = False,
        client: Any = None,
    ) -> None:
        if _instructor.__name__ != "instructor":
            raise missing_extra("instructor", "AISchemaGateHook")
        self._dry_run = dry_run
        self._gate: LLMGate = build_gate(
            api_key=api_key,
            contract_id=contract_id,
            base_url=base_url,
            version=version,
            disposition=disposition,
            on_fail=on_fail,
            client=client,
        )

    def guard(self, model: Type[ModelT]) -> Type[ModelT]:
        """Return ``model`` with the structural check installed in front.

        Use it as ``@hook.guard`` on the response model, or call
        ``hook.guard(Model)``. The class name is unchanged so Instructor's
        tool name stays the model's name.
        """
        if not isinstance(model, type) or not issubclass(model, BaseModel):
            raise TypeError("AISchemaGateHook.guard expects a Pydantic BaseModel subclass.")

        gate = self._gate
        dry_run = self._dry_run
        model_name = model.__name__
        model_qualname = model.__qualname__
        model_module = model.__module__

        class Guarded(model):
            @model_validator(mode="before")
            @classmethod
            def _aischemagate_structural(cls, data: Any) -> Any:
                # Before field validation. Raising here is the error
                # Instructor reports, and later validators do not run.
                return check_output(gate, data, dry_run=dry_run)

        Guarded.__name__ = model_name
        Guarded.__qualname__ = model_qualname
        Guarded.__module__ = model_module
        Guarded.model_rebuild(force=True)
        return Guarded

    def close(self) -> None:
        self._gate.close()

    def __enter__(self) -> "AISchemaGateHook":
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()


__all__ = ["AISchemaGateHook", "AISchemaGateRejected"]
