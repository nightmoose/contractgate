"""LLMGate — thin wrapper over existing egress validation for LLM outputs.

See `docs/rfcs/094-sdk-llm-output-guard.md`. All validation goes through
`client.egress(contract_id=..., events=[...])`; this module adds response-
shape extraction and a bounded auto-repair re-prompt loop. No engine code,
no new endpoint. The caller brings their own LLM client (OpenAI, Anthropic,
Ollama, vLLM) and binds it on the gate.

Public surface:
    LLMGate, AsyncLLMGate       sync + async wrapper classes
    LLMGateResult               dataclass returned by every call
    LLMResponseError            raised on unparseable / streamed responses
    extract_json                module-level extractor (also used internally)

Keep imports lazy — the submodule must be importable without pulling any
provider SDK.
"""

from __future__ import annotations

import json
import re
import warnings
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable, Dict, List, Optional

from contractgate.async_client import AsyncClient
from contractgate.client import Client
from contractgate.exceptions import ContractGateError
from contractgate.models import ContractResponse, EgressResponse, Violation

_UUID_RE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", re.IGNORECASE
)


def _is_uuid(value: str) -> bool:
    return bool(_UUID_RE.match(value))


def _match_contract(rows: List[ContractResponse], name: str) -> str:
    """Return the id of the contract named ``name``.

    The gateway's egress and ingest routes take a contract UUID, but the
    contract's YAML ``name`` is what people know, so a name is accepted and
    looked up once on the key's contract list.
    """
    match = next((r for r in rows if r.name == name), None) or next(
        (r for r in rows if r.name.lower() == name.lower()), None
    )
    if match is None:
        names = ", ".join(sorted(r.name for r in rows)) or "none"
        raise ContractGateError(
            f"No contract named {name!r} on this API key. Pass the contract's "
            f"UUID or one of these names: {names}."
        )
    return match.id

MAX_RETRIES_CAP = 5
_DEFAULT_MAX_RETRIES = 2
_FENCE = "```"


def _strip_fence(s: str) -> str:
    """Remove one outer ```` ```json ... ``` ```` fence from ``s`` (already stripped).

    Plain string checks instead of a regex: LLM output is untrusted, and the
    lazy-match regex this replaces backtracks polynomially on input like a
    fence followed by many spaces (CodeQL js/polynomial-redos).
    """
    if len(s) < 2 * len(_FENCE) or not (s.startswith(_FENCE) and s.endswith(_FENCE)):
        return s
    inner = s[len(_FENCE) : -len(_FENCE)]
    if inner[:4].lower() == "json":
        inner = inner[4:]
    return inner.strip()


class LLMResponseError(ContractGateError):
    """Raised when an LLM response cannot be parsed into a JSON object."""


@dataclass
class LLMGateResult:
    """Outcome of a validate / completion / message call.

    ``payload`` is the egress-sanitized object the caller should forward
    downstream — RFC-093 forbids forwarding the original response body
    after a ``block`` or ``fail`` disposition. On ``ok=False``, ``payload``
    is the sanitized form of what the gateway rejected.
    """

    ok: bool
    payload: Dict[str, Any]
    violations: List[Violation]
    attempts: int
    raw_response: Any
    repair_hints: List[str] = field(default_factory=list)


# ---------------------------------------------------------------------------
# Response-shape extraction
# ---------------------------------------------------------------------------


