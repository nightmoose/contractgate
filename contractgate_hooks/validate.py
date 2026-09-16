"""Parse + compile ContractGate YAML. No network.

Used by the pre-commit hook and the GitHub Action. Depends on the
published ``contractgate`` SDK (``Contract.from_yaml`` + ``compile``).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Iterable, List, Sequence

from contractgate.contract import Contract
from contractgate.exceptions import ContractCompileError

DEFAULT_DIR = Path("contracts")
YAML_SUFFIXES = {".yaml", ".yml"}


def collect_paths(paths: Sequence[str], directory: str | None) -> List[Path]:
    found: List[Path] = []
    for raw in paths:
        p = Path(raw)
        if p.is_dir():
            found.extend(_yaml_under(p))
        elif p.is_file():
            found.append(p)
        else:
            raise FileNotFoundError(f"not a file or directory: {p}")
    if directory:
        d = Path(directory)
        if not d.is_dir():
            raise FileNotFoundError(f"directory not found: {d}")
        found.extend(_yaml_under(d))
    # De-dupe while preserving order
    seen = set()
    unique: List[Path] = []
    for p in found:
        resolved = p.resolve()
        if resolved not in seen:
            seen.add(resolved)
            unique.append(p)
    return unique


def _yaml_under(root: Path) -> Iterable[Path]:
    for p in sorted(root.rglob("*")):
        if p.is_file() and p.suffix.lower() in YAML_SUFFIXES:
            yield p


def compile_file(path: Path) -> None:
    text = path.read_text(encoding="utf-8")
    Contract.from_yaml(text).compile()


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="contractgate-validate",
        description="Parse and compile ContractGate YAML contracts (local, no network).",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="Contract YAML files or directories. Defaults to ./contracts if omitted.",
    )
    parser.add_argument(
        "--dir",
        dest="directory",
        default=None,
        help="Also glob **/*.{yaml,yml} under this directory.",
    )
    parser.add_argument(
        "--allow-empty",
        action="store_true",
        help="Exit 0 when no files match (default: exit 1).",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    try:
        files = collect_paths(args.paths, args.directory)
    except FileNotFoundError as e:
        print(f"FAIL  {e}", file=sys.stderr)
        return 1

    if not files and not args.paths and args.directory is None:
        if DEFAULT_DIR.is_dir():
            files = list(_yaml_under(DEFAULT_DIR))
        elif args.allow_empty:
            print("PASS  no contract files")
            return 0
        else:
            print(f"FAIL  no contract files matched (looked for {DEFAULT_DIR}/)", file=sys.stderr)
            return 1

    if not files:
        if args.allow_empty:
            print("PASS  no contract files")
            return 0
        print("FAIL  no contract files matched", file=sys.stderr)
        return 1

    failed = 0
    for path in files:
        try:
            compile_file(path)
        except ContractCompileError as e:
            failed += 1
            print(f"FAIL  {path}\n      {e}", file=sys.stderr)
        except Exception as e:  # noqa: BLE001 — surface unexpected parse IO to CI
            failed += 1
            print(f"FAIL  {path}\n      {e}", file=sys.stderr)
        else:
            print(f"PASS  {path}")

    if failed:
        print(f"{failed}/{len(files)} contract(s) failed compile", file=sys.stderr)
        return 1
    print(f"{len(files)} contract(s) ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
