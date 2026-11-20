"""Provenance check for LLM repair output.

When ``LLMGate`` re-prompts a model to fix contract violations, the model can
satisfy the contract by making values up: an ``invoice_id`` of ``"INV001"``
for a note with no invoice number, or a ``total`` of ``0.0`` for a note whose
total is "TBD". The contract passes and the invented value flows downstream.

This module checks identifier and amount fields of a payload against the
source text the caller extracted from. A value that does not appear in the
source text is *unsupported*. The gate then clears it (sets it to ``null``,
or leaves the field out, whichever the contract accepts) or reports it as a
failure. It never passes silently.

The check is deterministic and conservative:

* Strings match if they occur in the source, ignoring case, or if their
  letters and digits occur in order in the source's letters and digits
  (``"INV 1001"`` matches ``INV-1001``).
* Numbers match if the same number is written in the source with digits
  (``1,250.00``, ``980,40``, ``3.2k`` and a leading minus sign are
  understood). Numbers written in words are not, so they count as
  unsupported.
* ``None``, empty strings and booleans are never unsupported.
* Only top-level fields are checked.
"""

from __future__ import annotations

import math
import re
import unicodedata
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

_ON_UNSUPPORTED = ("null", "fail")
_SCOPES = ("repair", "all")

# Last name token that marks an identifier field (``invoice_id``,
# ``orderRef``, ``ticket_number``).
_ID_TOKENS = frozenset({"id", "ids", "ref", "reference", "number", "no", "num", "uuid", "sku"})
# Any name token that marks an amount field (``total``, ``refund_amount_usd``).
_AMOUNT_TOKENS = frozenset(
    {"amount", "total", "subtotal", "price", "cost", "fee", "fees", "balance", "refund", "tax"}
)

# Linear-time patterns only: untrusted model output and user text go through
# them (see the ReDoS note on ``_strip_fence`` in ``llm.py``).
_CAMEL = re.compile(r"([a-z0-9])([A-Z])")
_SPLIT = re.compile(r"[^a-z0-9]+")
_DIGIT_RUN = re.compile(r"\d[\d.,]*")


@dataclass(frozen=True)
class ProvenanceCheck:
    """Configuration for the repair-loop provenance check.

    ``enabled``: turn the check on or off (``LLMGate(provenance=False)`` is a
    shortcut for off).
    ``fields``: field names to check. ``None`` (default) picks identifier and
    amount fields by name; see :func:`is_checked_field`.
    ``on_unsupported``: ``"null"`` (default) sets unsupported values to
    ``null`` (or, if the contract rejects ``null``, leaves the fields out)
    and keeps the result only if the contract still passes; otherwise the
    attempt fails. ``"fail"`` always fails the attempt.
    ``scope``: ``"repair"`` (default) checks only outputs produced by a repair
    re-prompt; ``"all"`` also checks the first answer.
    """

    enabled: bool = True
    fields: Optional[Sequence[str]] = None
    on_unsupported: str = "null"
    scope: str = "repair"

    def __post_init__(self) -> None:
        if self.on_unsupported not in _ON_UNSUPPORTED:
            raise ValueError(
                f"on_unsupported must be one of {_ON_UNSUPPORTED}, got {self.on_unsupported!r}"
            )
        if self.scope not in _SCOPES:
            raise ValueError(f"scope must be one of {_SCOPES}, got {self.scope!r}")
        if isinstance(self.fields, str):
            raise TypeError("fields must be a sequence of field names, not a string")


@dataclass(frozen=True)
class ProvenanceFinding:
    """One unsupported value found by the check.

    ``action`` is ``"nulled"`` when the value was replaced by ``null`` and the
    contract still passed, ``"removed"`` when the field was left out instead
    (the contract treats it as optional but rejects ``null``), or
    ``"failed"`` when the attempt was rejected.
    """

    field: str
    value: Any
    action: str
    attempt: int


def coerce_config(value: Any) -> ProvenanceCheck:
    """Accept ``True``/``False``/``None`` or a :class:`ProvenanceCheck`."""
    if isinstance(value, ProvenanceCheck):
        return value
    if value is None or value is False:
        return ProvenanceCheck(enabled=False)
    if value is True:
        return ProvenanceCheck()
    raise TypeError("provenance must be a bool or a ProvenanceCheck")


def _tokens(name: str) -> List[str]:
    spaced = _CAMEL.sub(r"\1_\2", name)
    return [t for t in _SPLIT.split(spaced.lower()) if t]


def is_checked_field(name: str) -> bool:
    """True if ``name`` looks like an identifier or amount field."""
    toks = _tokens(name)
    if not toks:
        return False
    if toks[-1] in _ID_TOKENS:
        return True
    return any(t in _AMOUNT_TOKENS for t in toks)


def _norm(text: str) -> str:
    return unicodedata.normalize("NFKC", text).casefold()


def _alnum(text: str) -> str:
    return "".join(ch for ch in text if ch.isalnum())