def extract_json(response: Any) -> Dict[str, Any]:
    """Pull a JSON object out of common LLM response shapes.

    Supports:
      - OpenAI ChatCompletion         ``choices[0].message.content`` (JSON mode)
      - OpenAI ChatCompletion         ``choices[0].message.tool_calls[0].function.arguments``
      - OpenAI Responses API          ``output_parsed`` (dict passthrough)
      - Anthropic Message             first ``content[*]`` with ``type == 'tool_use'`` → ``.input``
      - str                           parsed with markdown-fence tolerance
      - dict                          passthrough

    Streaming responses (iterators) raise ``LLMResponseError``.
    Unrecognizable shapes raise ``LLMResponseError``.
    """
    if _is_streaming(response):
        raise LLMResponseError(
            "streaming responses are not supported in v1 — pass stream=False "
            "or call validate_only(response) with a non-streaming response"
        )

    extracted = _try_openai_chat(response)
    if extracted is not None:
        return extracted

    extracted = _try_anthropic_message(response)
    if extracted is not None:
        return extracted

    extracted = _try_openai_responses(response)
    if extracted is not None:
        return extracted

    if isinstance(response, str):
        return _json_loads_fence_tolerant(response)

    if isinstance(response, dict):
        return response

    raise LLMResponseError(
        f"unrecognized response shape (type={type(response).__name__}); "
        "pass a dict, a JSON string, or a response from OpenAI / Anthropic"
    )


def _is_streaming(response: Any) -> bool:
    if isinstance(response, (dict, list, str, bytes)):
        return False
    # Non-stream responses expose one of these terminal attributes/keys.
    for name in ("choices", "content", "output_parsed"):
        if hasattr(response, name) or (isinstance(response, dict) and name in response):
            return False
    return hasattr(response, "__next__") or hasattr(response, "__aiter__")


def _get(obj: Any, key: str) -> Any:
    """Attribute-or-key lookup. Treats missing as ``None``."""
    if obj is None:
        return None
    if hasattr(obj, key):
        return getattr(obj, key)
    if isinstance(obj, dict):
        return obj.get(key)
    return None


def _try_openai_chat(response: Any) -> Optional[Dict[str, Any]]:
    choices = _get(response, "choices")
    if not choices:
        return None
    first = choices[0] if isinstance(choices, list) and choices else None
    if first is None:
        return None
    msg = _get(first, "message")
    if msg is None:
        return None

    # Tool call takes precedence over message content when both are present —
    # tool_calls carry structured args that are typically more trustworthy
    # than free-form JSON mode output.
    tool_calls = _get(msg, "tool_calls")
    if tool_calls:
        func = _get(tool_calls[0], "function")
        args = _get(func, "arguments")
        if isinstance(args, str):
            return _json_loads_fence_tolerant(args)
        if isinstance(args, dict):
            return args

    content = _get(msg, "content")
    if isinstance(content, str) and content.strip():
        return _json_loads_fence_tolerant(content)
    return None


def _try_anthropic_message(response: Any) -> Optional[Dict[str, Any]]:
    content = _get(response, "content")
    if not isinstance(content, list) or not content:
        return None
    for block in content:
        if _get(block, "type") == "tool_use":
            inp = _get(block, "input")
            if isinstance(inp, dict):
                return inp
    # Fallback: Anthropic can also return a text block with a JSON string.
    for block in content:
        if _get(block, "type") == "text":
            text = _get(block, "text")
            if isinstance(text, str) and text.strip():
                try:
                    return _json_loads_fence_tolerant(text)
                except LLMResponseError:
                    continue
    return None


def _try_openai_responses(response: Any) -> Optional[Dict[str, Any]]:
    out = _get(response, "output_parsed")
    if isinstance(out, dict):
        return out
    return None


def _json_loads_fence_tolerant(s: str) -> Dict[str, Any]:
    """``json.loads`` that strips an outer ```` ```json ... ``` ```` fence."""
    stripped = _strip_fence(s.strip())
    try:
        obj = json.loads(stripped)
    except json.JSONDecodeError as e:
        raise LLMResponseError(f"response is not valid JSON: {e}") from e
    if not isinstance(obj, dict):
        raise LLMResponseError(
            f"response JSON is not an object (got {type(obj).__name__})"
        )
    return obj


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


def _clamp_retries(n: int) -> int:
    if n < 0:
        return 0
    if n > MAX_RETRIES_CAP:
        warnings.warn(
            f"max_retries={n} exceeds hard cap {MAX_RETRIES_CAP}; clamping to {MAX_RETRIES_CAP}",
            stacklevel=3,
        )
        return MAX_RETRIES_CAP
    return n


