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
    3. Per-event quality rules: ``completeness`` and ``freshness``
       (``uniqueness`` is batch-level, ``validity`` is declarative).
    3b. ``cross_field`` rules (RFC-118), including per-item ``[]`` paths.
    4. If ``compliance_mode`` is true, emit ``UNDECLARED_FIELD`` for
       every top-level key not in the declared set.

String lengths count UTF-8 bytes, as Rust's ``str::len`` does.
"""

from __future__ import annotations

import re
import time
from typing import Any, Dict, List, Optional, Pattern

from contractgate.contract import (
    CompareOp,
    CompiledContract,
    CrossFieldRule,
    FieldDefinition,
    FieldType,
    MetricDefinition,
    QualityRule,
    QualityRuleType,
)
from contractgate.models import ValidationResult, Violation, ViolationKind


def validate(
    compiled: CompiledContract, event: Any, *, now: Optional[float] = None
) -> ValidationResult:
    """Validate ``event`` against ``compiled``.

    Always succeeds — never raises. Mirrors the ``validate`` function
    in ``src/validation.rs``. ``now`` (Unix seconds) is the time
    ``freshness`` rules are measured from; the wall clock when omitted
    (``validate_at`` in Rust).
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
    null_as_absent = compiled.contract.null_as_absent
    _validate_fields(
        compiled.contract.entities, event, "", compiled.patterns, violations, null_as_absent
    )

    # 2. Metric definitions
    for metric in compiled.contract.metrics:
        _validate_metric(metric, event, violations)

    # 3. Per-event quality rules
    for rule in compiled.contract.quality:
        if rule.rule_type == QualityRuleType.COMPLETENESS:
            _check_completeness(rule, event, violations)
        elif rule.rule_type == QualityRuleType.FRESHNESS:
            _check_freshness(rule, event, now, violations)

    # 3b. Cross-field rules (RFC-118)
    for cross in compiled.contract.cross_field:
        _check_cross_field(cross, event, violations)

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
    null_as_absent: bool = False,
) -> None:
    if not isinstance(data, dict):
        # Parent type mismatch already reported.
        return

    for f in fields:
        path = f.name if not prefix else f"{prefix}.{f.name}"
        # RFC-087: with null_as_absent, a present null is a missing key.
        if f.name not in data or (null_as_absent and data[f.name] is None):
            if f.required:
                violations.append(
                    Violation(
                        field=path,
                        message=f"Required field '{f.name}' is missing",
                        kind=ViolationKind.MISSING_REQUIRED_FIELD,
                    )
                )
            continue
        _validate_value(f, data[f.name], path, patterns, violations, null_as_absent)


def _validate_value(
    f: FieldDefinition,
    value: Any,
    path: str,
    patterns: Dict[str, Pattern[str]],
    violations: List[Violation],
    null_as_absent: bool = False,
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
        n_bytes = len(value.encode("utf-8"))
        if f.min_length is not None and n_bytes < f.min_length:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' length {n_bytes} is below minimum "
                        f"{f.min_length}"
                    ),
                    kind=ViolationKind.LENGTH_VIOLATION,
                )
            )
        if f.max_length is not None and n_bytes > f.max_length:
            violations.append(
                Violation(
                    field=path,
                    message=(
                        f"Field '{path}' length {n_bytes} exceeds maximum "
                        f"{f.max_length}"
                    ),
                    kind=ViolationKind.LENGTH_VIOLATION,
                )
            )
        regex = _pattern_for(patterns, path)
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
        _validate_fields(f.properties, value, path, patterns, violations, null_as_absent)

    # --- Recurse into array items ---
    if f.field_type == FieldType.ARRAY and f.items is not None and isinstance(value, list):
        for idx, item in enumerate(value):
            item_path = f"{path}[{idx}]"
            _validate_value(f.items, item, item_path, patterns, violations, null_as_absent)


_INDEX_RE = re.compile(r"\[[^\]]*\]")


def _pattern_for(patterns: Dict[str, Pattern[str]], path: str) -> Optional[Pattern[str]]:
    """Compiled pattern for a concrete path, including array items
    (``lines[3].sku`` looks up ``lines[].sku``)."""
    regex = patterns.get(path)
    if regex is None and "[" in path:
        regex = patterns.get(_INDEX_RE.sub("[]", path))
    return regex


# ---------------------------------------------------------------------------
# Quality rules (per event)
# ---------------------------------------------------------------------------


