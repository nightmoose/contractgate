"""Local validator unit tests.

Mirrors the unit-test corpus in ``src/validation.rs::tests`` plus a
few extra cases for nesting, arrays, metrics, and compliance mode.
"""

from __future__ import annotations

import pathlib

from contractgate import Contract, ViolationKind

FIXTURES = pathlib.Path(__file__).parent / "fixtures" / "contracts"


def _load(name: str) -> str:
    return (FIXTURES / name).read_text()


def _compiled(name: str):
    return Contract.from_yaml(_load(name)).compile()


# ---------------------------------------------------------------------------
# Happy path / single-kind violations
# ---------------------------------------------------------------------------


def test_valid_event_passes():
    cc = _compiled("user_events.yaml")
    r = cc.validate(
        {"user_id": "alice_01", "event_type": "click", "timestamp": 1712000000}
    )
    assert r.passed, r.violations


def test_missing_required_field():
    cc = _compiled("user_events.yaml")
    r = cc.validate({"user_id": "alice_01", "event_type": "click"})
    assert not r.passed
    assert any(v.kind == ViolationKind.MISSING_REQUIRED_FIELD for v in r.violations)


def test_pattern_violation():
    cc = _compiled("user_events.yaml")
    r = cc.validate(
        {"user_id": "alice 01!!", "event_type": "click", "timestamp": 1712000000}
    )
    assert not r.passed
    assert any(v.kind == ViolationKind.PATTERN_MISMATCH for v in r.violations)


def test_enum_violation():
    cc = _compiled("user_events.yaml")
    r = cc.validate(
        {"user_id": "alice_01", "event_type": "delete", "timestamp": 1712000000}
    )
    assert not r.passed
    assert any(v.kind == ViolationKind.ENUM_VIOLATION for v in r.violations)


def test_range_violation():
    cc = _compiled("user_events.yaml")
    r = cc.validate({"user_id": "alice_01", "event_type": "click", "timestamp": -1})
    assert not r.passed
    assert any(v.kind == ViolationKind.RANGE_VIOLATION for v in r.violations)


def test_type_mismatch():
    cc = _compiled("user_events.yaml")
    r = cc.validate(
        {"user_id": "alice_01", "event_type": "click", "timestamp": "not-a-number"}
    )
    assert not r.passed
    assert any(v.kind == ViolationKind.TYPE_MISMATCH for v in r.violations)


def test_non_object_event():
    cc = _compiled("user_events.yaml")
    r = cc.validate(["not", "an", "object"])
    assert not r.passed


# ---------------------------------------------------------------------------
# Compliance mode
# ---------------------------------------------------------------------------


def test_compliance_mode_undeclared_field_rejected():
    cc = _compiled("compliance_mode.yaml")
    r = cc.validate({"user_id": "x", "event_type": "click", "stray_field": 1})
    assert not r.passed
    kinds = [v.kind for v in r.violations]
    assert ViolationKind.UNDECLARED_FIELD in kinds


def test_compliance_mode_declared_only_passes():
    cc = _compiled("compliance_mode.yaml")
    r = cc.validate({"user_id": "x", "event_type": "click"})
    assert r.passed, r.violations


def test_undeclared_appears_after_other_violations():
    cc = _compiled("compliance_mode.yaml")
    # Missing required + undeclared in one event — order should put
    # missing first (matches Rust pipeline order).
    r = cc.validate({"user_id": "x", "stray": 1})
    assert not r.passed
    kinds = [v.kind for v in r.violations]
    missing_idx = kinds.index(ViolationKind.MISSING_REQUIRED_FIELD)
    undeclared_idx = kinds.index(ViolationKind.UNDECLARED_FIELD)
    assert missing_idx < undeclared_idx


# ---------------------------------------------------------------------------
# Nested / array
# ---------------------------------------------------------------------------


def test_nested_object_path_dotted():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: user
      type: object
      properties:
        - name: address
          type: object
          properties:
            - name: zip
              type: string
              pattern: "^[0-9]{5}$"
"""
    cc = Contract.from_yaml(yaml).compile()
    r = cc.validate({"user": {"address": {"zip": "abc"}}})
    assert not r.passed
    paths = {v.field for v in r.violations}
    assert "user.address.zip" in paths


def test_array_item_path_indexed():
    """Array items inherit the item type check, with indexed paths.

    Note: pattern checks on array items are not wired in either side
    (Rust ``compile_field_patterns`` only recurses into ``Object``
    properties, not ``Array.items``). The validator still runs type +
    enum checks on each item, which is what we assert here. If we
    decide to wire item patterns on the Rust side later, this test
    grows to cover them on both sides simultaneously.
    """
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: tags
      type: array
      items:
        name: tag
        type: string
"""
    cc = Contract.from_yaml(yaml).compile()
    r = cc.validate({"tags": ["ok", 42, "good"]})
    assert not r.passed
    paths = {v.field for v in r.violations}
    assert "tags[1]" in paths
    assert any(v.kind == ViolationKind.TYPE_MISMATCH for v in r.violations)


