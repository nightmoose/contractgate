"""LlamaIndex output parser that checks text with AISchemaGate.

Install with ``pip install 'aischemagate[llamaindex]'``. Importing this module
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
    from llama_index.core.output_parsers import BaseOutputParser
except ImportError as exc:  # pragma: no cover - exercised when the extra is absent
    raise missing_extra("llamaindex", "AISchemaGateParser") from exc

from contractgate.llm import LLMGate


_FORMAT_INSTRUCTIONS = (
    "Return one JSON object and nothing else. "
    "AISchemaGate checks the object against the configured contract."
)


class AISchemaGateParser(BaseOutputParser):
    """Parse model text into the object ``POST /egress/{contract_id}`` accepted.

    ``contract_id`` is a contract UUID, or a contract name resolved with
    ``GET /contracts`` by :class:`contractgate.llm.LLMGate`. ``parse`` returns
    the egress payload. A structural rejection, or text that is not a JSON
    object, raises :class:`aischemagate._output.AISchemaGateRejected`.

    Streaming output is not supported. Pass the finished text.
    """

    def __init__(
        self,
        *,
        api_key: str,
        contract_id: str,
        base_url: str = DEFAULT_BASE_URL,
        version: Optional[str] = None,
        disposition: Optional[str] = None,
        dry_run: bool = False,
        client: Any = None,
    ) -> None:
        self._dry_run = dry_run
        self._gate: LLMGate = build_gate(
            api_key=api_key,
            contract_id=contract_id,
            base_url=base_url,
            version=version,
            disposition=disposition,
            client=client,
        )

    def parse(self, output: str) -> Dict[str, Any]:
        return check_output(self._gate, output, dry_run=self._dry_run)

    def format(self, query: str) -> str:
        return f"{query}\n\n{_FORMAT_INSTRUCTIONS}"

    def close(self) -> None:
        self._gate.close()

    def __enter__(self) -> "AISchemaGateParser":
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()


__all__ = ["AISchemaGateParser", "AISchemaGateRejected"]