def _get_nested(event: Any, path: str) -> Any:
    """Dot path lookup that tells "absent" from "null" (``_MISSING``)."""
    cur = event
    for seg in path.split("."):
        if not isinstance(cur, dict) or seg not in cur:
            return _ABSENT
        cur = cur[seg]
    return cur


_ABSENT = object()


def _check_completeness(rule: QualityRule, event: Any, violations: List[Violation]) -> None:
    v = _get_nested(event, rule.field)
    if v is _ABSENT or v is None:
        violations.append(
            Violation(
                field=rule.field,
                message=f"Quality completeness: field '{rule.field}' is absent or null",
                kind=ViolationKind.COMPLETENESS_VIOLATION,
            )
        )
    elif v == "" and isinstance(v, str):
        violations.append(
            Violation(
                field=rule.field,
                message=f"Quality completeness: field '{rule.field}' is an empty string",
                kind=ViolationKind.COMPLETENESS_VIOLATION,
            )
        )


_I64_MIN = -(2**63)


def _freshness_seconds(v: Any) -> Optional[int]:
    """Mirror the Rust freshness reader: integer epochs (ms when above
    1_700_000_000_000), fractional epochs (floored), RFC 3339 strings."""
    import math

    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        if _I64_MIN <= v <= _I64_MAX:
            return v // 1000 if v > 1_700_000_000_000 else v
        f = float(v)
    elif isinstance(v, float):
        f = v
    elif isinstance(v, str):
        inst = _rfc3339_instant(v)
        return None if inst is None else inst[0]
    else:
        return None
    if not math.isfinite(f):
        return None
    # Rust: f.floor() as i64 saturates at the i64 bounds.
    secs = max(min(math.floor(f), _I64_MAX), _I64_MIN)
    return secs // 1000 if secs > 1_700_000_000_000 else secs


def _check_freshness(
    rule: QualityRule, event: Any, now: Optional[float], violations: List[Violation]
) -> None:
    if rule.max_age_seconds is None:
        return
    v = _get_nested(event, rule.field)
    if v is _ABSENT:
        return
    ts = _freshness_seconds(v)
    if ts is None:
        violations.append(
            Violation(
                field=rule.field,
                message=(
                    f"Quality freshness: field '{rule.field}' is not a numeric Unix "
                    "timestamp or an RFC 3339 date-time"
                ),
                kind=ViolationKind.FRESHNESS_VIOLATION,
            )
        )
        return
    now_secs = int(now) if now is not None else int(time.time())
    age = now_secs - ts
    if age < 0 or age > rule.max_age_seconds:
        violations.append(
            Violation(
                field=rule.field,
                message=(
                    f"Quality freshness: field '{rule.field}' timestamp is {age}s old "
                    f"(max {rule.max_age_seconds}s)"
                ),
                kind=ViolationKind.FRESHNESS_VIOLATION,
            )
        )


# ---------------------------------------------------------------------------
# Cross-field rules (RFC-118)
# ---------------------------------------------------------------------------


def _resolve_indexed(event: Any, path: str) -> Any:
    """``a.b[2].c``; returns ``_ABSENT`` when any step is missing."""
    cur = event
    for seg in path.split("."):
        open_i = seg.find("[")
        name = seg if open_i < 0 else seg[:open_i]
        if open_i < 0 or name:
            if not isinstance(cur, dict) or name not in cur:
                return _ABSENT
            cur = cur[name]
        if open_i >= 0:
            for idx in seg[open_i:].split("[")[1:]:
                if not idx.endswith("]"):
                    return _ABSENT
                raw = idx[:-1]
                if not raw.isdigit() or not raw.isascii():
                    return _ABSENT
                i = int(raw)
                if not isinstance(cur, list) or i >= len(cur):
                    return _ABSENT
                cur = cur[i]
    return cur


def _expand_cross_rule(rule: CrossFieldRule, event: Any) -> List[CrossFieldRule]:
    prefix = rule.wildcard_array()
    if prefix is None:
        return [rule]
    arr = _resolve_indexed(event, prefix)
    n = len(arr) if isinstance(arr, list) else 0
    return [rule.concrete(i) for i in range(n)]


