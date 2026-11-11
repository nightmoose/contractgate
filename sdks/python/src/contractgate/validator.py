"""Pure-Python validator — port of ``src/validation.rs``.

Strict parity with the Rust engine: same per-event check order, same
``ViolationKind`` values, same field-path format, same message text.
The shared fixture corpus in ``tests/fixtures/parity/`` locks both
implementations against drift.

Read-only: this module reports violations. RFC-004 PII transforms are
NOT applied here (the gateway is the single source of truth for the
post-transform payload).

Per-event order (matches Rust):
    1. Walk ontology fields recursively (required / type / pattern /
       format / enum / range / length). ``format`` of ``email``, ``uuid``,
       and ``uri``/``url`` matches ``string_format_ok`` in
       ``src/validation.rs``. Other format names pass. ``type: date`` and
       ``type: timestamp`` match ``NaiveDate::parse_from_str("%Y-%m-%d")``
       and ``DateTime::parse_from_rfc3339`` in that file.
    2. Walk metrics with ``field`` set + ``min``/``max`` bounds.
    3. If ``compliance_mode`` is true, emit ``UNDECLARED_FIELD`` for
       every top-level key not in the declared set.
"""

from __future__ import annotations

import re
import time
from typing import Any, Dict, List, Optional, Pattern

from contractgate.contract import (
    CompiledContract,
    FieldDefinition,
    FieldType,
    MetricDefinition,
)
from contractgate.models import ValidationResult, Violation, ViolationKind


def validate(compiled: CompiledContract, event: Any) -> ValidationResult:
    """Validate ``event`` against ``compiled``.

    Always succeeds — never raises. Mirrors the ``validate`` function
    in ``src/validation.rs``.
    """
    t0 = time.perf_counter()
    violations: List[Violation] = []

    if not isinstance(event, dict):
        return ValidationResult(
            passed=False,
            violations=[
                Violation(
                    field="<root>",
                    message="Event must be a JSON object",
                    kind=ViolationKind.TYPE_MISMATCH,
                )
            ],
            validation_us=0,
        )

    # 1. Ontology fields
    _validate_fields(compiled.contract.entities, event, "", compiled.patterns, violations)

    # 2. Metric definitions
    for metric in compiled.contract.metrics:
        _validate_metric(metric, event, violations)

    # 3. Compliance-mode undeclared field check (last — keeps the
    #    standard violations first, matching Rust's order so triage
    #    workflows don't have to special-case the SDK).
    if compiled.contract.compliance_mode:
        for field_name in event.keys():
            if field_name not in compiled.declared_top_level_fields:
                violations.append(
                    Violation(
                        field=field_name,
                        message=(
                            f"Field '{field_name}' is not declared in the contract "
                            "ontology. Compliance mode rejects undeclared fields."
                        ),
                        kind=ViolationKind.UNDECLARED_FIELD,
                    )
                )

    elapsed_us = int((time.perf_counter() - t0) * 1_000_000)
    return ValidationResult(
        passed=not violations,
        violations=violations,
        validation_us=elapsed_us,
    )


# ---------------------------------------------------------------------------
# Field walker
# ---------------------------------------------------------------------------


def _validate_fields(
    fields: List[FieldDefinition],
    data: Any,
    prefix: str,
    patterns: Dict[str, Pattern[str]],
    violations: List[Violation],
) -> None:
    if not isinstance(data, dict):
        # Parent type mismatch already reported.
        return

    for f in fields:
        path = f.name if not prefix else f"{prefix}.{f.name}"
        if f.name not in data:
            if f.required:
                violations.append(
                    Violation(
                        field=path,
                        message=f"Required field '{f.name}' is missing",
                        kind=ViolationKind.MISSING_REQUIRED_FIELD,
                    )
                )
            continue
        _validate_value(f, data[f.name], path, patterns, violations)


