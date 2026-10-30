"""LangChain output parser that checks text with AISchemaGate.

Install with ``pip install 'aischemagate[langchain]'``. Importing this module
does not happen from ``import aischemagate``.
"""

from __future__ import annotations

from typing import Any, Dict, Optional

from aischemagate._output import (
    AISchemaGateRejected,
    DEFAULT_BASE_URL,
    build_gate,
    check_output,
    missing_extra,
)

try:
    from langchain_core.exceptions import OutputParserException
    from langchain_core.output_parsers import BaseOutputParser
    from pydantic import Field, PrivateAttr
except ImportError as exc:  # pragma: no cover - exercised when the extra is absent
    raise missing_extra("langchain", "AISchemaGateParser") from exc

from contractgate.llm import LLMGate


class AISchemaGateParser(BaseOutputParser[Dict[str, Any]]):
    """Parse model text into the object ``POST /egress/{contract_id}`` accepted.

    ``contract_id`` is a contract UUID, or a contract name. A name is resolved
    with ``GET /contracts``, which is what :class:`contractgate.llm.LLMGate`
    already does. ``parse`` returns that egress payload. A structural
    rejection, or text that is not a JSON object, raises
    :class:`OutputParserException`. The ``__cause__`` is
    :class:`aischemagate._output.AISchemaGateRejected`.

    Streaming output is not supported. Pass the finished text.
    """

    # Left out of repr and model_dump so a chain dump does not print the key
    # or the client that holds it.
    api_key: str = Field(repr=False, exclude=True)
    contract_id: str
    base_url: str = DEFAULT_BASE_URL
    version: Optional[str] = None
    disposition: Optional[str] = None
    on_fail: Optional[str] = None
    dry_run: bool = False
    client: Optional[Any] = Field(default=None, repr=False, exclude=True)

    _gate: Optional[LLMGate] = PrivateAttr(default=None)

    def model_post_init(self, context: Any) -> None:
        super().model_post_init(context)
        self._gate = build_gate(
            api_key=self.api_key,
            contract_id=self.contract_id,
            base_url=self.base_url,
            version=self.version,
            disposition=self.disposition,
            on_fail=self.on_fail,
            client=self.client,
        )

    def parse(self, text: str) -> Dict[str, Any]:
        if self._gate is None:
            raise RuntimeError("AISchemaGateParser is not initialized.")
        try:
            return check_output(self._gate, text, dry_run=self.dry_run)
        except AISchemaGateRejected as exc:
            observed = text if isinstance(text, str) else ""
            raise OutputParserException(
                str(exc),
                observation=str(exc),
                llm_output=observed,
                send_to_llm=False,
            ) from exc

    def get_format_instructions(self) -> str:
        return (
            "Return one JSON object and nothing else. "
            "AISchemaGate checks the object against the configured contract."
        )

    @property
    def _type(self) -> str:
        return "aischemagate"

    def close(self) -> None:
        if self._gate is not None:
            self._gate.close()

    def __enter__(self) -> "AISchemaGateParser":
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()


__all__ = ["AISchemaGateParser", "AISchemaGateRejected", "OutputParserException"]