def _rfc3339_instant(value: str) -> Optional[tuple]:
    """(seconds, nanos) for an RFC 3339 string, or None. Mirrors Chrono:
    second 60 becomes 59 plus 1_000_000_000 nanos."""
    import calendar

    if not _rfc3339_ok(value):
        return None
    raw = value
    year, month, day = int(raw[0:4]), int(raw[5:7]), int(raw[8:10])
    hour, minute, sec = int(raw[11:13]), int(raw[14:16]), int(raw[17:19])
    rest = raw[19:]
    nanos = 0
    if rest.startswith("."):
        parsed = _fractional_nanos(rest[1:])
        if parsed is None:
            return None
        rest, nanos = parsed
    if sec == 60:
        sec = 59
        nanos += 1_000_000_000
    offset = 0
    if rest[0] not in "Zz":
        sign = 1 if rest[0] == "+" else -1
        body = rest[1:]
        offset = sign * (int(body[0:2]) * 3600 + int(body[3:5]) * 60)
    secs = calendar.timegm((year, month, day, hour, minute, sec, 0, 0, 0)) - offset
    return secs, nanos


def _calendar_date(value: str) -> Optional[tuple]:
    if not _calendar_date_ok(value):
        return None
    year, rest = _signed_year(value)
    rest = rest[1:].lstrip()
    month, rest = _take_digits(rest, 1, 2)
    rest = rest[1:].lstrip()
    day, rest = _take_digits(rest, 1, 2)
    return (year, month, day)


def _compare_values(a: Any, b: Any) -> Optional[int]:
    """-1, 0, 1, or None when the values have no order (Rust ``compare_values``)."""
    x, y = _numeric_value(a), _numeric_value(b)
    if x is not None and y is not None:
        if x != x or y != y:  # NaN
            return None
        return (x > y) - (x < y)
    if not (isinstance(a, str) and isinstance(b, str)):
        return None
    ta, tb = _rfc3339_instant(a), _rfc3339_instant(b)
    da = None if ta is not None else _calendar_date(a)
    db = None if tb is not None else _calendar_date(b)
    if ta is not None and tb is not None:
        return (ta > tb) - (ta < tb)
    if ta is None and tb is None and da is not None and db is not None:
        return (da > db) - (da < db)
    if ta is None and tb is None and da is None and db is None:
        ea, eb = a.encode("utf-8"), b.encode("utf-8")
        return (ea > eb) - (ea < eb)
    return None


def _compare_holds(op: CompareOp, a: Any, b: Any) -> bool:
    o = _compare_values(a, b)
    if o is None:
        if op == CompareOp.EQ:
            return _json_equal(a, b)
        if op == CompareOp.NE:
            return not _json_equal(a, b)
        return True
    return {
        CompareOp.EQ: o == 0,
        CompareOp.NE: o != 0,
        CompareOp.LT: o < 0,
        CompareOp.LTE: o <= 0,
        CompareOp.GT: o > 0,
        CompareOp.GTE: o >= 0,
    }[op]


def _json_equal(a: Any, b: Any) -> bool:
    """serde_json ``Value`` equality: booleans are never numbers, and
    integers and floats compare by JSON number value."""
    if isinstance(a, bool) or isinstance(b, bool):
        return isinstance(a, bool) and isinstance(b, bool) and a == b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(_json_equal(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(_json_equal(x, y) for x, y in zip(a, b))
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        # serde_json keeps i64/u64 and f64 as different Number variants.
        return type(a) is type(b) and a == b
    return type(a) is type(b) and a == b


def _check_cross_field(rule: CrossFieldRule, event: Any, violations: List[Violation]) -> None:
    label = rule.label()
    for c in _expand_cross_rule(rule, event):
        if c.op is not None and c.other is not None:
            a = _resolve_indexed(event, c.field)
            b = _resolve_indexed(event, c.other)
            if a is _ABSENT or a is None or b is _ABSENT or b is None:
                continue
            if not _compare_holds(c.op, a, b):
                violations.append(
                    Violation(
                        field=c.field,
                        message=(
                            f"Cross-field rule '{label}' failed: "
                            f"{_json_compact(a)} vs {_json_compact(b)}"
                        ),
                        kind=ViolationKind.CROSS_FIELD_VIOLATION,
                    )
                )
        elif c.required_if_field is not None:
            cond = _resolve_indexed(event, c.required_if_field)
            if cond is _ABSENT or not _json_equal(cond, c.required_if_equals):
                continue
            v = _resolve_indexed(event, c.field)
            if v is _ABSENT or v is None:
                violations.append(
                    Violation(
                        field=c.field,
                        message=f"Cross-field rule '{label}' failed: '{c.field}' is missing",
                        kind=ViolationKind.CROSS_FIELD_VIOLATION,
                    )
                )


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
