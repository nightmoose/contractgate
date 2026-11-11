#!/usr/bin/env python3
"""Read terraform output -json from examples/aws-rag/terraform (no secrets)."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

TF_DIR = Path(__file__).resolve().parents[1] / "terraform"


def load() -> dict:
    proc = subprocess.run(
        ["terraform", "output", "-json"],
        cwd=TF_DIR,
        check=True,
        capture_output=True,
        text=True,
    )
    raw = json.loads(proc.stdout)
    return {k: v.get("value") for k, v in raw.items()}


if __name__ == "__main__":
    print(json.dumps(load(), indent=2))
