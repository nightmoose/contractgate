"""``contractgate-py review`` is a thin wrapper around the Rust CLI."""

from __future__ import annotations

import os
from pathlib import Path

from contractgate.cli import main


def test_review_help() -> None:
    # argparse help exits 0 via SystemExit
    try:
        main(["review", "--help"])
    except SystemExit as e:
        assert e.code == 0
    else:
        raise AssertionError("expected SystemExit from --help")


def test_review_errors_when_rust_cli_missing(tmp_path: Path, monkeypatch) -> None:
    csv = tmp_path / "t.csv"
    csv.write_text("id,email\n1,a@b.co\n")
    monkeypatch.setenv("PATH", str(tmp_path))  # empty of contractgate
    # Ensure which() cannot find a leftover binary via other env
    monkeypatch.delenv("PATHEXT", raising=False)
    code = main(["review", str(csv), "--out", str(tmp_path / "out")])
    assert code == 2
