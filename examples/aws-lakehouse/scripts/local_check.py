#!/usr/bin/env python3
"""Validate a 1k synthetic batch with no AWS calls."""
from __future__ import annotations

import lakehouse as lh

passed, failed, labels, kinds, p50 = lh.generate_and_validate(1000, 42)
assert len(passed) + len(failed) == 1000 and failed
print("local_ok", "passed", len(passed), "failed", len(failed), "p50_ns", p50)
print("labels", dict(labels))