# ---------------------------------------------------------------------------
# Metrics
# ---------------------------------------------------------------------------


def test_metric_range_violation():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: latency
      type: float
metrics:
  - name: latency_ms
    field: latency
    max: 500
"""
    cc = Contract.from_yaml(yaml).compile()
    r = cc.validate({"latency": 999.0})
    assert not r.passed
    assert any(v.kind == ViolationKind.METRIC_RANGE_VIOLATION for v in r.violations)


def test_metric_missing_field_reports_missing():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: latency
      type: float
      required: false
metrics:
  - name: latency_ms
    field: latency
    max: 500
"""
    cc = Contract.from_yaml(yaml).compile()
    r = cc.validate({})
    assert not r.passed
    assert any(v.kind == ViolationKind.MISSING_REQUIRED_FIELD for v in r.violations)


# ---------------------------------------------------------------------------
# Wire-shape parity: violations serialize to the strings Rust emits
# ---------------------------------------------------------------------------


def _format_contract(fmt: str) -> str:
    return f"""
version: "1.0"
name: "fmt"
ontology:
  entities:
    - name: value
      type: string
      required: true
      format: {fmt}
"""


def test_format_email_matches_rust_engine():
    cc = Contract.from_yaml(_format_contract("email")).compile()
    assert cc.validate({"value": "a@b.co"}).passed
    assert cc.validate({"value": "A@B.CO"}).passed
    assert cc.validate({"value": "a@b.co.uk"}).passed
    bad_emails = (
        "a@b",
        "a b@c.d",
        "not-an-email",
        "@b.co",
        "a@",
        "a@.co",
        "a@b.",
        "a@b.co.",
        "a@@b.co",
    )
    for bad in bad_emails:
        result = cc.validate({"value": bad})
        assert not result.passed, bad
        assert result.violations[0].kind == ViolationKind.PATTERN_MISMATCH
        assert result.violations[0].message == (
            f"Field 'value' value \"{bad}\" is not a valid email"
        )


def test_format_name_match_is_case_insensitive_and_message_keeps_spelling():
    cc = Contract.from_yaml(_format_contract("Email")).compile()
    result = cc.validate({"value": "not-an-email"})
    assert result.violations[0].message == (
        'Field \'value\' value "not-an-email" is not a valid Email'
    )


def test_format_uuid_and_uri_match_rust_engine():
    uuid_cc = Contract.from_yaml(_format_contract("uuid")).compile()
    good_uuid = "123e4567-e89b-12d3-a456-426614174000"
    assert uuid_cc.validate({"value": good_uuid}).passed
    bare = "123e4567e89b12d3a456426614174000"
    uuid_bad = uuid_cc.validate({"value": bare})
    assert not uuid_bad.passed
    assert uuid_bad.violations[0].message == (
        f'Field \'value\' value "{bare}" is not a valid uuid'
    )

    for fmt in ("uri", "URL"):
        cc = Contract.from_yaml(_format_contract(fmt)).compile()
        assert cc.validate({"value": "https://x.y/z"}).passed
        assert cc.validate({"value": "mailto:user@example.com"}).passed
        refused = cc.validate({"value": "not a uri"})
        assert not refused.passed
        assert f"is not a valid {fmt}" in refused.violations[0].message
    assert not Contract.from_yaml(_format_contract("uri")).compile().validate(
        {"value": "http:"}
    ).passed
    assert not Contract.from_yaml(_format_contract("uri")).compile().validate(
        {"value": "1http://x"}
    ).passed


def test_unknown_format_is_metadata():
    cc = Contract.from_yaml(_format_contract("hostname")).compile()
    assert cc.validate({"value": "not-an-email"}).passed


def test_violation_kind_serializes_to_snake_case():
    # ``ViolationKind`` extends ``str`` so its value IS the snake_case
    # string Rust serializes to. This asserts the contract.
    assert ViolationKind.MISSING_REQUIRED_FIELD == "missing_required_field"
    assert ViolationKind.UNDECLARED_FIELD == "undeclared_field"
    assert ViolationKind.METRIC_RANGE_VIOLATION == "metric_range_violation"
