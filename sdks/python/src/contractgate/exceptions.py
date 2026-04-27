"""Exception hierarchy for the ContractGate SDK.

All SDK exceptions inherit from ``ContractGateError`` so users can
catch the entire surface with one ``except``. HTTP-shaped errors carry
the response status and decoded body (when JSON-decodable) for
debugging.

Note: per-event validation **failures** in a 207 Multi-Status response
do NOT raise. They surface in ``BatchIngestResponse.results``. Only
whole-batch rejects (422) raise ``ValidationFailedError``.
"""

from __future__ import annotations

from typing import Any, Optional


class ContractGateError(Exception):
    """Base class for every SDK exception."""


class ContractCompileError(ContractGateError):
    """Local contract YAML parse / compile failure.

    Raised by :py:meth:`Contract.from_yaml` and
    :py:meth:`Contract.compile`. Never raised by HTTP calls.
    """


class ConnectionError(ContractGateError):  # noqa: A001 — shadow stdlib intentionally inside this namespace
    """Network-level failure: DNS, connection refused, timeout."""


class HTTPError(ContractGateError):
    """Response-attached HTTP error.

    Subclasses by status range / semantic meaning. ``status`` is the
    integer HTTP status; ``body`` is the JSON-decoded response body
    when available, otherwise the raw text.
    """

    def __init__(
        self,
        message: str,
        *,
        status: int,
        body: Any = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.body = body


class BadRequestError(HTTPError):
    """400 — malformed input, oversized batch, bad UUID."""


class AuthError(HTTPError):
    """401 — missing or invalid ``x-api-key``."""


class NotFoundError(HTTPError):
    """404 — contract or version not found."""


class ConflictError(HTTPError):
    """409 — typically ``NoStableVersion`` for an unpinned ingest.

    The contract has no stable version yet; ingest must specify a
    version pin via header or path suffix.
    """


class ValidationFailedError(HTTPError):
    """422 — whole-batch rejection.

    Raised when the gateway rejects an entire batch:
    - ``atomic=true`` and any event failed,
    - all events failed validation, or
    - the batch was pinned to a deprecated version.

    Per-event failures in a mixed 207 batch do NOT raise.
    """


class ServerError(HTTPError):
    """5xx — server fault. Retry with backoff if appropriate."""


def status_to_exception(status: int) -> type[HTTPError]:
    """Map an HTTP status to the most specific ``HTTPError`` subclass."""
    if status == 400:
        return BadRequestError
    if status == 401:
        return AuthError
    if status == 404:
        return NotFoundError
    if status == 409:
        return ConflictError
    if status == 422:
        return ValidationFailedError
    if 500 <= status < 600:
        return ServerError
    return HTTPError


def raise_for_status(status: int, body: Any, default_message: Optional[str] = None) -> None:
    """Raise the appropriate ``HTTPError`` subclass for ``status``.

    No-op for 2xx responses. Used by the transport layer.
    """
    if 200 <= status < 300:
        return
    cls = status_to_exception(status)
    msg = default_message or _extract_error_message(body) or f"HTTP {status}"
    raise cls(msg, status=status, body=body)


def _extract_error_message(body: Any) -> Optional[str]:
    """Pull a human message out of a JSON error body, if present."""
    if isinstance(body, dict):
        for key in ("error", "message", "detail"):
            v = body.get(key)
            if isinstance(v, str):
                return v
    return None
