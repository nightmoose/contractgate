"""``contractgate synth`` CLI tests (RFC-118).

Uses ``httpx.MockTransport``; no live gateway.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List

import httpx
import pytest

from contractgate import Client
from contractgate.cli import main, to_csv

YAML = 'version: "1"\nname: t\nontology:\n  entities:\n    - name: id\n      type: string\n'

VALID = {
    "contract": {"name": "t", "version": "1"},
    "source": {"type": "inline"},
    "mode": "valid",
    "seed": 7,
    "anchor_time": "2026-01-01T00:00:00Z",
    "row_count": 2,
    "rows": [{"id": "a", "ship": {"country": "US"}, "tags": ["x"]}, {"id": "b"}],
    "skipped_rules": [],
    "envelope_payload": {"data": [{"id": "a"}, {"id": "b"}]},
}

INVALID = {
    **VALID,
    "mode": "invalid",
    "rows": [
        {"record": {"id": 5}, "broken_rule": {"field": "id", "rule": "type"}},
        {"record": {}, "broken_rule": {"field": "id", "rule": "required"}},
    ],
    "skipped_rules": [{"field": "x", "rule": "min", "reason": "coupled"}],
}


def _mock(body: Dict[str, Any], status: int = 200, seen: List[httpx.Request] = None):
    def handler(req: httpx.Request) -> httpx.Response:
        if seen is not None:
            seen.append(req)
        return httpx.Response(status, json=body)

    return httpx.MockTransport(handler)


@pytest.fixture
def contract(tmp_path):
    p = tmp_path / "c.yaml"
    p.write_text(YAML)
    return str(p)


def test_valid_json_sends_the_right_request(contract, capsys, monkeypatch):
    monkeypatch.setenv("CONTRACTGATE_API_KEY", "k-test")
    seen: List[httpx.Request] = []
    code = main(
        [
            "synth",
            contract,
            "--rows",
            "2",
            "--seed",
            "7",
            "--anchor",
            "2026-01-01T00:00:00Z",
            "--base-url",
            "https://gw.example/",
        ],
        transport=_mock(VALID, seen=seen),
    )
    assert code == 0
    (req,) = seen
    assert req.method == "POST"
    assert str(req.url) == "https://gw.example/contracts/synth"
    assert req.headers["x-api-key"] == "k-test"
    assert json.loads(req.content) == {
        "rows": 2,
        "mode": "valid",
        "yaml_content": YAML,
        "seed": 7,
        "anchor_time": "2026-01-01T00:00:00Z",
    }
    assert json.loads(capsys.readouterr().out) == VALID["rows"]


def test_contract_id_invalid_csv(capsys):
    seen: List[httpx.Request] = []
    code = main(
        ["synth", "--contract-id", "abc", "--version", "1.0.0", "--invalid", "--format", "csv",
         "--api-key", "k"],
        transport=_mock(INVALID, seen=seen),
    )
    assert code == 0
    body = json.loads(seen[0].content)
    assert body["contract_id"] == "abc" and body["version"] == "1.0.0"
    assert body["mode"] == "invalid" and "yaml_content" not in body
    out, err = capsys.readouterr()
    lines = out.splitlines()
    assert lines[0] == "id,_broken_field,_broken_rule"
    assert lines[1] == "5,id,type"
    assert lines[2] == ",id,required"
    assert "seed: 7" in err and "no row breaks the min rule" in err


def test_envelope_output(contract, capsys, tmp_path):
    seen: List[httpx.Request] = []
    out_file = tmp_path / "o.json"
    code = main(
        ["synth", contract, "--envelope", "--seed", "1", "-o", str(out_file)],
        transport=_mock(VALID, seen=seen),
    )
    assert code == 0
    assert json.loads(seen[0].content)["envelope"] is True
    assert json.loads(out_file.read_text()) == VALID["envelope_payload"]

    code = main(["synth", contract, "--envelope", "--invalid", "--seed", "1"],
                transport=_mock(INVALID))
    assert code == 0
    v = json.loads(capsys.readouterr().out)
    assert v["payload"] == VALID["envelope_payload"]
    assert [l["rule"] for l in v["labels"]] == ["type", "required"]


def test_csv_flattens_like_the_rust_cli():
    text = to_csv(VALID)
    assert text.splitlines() == ["id,ship.country,tags", 'a,US,"[""x""]"', "b,,"]


def test_unsatisfiable_rule_exits_1(contract, capsys):
    err = {"error": "unsatisfiable_rule", "rule": "min_length", "field": "c",
           "message": "min_length 5 > max_length 2", "status": 422}
    code = main(["synth", contract, "--seed", "1"], transport=_mock(err, status=422))
    assert code == 1
    assert "rule 'min_length' on 'c'" in capsys.readouterr().err


def test_other_errors_exit_2(contract, capsys, tmp_path):
    code = main(["synth", contract, "--seed", "1"],
                transport=_mock({"error": "unauthorized"}, status=401))
    assert code == 2
    assert "HTTP 401" in capsys.readouterr().err
    assert main(["synth"], transport=_mock(VALID)) == 2
    assert main(["synth", contract, "--contract-id", "x"], transport=_mock(VALID)) == 2
    assert main(["synth", contract, "--envelope", "--format", "csv"],
                transport=_mock(VALID)) == 2
    assert main(["synth", str(tmp_path / "missing.yaml")], transport=_mock(VALID)) == 2


def test_client_synth_validates_arguments():
    with Client(base_url="https://gw.example", transport=_mock(VALID)) as cg:
        with pytest.raises(ValueError):
            cg.synth()
        with pytest.raises(ValueError):
            cg.synth(yaml_content=YAML, mode="weird")
        assert cg.synth(yaml_content=YAML)["row_count"] == 2


async def test_async_client_synth():
    from contractgate import AsyncClient

    async with AsyncClient(base_url="https://gw.example", transport=_mockasync()) as cg:
        body = await cg.synth(contract_id="abc", rows=3, envelope=True)
    assert body["mode"] == "valid"


def _mockasync():
    return httpx.MockTransport(lambda req: httpx.Response(200, json=VALID))
