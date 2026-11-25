"""RFC-118 parity: the local validator must match the Rust engine on the
shared corpus in ``tests/fixtures/parity/rfc118.json`` at the repo root
(cross-field rules, per-item ``[]`` paths, patterns in array items,
completeness, freshness and byte lengths). Rust runs the same file in
``tests/parity_rfc118.rs``.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from contractgate import Contract, ContractCompileError
from contractgate.validator import validate

FIXTURE = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "parity" / "rfc118.json"

pytestmark = pytest.mark.skipif(not FIXTURE.exists(), reason="repo parity corpus not present")


def _load():
    return json.loads(FIXTURE.read_text(encoding="utf-8"))


def _cases():
    if not FIXTURE.exists():
        return []
    return [pytest.param(c, id=c["name"]) for c in _load()["cases"]]


@pytest.mark.parametrize("case", _cases())
def test_matches_rust(case):
    fx = _load()
    compiled = Contract.from_yaml(fx["contract_yaml"]).compile()
    got = [
        [v.field, v.kind.value, v.message]
        for v in validate(compiled, case["event"], now=fx["now"]).violations
    ]
    assert got == case["expected"]


@pytest.mark.parametrize(
    "rule, message",
    [
        ("  - field: a\n    op: gte\n", "must set either"),
        (
            "  - field: a\n    op: gte\n    other: b\n    required_if:\n      field: c\n      equals: 1\n",
            "must set either",
        ),
        ("  - field: a[].x[].y\n    op: gte\n    other: b\n", "more than one"),
        ("  - field: a[].x\n    op: gte\n    other: b[].y\n", "two different arrays"),
    ],
)
def test_bad_cross_field_shapes_fail_to_compile(rule, message):
    yaml_src = (
        'version: "1"\nname: t\nontology:\n  entities:\n    - name: a\n      type: integer\n'
        "cross_field:\n" + rule
    )
    with pytest.raises(ContractCompileError, match=message):
        Contract.from_yaml(yaml_src).compile()


def test_contracts_without_new_sections_are_unchanged():
    c = Contract.from_yaml(
        'version: "1"\nname: t\nontology:\n  entities:\n    - name: a\n      type: integer\n'
    )
    assert c.quality == [] and c.cross_field == [] and c.null_as_absent is False
    assert validate(c.compile(), {"a": 1}).passed


def test_null_as_absent():
    c = Contract.from_yaml(
        'version: "1"\nname: t\nnull_as_absent: true\nontology:\n  entities:\n'
        "    - name: a\n      type: integer\n      required: false\n"
        "    - name: b\n      type: integer\n"
    ).compile()
    res = validate(c, {"a": None, "b": None})
    assert [(v.field, v.kind.value) for v in res.violations] == [
        ("b", "missing_required_field")
    ]