def _default_repair_prompt(violations: List[Violation]) -> str:
    lines = [
        "The previous JSON was rejected by schema validation. Fix the "
        "violations below and return ONLY the corrected JSON object. No "
        "prose, no markdown fence.",
    ]
    for v in violations:
        lines.append(f"- field '{v.field}': {v.message}")
    return "\n".join(lines)


def _result_from_egress(
    egress: EgressResponse,
    *,
    raw_response: Any,
    attempts: int,
    hints: Optional[List[str]] = None,
) -> LLMGateResult:
    first_outcome = egress.outcomes[0] if egress.outcomes else None
    violations: List[Violation] = list(first_outcome.violations) if first_outcome else []
    payload_record: Dict[str, Any] = {}
    if egress.payload:
        first = egress.payload[0]
        if isinstance(first, dict):
            payload_record = first
    return LLMGateResult(
        ok=egress.failed == 0,
        payload=payload_record,
        violations=violations,
        attempts=attempts,
        raw_response=raw_response,
        repair_hints=list(hints) if hints else [],
    )


def _append_user_message(messages: List[Dict[str, Any]], hint: str) -> List[Dict[str, Any]]:
    return list(messages) + [{"role": "user", "content": hint}]


# ---------------------------------------------------------------------------
# Sync gate
# ---------------------------------------------------------------------------


