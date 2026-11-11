#!/usr/bin/env python3
"""Copy ContractGate docs and catalog seeds into corpus/build with stable ids.

Ids are repo-relative paths (README.md, docs/..., catalog/seeds/...).
Those ids are what the eval golden set uses for hit@k.
"""
from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # contractgate repo root
OUT = Path(__file__).resolve().parents[1] / "corpus" / "build"
INCLUDE_SUFFIXES = {".md", ".txt", ".yaml", ".yml", ".json"}
SKIP_PARTS = {"rfcs", "images", "data-room", "node_modules", ".git"}


def should_keep(path: Path) -> bool:
    if path.suffix.lower() not in INCLUDE_SUFFIXES:
        return False
    if any(part in SKIP_PARTS for part in path.parts):
        return False
    return True


def collect() -> list[dict]:
    files: list[Path] = []
    readme = ROOT / "README.md"
    if readme.exists():
        files.append(readme)
    docs = ROOT / "docs"
    if docs.exists():
        files.extend(p for p in docs.rglob("*") if p.is_file() and should_keep(p))
    seeds = ROOT / "catalog" / "seeds"
    if seeds.exists():
        files.extend(p for p in seeds.rglob("*") if p.is_file() and should_keep(p))
    unique = sorted(set(files))
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    manifest = []
    for src in unique:
        rel = src.relative_to(ROOT).as_posix()
        dest = OUT / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(src.read_bytes())
        text = src.read_text(errors="replace")
        manifest.append(
            {
                "id": rel,
                "path": rel,
                "bytes": src.stat().st_size,
                "chars": len(text),
                "approx_tokens": max(1, len(text) // 4),
            }
        )
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main() -> int:
    manifest = collect()
    tokens = sum(m["approx_tokens"] for m in manifest)
    bytes_ = sum(m["bytes"] for m in manifest)
    print(f"Wrote {len(manifest)} files to {OUT} ({bytes_} bytes, ~{tokens} tokens)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
