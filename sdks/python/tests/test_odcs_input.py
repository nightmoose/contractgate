"""ODCS v3.x documents (no ``ontology``) parse with ``Contract.from_yaml``."""

from __future__ import annotations

import pathlib

import pytest

from contractgate import Contract, ContractCompileError, FieldType

FIXTURES = pathlib.Path(__file__).parent / "fixtures" / "contracts"


def _load(name: str) -> str:
    return (FIXTURES / name).read_text()


def test_minimal_odcs_parses_and_validates_events():
    c = Contract.from_yaml(_load("odcs_minimal.yaml"))
    assert c.name == "orders"
    assert c.description == "Minimal orders contract"
    assert [e.name for e in c.entities] == ["order_id", "amount", "status"]
    assert c.entities[1].field_type == FieldType.FLOAT
    compiled = c.compile()
    assert compiled.validate({"order_id": "a", "amount": 2.5, "status": "open"}).passed
    assert not compiled.validate({"order_id": "a", "amount": -1, "status": "open"}).passed
    assert not compiled.validate({"order_id": "a", "amount": 1, "status": "nope"}).passed


def test_official_full_example_parses():
    c = Contract.from_yaml(_load("odcs_full_example.yaml"))
    # description is a map in ODCS 3.x
    assert c.description == "Views built on top of the seller tables."
    names = {e.name: e for e in c.entities}
    assert names["rcvr_cntry_code"].allowed_values == ["US", "CA", "MX"]
    assert names["transaction_reference_date"].field_type == FieldType.DATE
    assert names["rcvr_id"].required is True  # primaryKey
    compiled = c.compile()
    assert compiled.validate({"rcvr_id": "x", "rcvr_cntry_code": "US"}).passed


def test_ontology_is_optional_for_native_contracts():
    c = Contract.from_yaml('version: "1.0"\nname: bare\n')
    assert c.entities == []


def test_unsupported_odcs_errors_are_clear():
    with pytest.raises(ContractCompileError, match="no 'schema' array"):
        Contract.from_yaml("apiVersion: v3.1.0\nkind: DataContract\nname: n\nversion: 1.0.0\n")
    with pytest.raises(ContractCompileError, match="unsupported ODCS apiVersion"):
        Contract.from_yaml("apiVersion: v2.2.2\nkind: DataContract\nname: n\nversion: 1\nschema: []\n")


def test_distribution_hint_is_read_and_checked():
    c = Contract.from_yaml(
        'version: "1.0"\nname: d\nontology:\n  entities:\n'
        "    - name: x\n      type: number\n"
        "      distribution: {type: normal, mean: 5, sd: 2}\n"
    )
    assert c.entities[0].distribution == {"type": "normal", "mean": 5.0, "sd": 2.0}
    # the validator ignores it
    assert c.compile().validate({"x": 1.5}).passed
    with pytest.raises(ContractCompileError, match="sd greater than 0"):
        Contract.from_yaml(
            'version: "1.0"\nname: d\nontology:\n  entities:\n'
            "    - name: x\n      type: number\n      distribution: {type: normal, mean: 1, sd: 0}\n"
        )
    with pytest.raises(ContractCompileError, match="unknown distribution type"):
        Contract.from_yaml(
            'version: "1.0"\nname: d\nontology:\n  entities:\n'
            "    - name: x\n      type: number\n      distribution: {type: weibull}\n"
        )


def test_odcs_x_cg_distribution_custom_property():
    c = Contract.from_yaml(
        "apiVersion: v3.1.0\nkind: DataContract\nversion: 1.0.0\nname: n\nschema:\n  - name: t\n    properties:\n"
        "      - name: x\n        logicalType: number\n        customProperties:\n"
        "          - property: x-cg-distribution\n            value: {type: lognormal, mean: 3, sd: 0.5}\n"
    )
    assert c.entities[0].distribution == {"type": "lognormal", "mean": 3.0, "sd": 0.5}
