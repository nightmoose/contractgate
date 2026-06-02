"""Pytest configuration shared across SDK tests."""

from __future__ import annotations

import pathlib
import sys

# Make ``src/`` importable when running ``pytest`` from the SDK root
# without first running ``pip install -e .``. CI installs the package,
# but local dev iterates faster without the pip step.
SRC = pathlib.Path(__file__).resolve().parent.parent / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))