def _validate_value(
    f: FieldDefinition,
    value: Any,
    path: str,
    patterns: Dict[str, Pattern[str]],
    violations: List[Violation],
) -> None:
    # --- Type check ---
    if not _type_matches(f.field_type, value):
        violations.append(
            Violation(
                field=path,
                message=(
                    f"Field '{path}' expected type {_rust_field_type_repr(f.field_type)}, "
                    f"got {_json_type_name(value)}"
                ),
                kind=ViolationKind.TYPE_MISMATCH,
            )
        )
        return  # Further checks on the wrong type are noise.

    # --- String checks ---
    if isinstance(value, str):
        if f.min_length is not None and len(value) < f.min_length:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' length {len(value)} is below minimum "
                        f"{f.min_length}"
                    ),
                    kind=ViolationKind.LENGTH_VIOLATION,
                )
            )
        if f.max_length is not None and len(value) > f.max_length:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' length {len(value)} exceeds maximum "
                        f"{f.max_length}"
                    ),
                    kind=ViolationKind.LENGTH_VIOLATION,
                )
            )
        regex = patterns.get(path)
        if regex is not None and not regex.search(value):
            # Rust's ``regex::Regex::is_match`` is unanchored — it returns true
            # if the regex matches anywhere in the string. ``re.search`` is
            # the matching Python idiom (``re.match`` is left-anchored only).
            # User patterns that mean "match the whole value" use ``^...$``,
            # same as on the Rust side.
            violations.append(
                Violation(
                    field=path,
                    # Match Rust's ``{:?}`` debug rendering of a string —
                    # it adds surrounding quotes. ``json.dumps(s)`` produces
                    # the same shape (``"alice"``).
                    message=(
                        f"Field '{path}' value {_json_string_debug(value)} "
                        "does not match required pattern"
                    ),
                    kind=ViolationKind.PATTERN_MISMATCH,
                )
            )
        # ODCS logicalTypeOptions.format. String fields only; unknown names
        # pass. The message keeps the contract's spelling; the check folds case.
        if (
            f.field_type == FieldType.STRING
            and f.format
            and not _string_format_ok(f.format, value)
        ):
            violations.append(
                    Violation(
                        field=path,
                        message=(
                            f"Field '{path}' value {_json_string_debug(value)} "
                            f"is not a valid {f.format}"
                        ),
                        kind=ViolationKind.PATTERN_MISMATCH,
                    )
                )
        # Timestamp fields: chrono's ``DateTime::parse_from_rfc3339``.
        if f.field_type == FieldType.TIMESTAMP and not _rfc3339_ok(value):
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' value {_json_string_debug(value)} "
                        "is not an RFC 3339 timestamp"
                    ),
                    kind=ViolationKind.PATTERN_MISMATCH,
                )
            )
        # RFC-044: calendar date. Chrono's ``NaiveDate::parse_from_str``
        # with ``%Y-%m-%d``, which is padding-agnostic and proleptic Gregorian.
        if f.field_type == FieldType.DATE and not _calendar_date_ok(value):
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' value {_json_string_debug(value)} "
                        "is not a valid calendar date (expected YYYY-MM-DD)"
                    ),
                    kind=ViolationKind.PATTERN_MISMATCH,
                )
            )

    # --- Numeric range checks ---
    n = _numeric_value(value)
    if n is not None:
        if f.min is not None and n < f.min:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' value {_format_number(n)} is below minimum "
                        f"{_format_number(f.min)}"
                    ),
                    kind=ViolationKind.RANGE_VIOLATION,
                )
            )
        if f.max is not None and n > f.max:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' value {_format_number(n)} exceeds maximum "
                        f"{_format_number(f.max)}"
                    ),
                    kind=ViolationKind.RANGE_VIOLATION,
                )
            )

    # --- Enum check ---
    if f.allowed_values is not None and value not in f.allowed_values:
        rendered = ", ".join(_json_compact(v) for v in f.allowed_values)
        violations.append(
            Violation(
                field=path,
                message=(
                    f"Field '{path}' value {_json_compact(value)} not in allowed set: "
                    f"[{rendered}]"
                ),
                kind=ViolationKind.ENUM_VIOLATION,
            )
        )

    # --- Recurse into nested objects ---
    if f.field_type == FieldType.OBJECT and f.properties:
        _validate_fields(f.properties, value, path, patterns, violations)

    # --- Recurse into array items ---
    if f.field_type == FieldType.ARRAY and f.items is not None and isinstance(value, list):
        for idx, item in enumerate(value):
            item_path = f"{path}[{idx}]"
            _validate_value(f.items, item, item_path, patterns, violations)


