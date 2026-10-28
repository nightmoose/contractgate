"""Import-shape tests for the ``aischemagate`` package.

The root import re-exports ``contractgate`` + ``contractgate.llm``.
These tests assert every documented symbol is importable from
``aischemagate`` and resolves to the same object as its ``contractgate``
counterpart. Framework adapters are optional submodules and stay unloaded.
"""

from __future__ import annotations

import pytest


def test_version_is_pinned():
    import aischemagate

    assert aischemagate.__version__ == "0.5.0"
    # The alias also exposes the pinned contractgate version so users can
    # confirm the floor at runtime without a second import.
    assert aischemagate.__contractgate_version__.startswith("0.4")


def test_llmgate_symbols_available():
    from aischemagate import (
        AsyncLLMGate,
        LLMGate,
        LLMGateResult,
        LLMResponseError,
        MAX_RETRIES_CAP,
        extract_json,
    )

    assert MAX_RETRIES_CAP == 5
    assert LLMGate.__name__ == "LLMGate"
    assert AsyncLLMGate.__name__ == "AsyncLLMGate"
    assert LLMGateResult.__name__ == "LLMGateResult"
    assert issubclass(LLMResponseError, Exception)
    assert callable(extract_json)


def test_llmgate_is_identical_to_contractgate_llmgate():
    """Alias re-exports MUST be the same object, not a copy."""
    import aischemagate
    import contractgate.llm

    assert aischemagate.LLMGate is contractgate.llm.LLMGate
    assert aischemagate.AsyncLLMGate is contractgate.llm.AsyncLLMGate
    assert aischemagate.LLMResponseError is contractgate.llm.LLMResponseError
    assert aischemagate.extract_json is contractgate.llm.extract_json


def test_core_contractgate_symbols_available():
    """Every exported symbol in `contractgate` must be reachable via `aischemagate`.

    ``__version__`` is intentionally per-package (aischemagate ships its own
    version track; the pinned `contractgate` floor is exposed separately as
    ``aischemagate.__contractgate_version__``).
    """
    import aischemagate
    import contractgate

    for name in contractgate.__all__:
        if name == "__version__":
            continue
        assert hasattr(aischemagate, name), f"aischemagate is missing: {name}"
        assert getattr(aischemagate, name) is getattr(contractgate, name), (
            f"aischemagate.{name} is not the same object as contractgate.{name}"
        )


def test_client_construction_works_through_alias():
    """Smoke test: LLMGate constructed via the alias works identically."""
    from aischemagate import LLMGate

    class _FakeLLM:
        pass

    gate = LLMGate(api_key="k", contract_id="c", llm=_FakeLLM())
    assert gate is not None


def test_llm_required_message_still_fires():
    from aischemagate import LLMGate

    gate = LLMGate(api_key="k", contract_id="c")  # llm is optional for validate_only
    with pytest.raises(ValueError, match="need an `llm` client"):
        gate.completion(model="m", messages=[])


def test_root_import_does_not_load_framework_adapters():
    """Optional extras stay unloaded until their submodule is imported."""
    import subprocess
    import sys

    script = """
import sys
import aischemagate
forbidden = ("langchain_core", "llama_index", "instructor", "aischemagate.langchain", "aischemagate.llamaindex", "aischemagate.instructor")
loaded = [name for name in forbidden if name in sys.modules]
if loaded:
    raise SystemExit("root import loaded " + ", ".join(loaded))
if hasattr(aischemagate, "AISchemaGateParser"):
    raise SystemExit("parser must not be a root export")
"""
    completed = subprocess.run([sys.executable, "-c", script], check=False, capture_output=True, text=True)
    assert completed.returncode == 0, completed.stdout + completed.stderr
