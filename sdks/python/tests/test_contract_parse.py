"""Contract YAML parse + compile tests."""

from __future__ import annotations

import pathlib

import pytest

from contractgate import (
    Contract,
    ContractCompileError,
    FieldType,
    MaskStyle,
    TransformKind,
)

FIXTURES = pathlib.Path(__file__).parent / "fixtures" / "contracts"


def _load(name: str) -> str:
    return (FIXTURES / name).read_text()


def test_canonical_contract_parses():
    c = Contract.from_yaml(_load("user_events.yaml"))
    assert c.version == "1.0"
    assert c.name == "user_events"
    assert c.compliance_mode is False
    assert len(c.entities) == 4
    user_id = c.entities[0]
    assert user_id.field_type == FieldType.STRING
    assert user_id.pattern == "^[a-zA-Z0-9_-]+$"
    # `type: number` aliases to FLOAT
    assert c.entities[3].field_type == FieldType.FLOAT


def test_compile_returns_compiled_contract():
    compiled = Contract.from_yaml(_load("user_events.yaml")).compile()
    assert "user_id" in compiled.patterns
    # compliance_mode off → declared set is empty
    assert compiled.declared_top_level_fields == set()


def test_compliance_mode_parses_and_caches_declared_set():
    compiled = Contract.from_yaml(_load("compliance_mode.yaml")).compile()
    assert compiled.contract.compliance_mode is True
    assert compiled.declared_top_level_fields == {"user_id", "event_type"}


def test_transform_parses():
    compiled = Contract.from_yaml(_load("with_transform.yaml")).compile()
    user_id = compiled.contract.entities[0]
    assert user_id.transform is not None
    assert user_id.transform.kind == TransformKind.MASK
    assert user_id.transform.style == MaskStyle.OPAQUE


def test_bad_regex_raises_compile_error():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: bad
      type: string
      pattern: "([unclosed"
"""
    with pytest.raises(ContractCompileError, match="Invalid regex"):
        Contract.from_yaml(yaml).compile()


def test_transform_on_non_string_field_rejected():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: amount
      type: integer
      transform:
        kind: hash
"""
    # Same wording as Rust ``validate_transform_types`` — locked here so
    # we notice a drift on either side immediately.
    with pytest.raises(
        ContractCompileError,
        match="declares a PII transform but has type 'Integer'",
    ):
        Contract.from_yaml(yaml).compile()


def test_missing_top_level_required_fields():
    with pytest.raises(ContractCompileError):
        Contract.from_yaml("ontology:\n  entities: []\n")  # no version/name


def test_glossary_legacy_aliases_accepted():
    yaml = """
version: "1.0"
name: "x"
ontology:
  entities:
    - name: a
      type: string
glossary:
  - term: a
    definition: "legacy aliases for field/description"
"""
    c = Contract.from_yaml(yaml)
    assert c.glossary[0].field == "a"
    assert "legacy" in c.glossary[0].description