# ---------------------------------------------------------------------------
# Metric walker
# ---------------------------------------------------------------------------


def _validate_metric(
    metric: MetricDefinition,
    event: Any,
    violations: List[Violation],
) -> None:
    if metric.field is None:
        # Formula-only metrics carry no per-event check.
        return
    if metric.min is None and metric.max is None:
        return

    raw_value = _resolve_path(event, metric.field)
    n = _numeric_value(raw_value) if raw_value is not None else None
    if n is None:
        violations.append(
            Violation(
                field=metric.field,
                message=(
                    f"Metric '{metric.name}' field '{metric.field}' is missing "
                    "or not numeric"
                ),
                kind=ViolationKind.MISSING_REQUIRED_FIELD,
            )
        )
        return
    if metric.min is not None and n < metric.min:
        violations.append(
            Violation(
                field=metric.field,
                message=(
                    f"Metric '{metric.name}' value {_format_number(n)} is below "
                    f"minimum {_format_number(metric.min)} (field: '{metric.field}')"
                ),
                kind=ViolationKind.METRIC_RANGE_VIOLATION,
            )
        )
    if metric.max is not None and n > metric.max:
        violations.append(
            Violation(
                field=metric.field,
                message=(
                    f"Metric '{metric.name}' value {_format_number(n)} exceeds "
                    f"maximum {_format_number(metric.max)} (field: '{metric.field}')"
                ),
                kind=ViolationKind.METRIC_RANGE_VIOLATION,
            )
        )


# ---------------------------------------------------------------------------
# Helpers — type/json shape
# ---------------------------------------------------------------------------


def _type_matches(ft: FieldType, value: Any) -> bool:
    if ft == FieldType.ANY:
        return True
    if ft == FieldType.STRING:
        return isinstance(value, str)
    if ft == FieldType.INTEGER:
        # Reject bool (a Python int subclass) — JSON booleans are not integers.
        return isinstance(value, int) and not isinstance(value, bool)
    if ft == FieldType.FLOAT:
        # Rust accepts i64/u64/f64 for ``Float``; mirror that.
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if ft == FieldType.BOOLEAN:
        return isinstance(value, bool)
    if ft == FieldType.OBJECT:
        return isinstance(value, dict)
    if ft == FieldType.ARRAY:
        return isinstance(value, list)
    if ft in (FieldType.DATE, FieldType.TIMESTAMP):
        return isinstance(value, str)
    return False


def _numeric_value(value: Any) -> Optional[float]:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return float(value)
    return None


def _resolve_path(value: Any, path: str) -> Any:
    """Resolve a dot-separated path in a JSON value."""
    current = value
    for key in path.split("."):
        if not isinstance(current, dict):
            return None
        current = current.get(key)
        if current is None:
            return None
    return current


def _json_type_name(value: Any) -> str:
    """Mirror Rust ``json_type_name``."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "float"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _rust_field_type_repr(ft: FieldType) -> str:
    """Mirror Rust's ``{:?}`` PascalCase debug rendering."""
    return {
        FieldType.STRING: "String",
        FieldType.INTEGER: "Integer",
        FieldType.FLOAT: "Float",
        FieldType.BOOLEAN: "Boolean",
        FieldType.OBJECT: "Object",
        FieldType.ARRAY: "Array",
        FieldType.ANY: "Any",
        FieldType.DATE: "Date",
        FieldType.TIMESTAMP: "Timestamp",
    }[ft]


def _format_number(n: float) -> str:
    """Render a numeric value the way Rust's ``Display`` does.

    Rust prints whole-number floats as ``"5"`` (no trailing ``.0``)
    when promoted from an integer, and ``Value::Number`` integers as
    plain integers. Python's ``str(5.0)`` produces ``"5.0"`` which
    drifts the parity tests, so collapse trailing zeros.
    """
    if isinstance(n, float) and n.is_integer():
        return str(int(n))
    return str(n)


