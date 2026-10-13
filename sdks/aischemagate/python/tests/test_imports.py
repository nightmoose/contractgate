"""Import-shape tests for the ``aischemagate`` alias package.

The alias is a pure re-export of ``contractgate`` + ``contractgate.llm``.
These tests assert every documented symbol is importable from
``aischemagate`` and resolves to the same object as its ``contractgate``
counterpart.
"""

from __future__ import annotations

import pytest


def test_version_is_pinned():
    import aischemagate

    assert aischemagate.__version__ == "0.3.1"
    # The alias also exposes the pinned contractgate version so users can
    # confirm the floor at runtime without a second import.
    assert aischemagate.__contractgate_version__.startswith("0.3")


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

    with pytest.raises(ValueError, match="requires an `llm` client"):
        LLMGate(api_key="k", contract_id="c", llm=None)