class LLMGate:
    """Validate LLM structured output against a deployed contract.

    Construct once per contract, call ``completion`` / ``message`` /
    ``validate_only``. The underlying ``Client`` is owned by this gate
    unless an existing client is injected via ``client=...`` (useful for
    tests + for sharing a connection pool across multiple gates).
    """

    def __init__(
        self,
        *,
        api_key: str,
        contract_id: str,
        llm: Any,
        base_url: str = "https://api.datacontractgate.com",
        version: Optional[str] = None,
        disposition: Optional[str] = None,
        max_retries: int = _DEFAULT_MAX_RETRIES,
        repair_prompt: Optional[Callable[[List[Violation]], str]] = None,
        client: Optional[Client] = None,
    ) -> None:
        if llm is None:
            raise ValueError(
                "LLMGate requires an `llm` client (OpenAI / Anthropic / "
                "Ollama / vLLM). Example: LLMGate(..., llm=OpenAI())"
            )
        self._llm = llm
        self._contract_id = contract_id
        self._resolved: Dict[str, str] = {}
        self._version = version
        self._disposition = disposition
        self._max_retries = _clamp_retries(max_retries)
        self._repair_prompt = repair_prompt or _default_repair_prompt
        if client is None:
            self._client = Client(base_url=base_url, api_key=api_key)
            self._owns_client = True
        else:
            self._client = client
            self._owns_client = False

    # ---- lifecycle -------------------------------------------------------

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def __enter__(self) -> "LLMGate":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    # ---- public API ------------------------------------------------------

    def completion(
        self,
        *,
        model: str,
        messages: List[Dict[str, Any]],
        auto_repair: bool = False,
        **provider_kwargs: Any,
    ) -> LLMGateResult:
        """OpenAI-protocol wrapper. Calls ``llm.chat.completions.create``.

        Works unchanged for vLLM and OpenAI-compatible Ollama endpoints.
        """
        self._reject_stream_kwarg(provider_kwargs)

        def call(current: List[Dict[str, Any]]) -> Any:
            return self._llm.chat.completions.create(
                model=model, messages=current, **provider_kwargs
            )

        return self._run_loop(messages, call, auto_repair=auto_repair)

    def message(
        self,
        *,
        model: str,
        messages: List[Dict[str, Any]],
        max_tokens: int = 1024,
        auto_repair: bool = False,
        **provider_kwargs: Any,
    ) -> LLMGateResult:
        """Anthropic-protocol wrapper. Calls ``llm.messages.create``."""
        self._reject_stream_kwarg(provider_kwargs)

        def call(current: List[Dict[str, Any]]) -> Any:
            return self._llm.messages.create(
                model=model,
                messages=current,
                max_tokens=max_tokens,
                **provider_kwargs,
            )

        return self._run_loop(messages, call, auto_repair=auto_repair)

    def _resolve(self, contract_id: str) -> str:
        if _is_uuid(contract_id):
            return contract_id
        if contract_id not in self._resolved:
            self._resolved[contract_id] = _match_contract(
                self._client.list_contracts(), contract_id
            )
        return self._resolved[contract_id]

    def validate_only(
        self,
        response: Any,
        *,
        dry_run: bool = False,
    ) -> LLMGateResult:
        """Validate a response the caller already produced."""
        payload = extract_json(response)
        egress = self._client.egress(
            contract_id=self._resolve(self._contract_id),
            events=[payload],
            version=self._version,
            disposition=self._disposition,
            dry_run=dry_run,
        )
        return _result_from_egress(egress, raw_response=response, attempts=1)

    def mask_prompt(
        self,
        payload: Dict[str, Any],
        *,
        prompt_contract_id: str,
    ) -> Dict[str, Any]:
        """Return a prompt payload with RFC-004 transforms applied.

        One-line convenience over ``client.ingest(dry_run=True)``. The caller
        formats their prompt string from the returned dict.
        """
        resp = self._client.ingest(
            contract_id=self._resolve(prompt_contract_id),
            events=[payload],
            dry_run=True,
        )
        if resp.results and isinstance(resp.results[0].transformed_event, dict):
            return resp.results[0].transformed_event
        return payload

    # ---- internals -------------------------------------------------------

    @staticmethod
    def _reject_stream_kwarg(kwargs: Dict[str, Any]) -> None:
        if kwargs.pop("stream", False):
            raise LLMResponseError(
                "streaming responses are not supported in v1 — pass stream=False"
            )

    def _run_loop(
        self,
        messages: List[Dict[str, Any]],
        call: Callable[[List[Dict[str, Any]]], Any],
        *,
        auto_repair: bool,
    ) -> LLMGateResult:
        current = list(messages)
        hints: List[str] = []
        last: Optional[LLMGateResult] = None
        total_attempts = self._max_retries + 1 if auto_repair else 1
        for attempt in range(1, total_attempts + 1):
            resp = call(current)
            try:
                payload = extract_json(resp)
            except LLMResponseError:
                if not auto_repair or attempt >= total_attempts:
                    raise
                hint = (
                    "The previous response was not valid JSON. Return ONLY a "
                    "JSON object matching the required schema. No prose, no "
                    "markdown fence."
                )
                hints.append(hint)
                current = _append_user_message(current, hint)
                continue
            egress = self._client.egress(
                contract_id=self._resolve(self._contract_id),
                events=[payload],
                version=self._version,
                disposition=self._disposition,
            )
            last = _result_from_egress(egress, raw_response=resp, attempts=attempt, hints=hints)
            if egress.failed == 0 or not auto_repair or attempt >= total_attempts:
                return last
            hint = self._repair_prompt(last.violations)
            hints.append(hint)
            current = _append_user_message(current, hint)
        assert last is not None  # unreachable: the loop always sets ``last`` before exiting
        return last


# ---------------------------------------------------------------------------
# Async gate
# ---------------------------------------------------------------------------