def _json_string_debug(s: str) -> str:
    """Mirror Rust's ``{:?}`` rendering of a string — quoted + escaped."""
    import json

    return json.dumps(s)


# Hyphenated UUID, 8-4-4-4-12. Rust accepts this form when
# ``Uuid::parse_str`` succeeds and the string length is 36. Other UUID
# spellings (bare hex, URN, braces) are longer or shorter and fail there.
_UUID_RE = re.compile(
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
)


def _string_format_ok(fmt: str, value: str) -> bool:
    """Mirror ``string_format_ok`` in ``src/validation.rs``.

    Unknown format names return true (they are metadata). The name match is
    case-insensitive.
    """
    kind = fmt.lower()
    if kind == "email":
        return _email_ok(value)
    if kind == "uuid":
        return _UUID_RE.match(value) is not None
    if kind in ("uri", "url"):
        return _uri_ok(value)
    return True


def _email_ok(value: str) -> bool:
    """Same rules as the Rust ``email`` arm.

    One ``@``, non-empty local part, no whitespace anywhere, and a domain
    that contains a dot with a non-empty label on each side and does not end
    in a dot. ``a@b.co`` passes. ``a@b`` and ``a b@c.d`` do not.
    """
    if "@" not in value:
        return False
    local, domain = value.split("@", 1)
    if not local or "@" in domain:
        return False
    if any(ch.isspace() for ch in value):
        return False
    if "." not in domain:
        return False
    label, rest = domain.split(".", 1)
    return bool(label) and bool(rest) and not rest.endswith(".")


def _uri_ok(value: str) -> bool:
    """Same rules as the Rust ``uri`` / ``url`` arm.

    A scheme of ASCII letters, digits, ``+``, ``-``, or ``.``, starting with
    a letter, then ``:`` and a non-empty remainder. No host check.
    """
    if ":" not in value:
        return False
    scheme, rest = value.split(":", 1)
    if not rest or not scheme:
        return False
    first = scheme[0]
    if not (first.isascii() and first.isalpha()):
        return False
    return all(c.isascii() and (c.isalnum() or c in "+-.") for c in scheme)


# Chrono ``NaiveDate`` year bounds (``MIN_YEAR`` / ``MAX_YEAR`` in 0.4.45).
_YEAR_MIN = -262143
_YEAR_MAX = 262142
_I64_MAX = 9223372036854775807
_NANO_SCALE = (
    0,
    100_000_000,
    10_000_000,
    1_000_000,
    100_000,
    10_000,
    1_000,
    100,
    10,
    1,
)


def _calendar_date_ok(value: str) -> bool:
    """Mirror ``chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")``.

    The generic Chrono parser trims whitespace before each number, accepts a
    leading ``+`` or ``-`` on the year (then any number of digits), and
    otherwise reads 1–4 year digits and 1–2 month and day digits. The whole
    string must be consumed. The date must be a real proleptic Gregorian day
    inside Chrono's year range, so ``2024-02-29`` passes and ``2023-02-29``
    and ``1900-02-29`` do not.
    """
    year, rest = _signed_year(value)
    if year is None or not rest.startswith("-"):
        return False
    rest = rest[1:].lstrip()
    month, rest = _take_digits(rest, 1, 2)
    if month is None or not rest.startswith("-"):
        return False
    rest = rest[1:].lstrip()
    day, rest = _take_digits(rest, 1, 2)
    if day is None or rest != "":
        return False
    return _real_ymd(year, month, day)


def _signed_year(value: str) -> tuple[Optional[int], str]:
    rest = value.lstrip()
    if not rest:
        return None, rest
    sign = 1
    if rest[0] in "+-":
        sign = -1 if rest[0] == "-" else 1
        year, rest = _take_digits(rest[1:], 1, None)
    else:
        year, rest = _take_digits(rest, 1, 4)
    if year is None:
        return None, rest
    return sign * year, rest