def _numbers_in(text: str) -> Set[float]:
    """Every number written with digits in ``text``, under common formats."""
    out: Set[float] = set()
    for m in _DIGIT_RUN.finditer(text):
        raw = m.group(0).rstrip(".,")
        start, end = m.start(), m.start() + len(raw)
        negative = start > 0 and text[start - 1] == "-" and (
            start == 1 or not text[start - 2].isalnum()
        )
        scale = 1000.0 if end < len(text) and text[end] in "kK" and (
            end + 1 == len(text) or not text[end + 1].isalpha()
        ) else 1.0
        for cand in _readings(raw):
            if not math.isfinite(cand):
                continue
            for s in {scale, 1.0}:
                v = cand * s
                out.add(-v if negative else v)
    return out


def _readings(raw: str) -> Iterable[float]:
    """``"1,250.00"`` -> 1250.0; ``"980,40"`` -> 980.4 and 98040.0; and so on."""
    seen: List[str] = [raw.replace(",", "")]  # comma as thousands separator
    if raw.count(",") == 1 and "." not in raw:
        seen.append(raw.replace(",", "."))  # decimal comma
    if "," in raw and "." in raw and raw.rfind(",") > raw.rfind("."):
        seen.append(raw.replace(".", "").replace(",", "."))  # 1.250,00
    if "," in raw:
        seen.extend(p for p in raw.split(",") if p)  # "3,5" may be a list
    for s in seen:
        if s.count(".") <= 1:
            try:
                yield float(s)
            except ValueError:
                continue


def _as_number(value: Any) -> Optional[float]:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return float(value)
    return None


def value_in_source(value: Any, source: str, *, _cache: Optional[Dict[str, Any]] = None) -> bool:
    """True if ``value`` is supported by ``source``. ``None``/``""``/bools always are."""
    if value is None or isinstance(value, bool):
        return True
    cache = _cache if _cache is not None else {}
    if "norm" not in cache:
        cache["norm"] = _norm(source)
        cache["alnum"] = _alnum(cache["norm"])
        cache["nums"] = _numbers_in(source)
    num = _as_number(value)
    if num is not None:
        nums: Set[float] = cache["nums"]
        if not math.isfinite(num):
            return False
        return any(abs(num - n) <= 1e-9 * max(1.0, abs(n)) for n in nums)
    if isinstance(value, str):
        text = _norm(value).strip()
        if not text:
            return True
        if text in cache["norm"]:
            return True
        compact = _alnum(text)
        return bool(compact) and compact in cache["alnum"]
    # Lists / objects: supported only if every scalar inside is.
    if isinstance(value, list):
        return all(value_in_source(v, source, _cache=cache) for v in value)
    if isinstance(value, dict):
        return all(value_in_source(v, source, _cache=cache) for v in value.values())
    return value_in_source(str(value), source, _cache=cache)


def unsupported_fields(
    payload: Dict[str, Any], source: str, config: ProvenanceCheck
) -> List[Tuple[str, Any]]:
    """``(field, value)`` pairs in ``payload`` that ``source`` does not support."""
    if not config.enabled or not isinstance(payload, dict):
        return []
    names = list(config.fields) if config.fields is not None else [
        k for k in payload if isinstance(k, str) and is_checked_field(k)
    ]
    cache: Dict[str, Any] = {}
    out = []
    for name in names:
        if name in payload and not value_in_source(payload[name], source, _cache=cache):
            out.append((name, payload[name]))
    return out


def source_text_from_messages(messages: Sequence[Dict[str, Any]]) -> str:
    """Text of the caller's ``user`` messages (system prompts are excluded).

    Handles plain string content and OpenAI / Anthropic content-part lists.
    Pass ``source_text=`` to ``completion()`` / ``message()`` when the user
    message also carries instructions or a contract brief with numbers.
    """
    parts: List[str] = []
    for msg in messages:
        if not isinstance(msg, dict) or msg.get("role") != "user":
            continue
        content = msg.get("content")
        if isinstance(content, str):
            parts.append(content)
        elif isinstance(content, list):
            for p in content:
                if isinstance(p, dict) and isinstance(p.get("text"), str):
                    parts.append(p["text"])
                elif isinstance(p, str):
                    parts.append(p)
    return "\n".join(parts)


def provenance_hint(found: Sequence[Tuple[str, Any]]) -> str:
    lines = [
        (
            "Some values in the previous JSON do not appear in the source text. Do not "
            "invent identifiers or amounts. Copy them exactly from the text, or use null "
            "if the text does not give them. Return ONLY the corrected JSON object."
        ),
    ]
    for name, _value in found:
        lines.append(f"- field '{name}': value not found in the source text")
    return "\n".join(lines)


__all__ = [
    "ProvenanceCheck",
    "ProvenanceFinding",
    "coerce_config",
    "is_checked_field",
    "provenance_hint",
    "source_text_from_messages",
    "unsupported_fields",
    "value_in_source",
]

