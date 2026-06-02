"""Shared transport wiring for sync + async HTTP clients.

Centralizes:
  - URL building (``base_url`` + path joining),
  - default headers (``x-api-key``, optional ``x-org-id``,
    ``User-Agent``),
  - request shaping for ingest, audit, contract reads,
  - response decode + error mapping (``status_to_exception``).

Both ``Client`` (httpx.Client) and ``AsyncClient`` (httpx.AsyncClient)
delegate request building here so they cannot drift on auth or path
shape. They differ only in *how* the request is dispatched (sync vs
``await``).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, List, Mapping, Optional, Tuple

from contractgate._version import __version__
from contractgate.exceptions import raise_for_status

# Default timeout matches the gateway's own 30s upper bound (see
# ``TimeoutLayer`` in ``src/main.rs``). Callers can override per-call.
DEFAULT_TIMEOUT_S = 30.0

USER_AGENT = f"contractgate-python/{__version__}"


@dataclass(frozen=True)
class TransportConfig:
    base_url: str
    api_key: Optional[str]
    org_id: Optional[str]
    timeout: float

    def headers(self, extra: Optional[Mapping[str, str]] = None) -> Dict[str, str]:
        h: Dict[str, str] = {
            "User-Agent": USER_AGENT,
            "Accept": "application/json",
        }
        if self.api_key:
            h["x-api-key"] = self.api_key
        if self.org_id:
            h["x-org-id"] = self.org_id
        if extra:
            h.update(extra)
        return h

    def url(self, path: str) -> str:
        # ``base_url`` may or may not have a trailing slash; ``path`` must
        # always start with one. Defensive join — httpx accepts both forms.
        base = self.base_url.rstrip("/")
        if not path.startswith("/"):
            path = "/" + path
        return base + path


# ---------------------------------------------------------------------------
# Request specs (built sync, dispatched by either client)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class RequestSpec:
    """A fully-built request — what the sync/async client dispatches."""

    method: str
    url: str
    headers: Dict[str, str]
    params: Optional[Dict[str, Any]]
    json_body: Optional[Any]
    timeout: float


def build_ingest_request(
    cfg: TransportConfig,
    contract_id: str,
    events: Any,
    *,
    version: Optional[str] = None,
    dry_run: bool = False,
    atomic: bool = False,
    timeout: Optional[float] = None,
) -> RequestSpec:
    """Build a POST to ``/ingest/{contract_id}[@version]``.

    ``events`` may be a single dict (the gateway treats a non-array
    body as a one-event batch) or a list. ``version`` becomes the
    ``X-Contract-Version`` header so the gateway can resolve to a
    specific pin (header > path-suffix > default-stable, per RFC-002).
    """
    path = f"/ingest/{contract_id}"
    extra_headers: Dict[str, str] = {}
    if version is not None:
        # Header takes precedence over path-suffix per RFC-002. We use
        # the header form by default — clients that need the @version
        # path form for environments that strip headers can pass
        # ``contract_id="<uuid>@<version>"`` and leave ``version=None``.
        extra_headers["X-Contract-Version"] = version
    params: Dict[str, Any] = {}
    if dry_run:
        params["dry_run"] = "true"
    if atomic:
        params["atomic"] = "true"
    return RequestSpec(
        method="POST",
        url=cfg.url(path),
        headers=cfg.headers(extra_headers),
        params=params or None,
        json_body=events,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_audit_request(
    cfg: TransportConfig,
    *,
    contract_id: Optional[str] = None,
    limit: int = 50,
    offset: int = 0,
    timeout: Optional[float] = None,
) -> RequestSpec:
    params: Dict[str, Any] = {"limit": str(limit), "offset": str(offset)}
    if contract_id is not None:
        params["contract_id"] = contract_id
    return RequestSpec(
        method="GET",
        url=cfg.url("/audit"),
        headers=cfg.headers(),
        params=params,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_get_contract_request(
    cfg: TransportConfig,
    contract_id: str,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url(f"/contracts/{contract_id}"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_list_contracts_request(
    cfg: TransportConfig,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url("/contracts"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_get_version_request(
    cfg: TransportConfig,
    contract_id: str,
    version: str,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url(f"/contracts/{contract_id}/versions/{version}"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_list_versions_request(
    cfg: TransportConfig,
    contract_id: str,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url(f"/contracts/{contract_id}/versions"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_latest_stable_request(
    cfg: TransportConfig,
    contract_id: str,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url(f"/contracts/{contract_id}/versions/latest-stable"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_global_stats_request(
    cfg: TransportConfig,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="GET",
        url=cfg.url("/stats"),
        headers=cfg.headers(),
        params=None,
        json_body=None,
        timeout=timeout if timeout is not None else cfg.timeout,
    )


def build_playground_request(
    cfg: TransportConfig,
    yaml_content: str,
    event: Any,
    *,
    timeout: Optional[float] = None,
) -> RequestSpec:
    return RequestSpec(
        method="POST",
        url=cfg.url("/playground/validate"),
        headers=cfg.headers({"Content-Type": "application/json"}),
        params=None,
        json_body={"yaml_content": yaml_content, "event": event},
        timeout=timeout if timeout is not None else cfg.timeout,
    )


# ---------------------------------------------------------------------------
# Response decoding
# ---------------------------------------------------------------------------


def decode_response(status: int, raw_body: bytes) -> Tuple[int, Any]:
    """Decode the JSON body (best-effort) and raise on non-2xx.

    Returns ``(status, decoded_body)``. ``decoded_body`` is whatever
    JSON parsing produced; if parsing fails the raw text is returned
    instead so the error message stays useful.
    """
    text = raw_body.decode("utf-8", errors="replace") if raw_body else ""
    body: Any = None
    if text:
        try:
            body = json.loads(text)
        except json.JSONDecodeError:
            body = text
    raise_for_status(status, body)
    return status, body


def expect_list(body: Any) -> List[Any]:
    """Coerce a list response, defending against unexpected null."""
    if body is None:
        return []
    if isinstance(body, list):
        return body
    raise ValueError(f"expected list response, got {type(body).__name__}")