def _take_digits(
    value: str, minimum: int, maximum: Optional[int]
) -> tuple[Optional[int], str]:
    """Read ``minimum``..``maximum`` digits. ``None`` maximum stops at ``i64``."""
    number = 0
    count = 0
    index = 0
    cap = maximum if maximum is not None else len(value)
    while index < len(value) and count < cap and value[index].isascii() and value[index].isdigit():
        digit = ord(value[index]) - 48
        if number > (_I64_MAX - digit) // 10:
            return None, value
        number = number * 10 + digit
        index += 1
        count += 1
    if count < minimum:
        return None, value
    return number, value[index:]


def _real_ymd(year: int, month: int, day: int) -> bool:
    if year < _YEAR_MIN or year > _YEAR_MAX:
        return False
    if month < 1 or month > 12 or day < 1:
        return False
    lengths = (0, 31, 29 if _is_leap(year) else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)
    return day <= lengths[month]


def _is_leap(year: int) -> bool:
    return year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)


def _rfc3339_ok(value: str) -> bool:
    """Mirror ``chrono::DateTime::parse_from_rfc3339`` (Chrono 0.4.45).

    Four-digit year, two-digit month and day, ``T`` / ``t`` / space, two-digit
    hour, minute, and second, optional fractional seconds (extra digits past
    nine are skipped), then ``Z`` / ``z`` or ``±HH:MM`` in ``-23:59``..``+23:59``.
    A Unicode minus (U+2212) is accepted. ``:60`` is a leap second: Chrono
    rewrites it to second 59 plus ``1_000_000_000`` nanos, which
    ``NaiveTime::from_hms_nano_opt`` accepts. ``:61`` is rejected. The
    calendar is the same proleptic Gregorian check as ``type: date``.
    """
    raw = value.encode("utf-8")
    if len(raw) < 19:
        return False
    fixed = raw[:19]
    if fixed[4:5] != b"-" or fixed[7:8] != b"-" or fixed[13:14] != b":" or fixed[16:17] != b":":
        return False
    if fixed[10:11] not in (b"T", b"t", b" "):
        return False
    parts = []
    for start, end in ((0, 4), (5, 7), (8, 10), (11, 13), (14, 16), (17, 19)):
        chunk = fixed[start:end]
        if not chunk.isdigit():
            return False
        parts.append(int(chunk))
    year, month, day, hour, minute, sec = parts
    if not _real_ymd(year, month, day):
        return False
    if hour > 23 or minute > 59 or sec > 60:
        return False
    try:
        rest = raw[19:].decode("utf-8")
    except UnicodeDecodeError:
        return False
    if rest.startswith("."):
        parsed = _fractional_nanos(rest[1:])
        if parsed is None:
            return False
        rest, _nanos = parsed
    return _timezone_offset_ok(rest)


def _fractional_nanos(value: str) -> Optional[tuple[str, int]]:
    number, rest = _take_digits(value, 1, 9)
    if number is None:
        return None
    consumed = len(value) - len(rest)
    nanos = number * _NANO_SCALE[consumed]
    while rest[:1].isdigit():
        rest = rest[1:]
    return rest, nanos


def _timezone_offset_ok(value: str) -> bool:
    if not value:
        return False
    if value[0] in "Zz":
        return value[1:] == ""
    if value[0] == "+":
        negative = False
        rest = value[1:]
    elif value[0] in ("-", "\u2212"):
        negative = True
        rest = value[1:]
    else:
        return False
    if len(rest) < 2 or not rest[:2].isdigit():
        return False
    hours = int(rest[:2])
    rest = rest[2:]
    if not rest.startswith(":"):
        return False
    rest = rest[1:]
    if len(rest) < 2 or not rest[:2].isdigit():
        return False
    if rest[0] > "5":
        return False
    minutes = int(rest[:2])
    if rest[2:] != "":
        return False
    seconds = hours * 3600 + minutes * 60
    if negative:
        seconds = -seconds
    return -86400 < seconds < 86400


def _json_compact(v: Any) -> str:
    """Render a value the way Rust's ``serde_json::Value::Display`` does.

    ``serde_json::Value`` prints with no whitespace between tokens,
    matching ``json.dumps(..., separators=(",", ":"))``.
    """
    import json

    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)
