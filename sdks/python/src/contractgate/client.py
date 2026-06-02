"""Synchronous HTTP client.

Thin wrapper over ``httpx.Client``. All request building and response
decoding lives in ``_transport.py`` so the sync and async clients
cannot drift on auth, headers, URL shape, or error mapping.
"""

from __future__ import annotations

from typing import Any, List, Optional

import httpx

from contractgate import _transport as _t
from contractgate.exceptions import ConnectionError as _ConnectionError
from contractgate.models import (
    AuditEntry,
    BatchIngestResponse,
    ContractResponse,
    IngestionStats,
    VersionResponse,
    VersionSummary,
)


class Client:
    """Synchronous client for the ContractGate gateway.

    Construct with the gateway's ``base_url`` and your API key:

        cg = Client(base_url="https://gw.example.com", api_key="cg_live_...")

    The client owns an ``httpx.Client`` for connection pooling. Close
    it explicitly with :py:meth:`close` or use as a context manager.
    """

    def __init__(
        self,
        *,
        base_url: str,
        api_key: Optional[str] = None,
        org_id: Optional[str] = None,
        timeout: float = _t.DEFAULT_TIMEOUT_S,
        transport: Optional[httpx.BaseTransport] = None,
    ) -> None:
        self._cfg = _t.TransportConfig(
            base_url=base_url,
            api_key=api_key,
            org_id=org_id,
            timeout=timeout,
        )
        # Keep a single ``httpx.Client`` so connection pooling works.
        # ``transport`` is exposed as a kwarg so test suites (and the
        # ``pytest-httpx`` plugin) can swap in a mock without
        # monkeypatching.
        self._http = httpx.Client(transport=transport, timeout=timeout)

    # ------------------------------------------------------------------
    # Context-manager + cleanup
    # ------------------------------------------------------------------

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    # ------------------------------------------------------------------
    # Ingest
    # ------------------------------------------------------------------

    def ingest(
        self,
        *,
        contract_id: str,
        events: Any,
        version: Optional[str] = None,
        dry_run: bool = False,
        atomic: bool = False,
        timeout: Optional[float] = None,
    ) -> BatchIngestResponse:
        """Validate + persist a batch via ``POST /ingest/{contract_id}``.

        ``events`` may be a single dict or a list of dicts. ``version``
        pins to a specific contract version via the
        ``X-Contract-Version`` header (RFC-002).

        Per-event failures in a 207 Multi-Status response do NOT
        raise — they appear in ``response.results``. Whole-batch
        rejections (422) raise ``ValidationFailedError``.
        """
        spec = _t.build_ingest_request(
            self._cfg,
            contract_id,
            events,
            version=version,
            dry_run=dry_run,
            atomic=atomic,
            timeout=timeout,
        )
        _, body = self._dispatch(spec)
        return BatchIngestResponse.from_dict(body)

    # ------------------------------------------------------------------
    # Audit / stats
    # ------------------------------------------------------------------

    def audit(
        self,
        *,
        contract_id: Optional[str] = None,
        limit: int = 50,
        offset: int = 0,
        timeout: Optional[float] = None,
    ) -> List[AuditEntry]:
        """Read recent audit entries. Latest first.

        Server caps ``limit`` at 500. Use ``offset`` to paginate.
        """
        spec = _t.build_audit_request(
            self._cfg,
            contract_id=contract_id,
            limit=limit,
            offset=offset,
            timeout=timeout,
        )
        _, body = self._dispatch(spec)
        return [AuditEntry.from_dict(r) for r in _t.expect_list(body)]

    def stats(self, *, timeout: Optional[float] = None) -> IngestionStats:
        """Global ingestion stats for the caller's org."""
        spec = _t.build_global_stats_request(self._cfg, timeout=timeout)
        _, body = self._dispatch(spec)
        return IngestionStats.from_dict(body)

    # ------------------------------------------------------------------
    # Contract reads
    # ------------------------------------------------------------------

    def get_contract(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> ContractResponse:
        spec = _t.build_get_contract_request(self._cfg, contract_id, timeout=timeout)
        _, body = self._dispatch(spec)
        return ContractResponse.from_dict(body)

    def list_contracts(self, *, timeout: Optional[float] = None) -> List[ContractResponse]:
        spec = _t.build_list_contracts_request(self._cfg, timeout=timeout)
        _, body = self._dispatch(spec)
        # ``GET /contracts`` returns ``ContractSummary`` rows server-side
        # but the response shape is a strict subset of ``ContractResponse``;
        # ``from_dict`` tolerates the missing fields.
        return [ContractResponse.from_dict(r) for r in _t.expect_list(body)]

    def list_versions(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> List[VersionSummary]:
        spec = _t.build_list_versions_request(self._cfg, contract_id, timeout=timeout)
        _, body = self._dispatch(spec)
        return [VersionSummary.from_dict(r) for r in _t.expect_list(body)]

    def get_version(
        self,
        contract_id: str,
        version: str,
        *,
        timeout: Optional[float] = None,
    ) -> VersionResponse:
        spec = _t.build_get_version_request(
            self._cfg, contract_id, version, timeout=timeout
        )
        _, body = self._dispatch(spec)
        return VersionResponse.from_dict(body)

    def get_latest_stable(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> VersionResponse:
        spec = _t.build_latest_stable_request(self._cfg, contract_id, timeout=timeout)
        _, body = self._dispatch(spec)
        return VersionResponse.from_dict(body)

    # ------------------------------------------------------------------
    # Playground (dry validate without persisting)
    # ------------------------------------------------------------------

    def playground_validate(
        self,
        *,
        yaml_content: str,
        event: Any,
        timeout: Optional[float] = None,
    ) -> Any:
        """Validate an in-flight contract YAML against an event.

        Returns the raw playground response — a JSON object containing
        ``passed``, ``violations``, ``validation_us``, and
        ``transformed_event`` (the dashboard's "what we'd store" view).
        Useful for pre-flighting a contract change before saving it.
        """
        spec = _t.build_playground_request(
            self._cfg, yaml_content, event, timeout=timeout
        )
        _, body = self._dispatch(spec)
        return body

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    def _dispatch(self, spec: _t.RequestSpec):
        """Send ``spec`` through ``httpx`` and decode the response."""
        try:
            r = self._http.request(
                spec.method,
                spec.url,
                headers=spec.headers,
                params=spec.params,
                json=spec.json_body,
                timeout=spec.timeout,
            )
        except httpx.HTTPError as e:
            # Wrap transport-level errors so callers don't have to
            # catch httpx's exception hierarchy in addition to ours.
            raise _ConnectionError(str(e)) from e
        return _t.decode_response(r.status_code, r.content)