class AsyncLLMGate:
    """Async mirror of :class:`LLMGate`. Same arguments, every call awaitable."""

    def __init__(
        self,
        *,
        api_key: str,
        contract_id: str,
        llm: Any,
        base_url: str = "https://api.datacontractgate.com",
        version: Optional[str] = None,
        disposition: Optional[str] = None,
        max_retries: int = _DEFAULT_MAX_RETRIES,
        repair_prompt: Optional[Callable[[List[Violation]], str]] = None,
        client: Optional[AsyncClient] = None,
    ) -> None:
        if llm is None:
            raise ValueError(
                "AsyncLLMGate requires an `llm` client. Example: "
                "AsyncLLMGate(..., llm=AsyncOpenAI())"
            )
        self._llm = llm
        self._contract_id = contract_id
        self._resolved: Dict[str, str] = {}
        self._version = version
        self._disposition = disposition
        self._max_retries = _clamp_retries(max_retries)
        self._repair_prompt = repair_prompt or _default_repair_prompt
        if client is None:
            self._client = AsyncClient(base_url=base_url, api_key=api_key)
            self._owns_client = True
        else:
            self._client = client
            self._owns_client = False

    async def aclose(self) -> None:
        if self._owns_client:
            await self._client.aclose()

    async def __aenter__(self) -> "AsyncLLMGate":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()

    async def completion(
        self,
        *,
        model: str,
        messages: List[Dict[str, Any]],
        auto_repair: bool = False,
        **provider_kwargs: Any,
    ) -> LLMGateResult:
        LLMGate._reject_stream_kwarg(provider_kwargs)

        async def call(current: List[Dict[str, Any]]) -> Any:
            return await self._llm.chat.completions.create(
                model=model, messages=current, **provider_kwargs
            )

        return await self._run_loop(messages, call, auto_repair=auto_repair)

    async def message(
        self,
        *,
        model: str,
        messages: List[Dict[str, Any]],
        max_tokens: int = 1024,
        auto_repair: bool = False,
        **provider_kwargs: Any,
    ) -> LLMGateResult:
        LLMGate._reject_stream_kwarg(provider_kwargs)

        async def call(current: List[Dict[str, Any]]) -> Any:
            return await self._llm.messages.create(
                model=model,
                messages=current,
                max_tokens=max_tokens,
                **provider_kwargs,
            )

        return await self._run_loop(messages, call, auto_repair=auto_repair)

    async def _resolve(self, contract_id: str) -> str:
        if _is_uuid(contract_id):
            return contract_id
        if contract_id not in self._resolved:
            self._resolved[contract_id] = _match_contract(
                await self._client.list_contracts(), contract_id
            )
        return self._resolved[contract_id]

    async def validate_only(
        self,
        response: Any,
        *,
        dry_run: bool = False,
    ) -> LLMGateResult:
        payload = extract_json(response)
        egress = await self._client.egress(
            contract_id=await self._resolve(self._contract_id),
            events=[payload],
            version=self._version,
            disposition=self._disposition,
            dry_run=dry_run,
        )
        return _result_from_egress(egress, raw_response=response, attempts=1)

    async def mask_prompt(
        self,
        payload: Dict[str, Any],
        *,
        prompt_contract_id: str,
    ) -> Dict[str, Any]:
        resp = await self._client.ingest(
            contract_id=await self._resolve(prompt_contract_id),
            events=[payload],
            dry_run=True,
        )
        if resp.results and isinstance(resp.results[0].transformed_event, dict):
            return resp.results[0].transformed_event
        return payload

    async def _run_loop(
        self,
        messages: List[Dict[str, Any]],
        call: Callable[[List[Dict[str, Any]]], Awaitable[Any]],
        *,
        auto_repair: bool,
    ) -> LLMGateResult:
        current = list(messages)
        hints: List[str] = []
        last: Optional[LLMGateResult] = None
        total_attempts = self._max_retries + 1 if auto_repair else 1
        for attempt in range(1, total_attempts + 1):
            resp = await call(current)
            try:
                payload = extract_json(resp)
            except LLMResponseError:
                if not auto_repair or attempt >= total_attempts:
                    raise
                hint = (
                    "The previous response was not valid JSON. Return ONLY a "
                    "JSON object matching the required schema. No prose, no "
                    "markdown fence."
                )
                hints.append(hint)
                current = _append_user_message(current, hint)
                continue
            egress = await self._client.egress(
                contract_id=await self._resolve(self._contract_id),
                events=[payload],
                version=self._version,
                disposition=self._disposition,
            )
            last = _result_from_egress(egress, raw_response=resp, attempts=attempt, hints=hints)
            if egress.failed == 0 or not auto_repair or attempt >= total_attempts:
                return last
            hint = self._repair_prompt(last.violations)
            hints.append(hint)
            current = _append_user_message(current, hint)
        assert last is not None
        return last


__all__ = [
    "LLMGate",
    "AsyncLLMGate",
    "LLMGateResult",
    "LLMResponseError",
    "extract_json",
    "MAX_RETRIES_CAP",
]
