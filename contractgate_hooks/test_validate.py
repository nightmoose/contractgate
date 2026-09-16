"""Tests for the local YAML compile helper (pre-commit + GitHub Action)."""

from __future__ import annotations

from pathlib import Path

import pytest

from contractgate_hooks.validate import collect_paths, compile_file, main

FIXTURES = (
    Path(__file__).resolve().parents[1]
    / "sdks"
    / "python"
    / "tests"
    / "fixtures"
    / "contracts"
)


def test_compile_good_fixture():
    compile_file(FIXTURES / "user_events.yaml")


def test_compile_rejects_garbage(tmp_path: Path):
    bad = tmp_path / "broken.yaml"
    bad.write_text("this: is: not: yaml: [", encoding="utf-8")
    with pytest.raises(Exception):
        compile_file(bad)


def test_compile_rejects_missing_ontology(tmp_path: Path):
    bad = tmp_path / "empty.yaml"
    bad.write_text("version: '1.0'\nname: nope\n", encoding="utf-8")
    from contractgate.exceptions import ContractCompileError

    with pytest.raises(ContractCompileError):
        compile_file(bad)


def test_collect_paths_from_dir():
    files = collect_paths([], str(FIXTURES))
    names = {p.name for p in files}
    assert "user_events.yaml" in names
    assert "compliance_mode.yaml" in names


def test_main_pass_on_fixture():
    assert main([str(FIXTURES / "user_events.yaml")]) == 0


def test_main_fail_on_bad(tmp_path: Path):
    bad = tmp_path / "bad.yaml"
    bad.write_text("version: '1.0'\nname: x\n", encoding="utf-8")
    assert main([str(bad)]) == 1


def test_main_allow_empty(tmp_path: Path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    assert main(["--allow-empty"]) == 0
    assert main([]) == 1
