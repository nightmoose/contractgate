"""Asynchronous HTTP client.

Thin wrapper over ``httpx.AsyncClient``. Mirrors :py:class:`Client`
method-for-method; the only difference is that every entry point is
``async def`` and returns an awaitable.

Use as an async context manager so the underlying ``httpx.AsyncClient``
is closed cleanly:

    async with AsyncClient(base_url=..., api_key=...) as cg:
        result = await cg.ingest(contract_id="...", events=[...])
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


class AsyncClient:
    """Asynchronous client for the ContractGate gateway."""

    def __init__(
        self,
        *,
        base_url: str,
        api_key: Optional[str] = None,
        org_id: Optional[str] = None,
        timeout: float = _t.DEFAULT_TIMEOUT_S,
        transport: Optional[httpx.AsyncBaseTransport] = None,
    ) -> None:
        self._cfg = _t.TransportConfig(
            base_url=base_url,
            api_key=api_key,
            org_id=org_id,
            timeout=timeout,
        )
        self._http = httpx.AsyncClient(transport=transport, timeout=timeout)

    # ------------------------------------------------------------------
    # Async context-manager + cleanup
    # ------------------------------------------------------------------

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> "AsyncClient":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()

    # ------------------------------------------------------------------
    # Ingest
    # ------------------------------------------------------------------

    async def ingest(
        self,
        *,
        contract_id: str,
        events: Any,
        version: Optional[str] = None,
        dry_run: bool = False,
        atomic: bool = False,
        timeout: Optional[float] = None,
    ) -> BatchIngestResponse:
        spec = _t.build_ingest_request(
            self._cfg,
            contract_id,
            events,
            version=version,
            dry_run=dry_run,
            atomic=atomic,
            timeout=timeout,
        )
        _, body = await self._dispatch(spec)
        return BatchIngestResponse.from_dict(body)

    # ------------------------------------------------------------------
    # Audit / stats
    # ------------------------------------------------------------------

    async def audit(
        self,
        *,
        contract_id: Optional[str] = None,
        limit: int = 50,
        offset: int = 0,
        timeout: Optional[float] = None,
    ) -> List[AuditEntry]:
        spec = _t.build_audit_request(
            self._cfg,
            contract_id=contract_id,
            limit=limit,
            offset=offset,
            timeout=timeout,
        )
        _, body = await self._dispatch(spec)
        return [AuditEntry.from_dict(r) for r in _t.expect_list(body)]

    async def stats(self, *, timeout: Optional[float] = None) -> IngestionStats:
        spec = _t.build_global_stats_request(self._cfg, timeout=timeout)
        _, body = await self._dispatch(spec)
        return IngestionStats.from_dict(body)

    # ------------------------------------------------------------------
    # Contract reads
    # ------------------------------------------------------------------

    async def get_contract(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> ContractResponse:
        spec = _t.build_get_contract_request(self._cfg, contract_id, timeout=timeout)
        _, body = await self._dispatch(spec)
        return ContractResponse.from_dict(body)

    async def list_contracts(
        self, *, timeout: Optional[float] = None
    ) -> List[ContractResponse]:
        spec = _t.build_list_contracts_request(self._cfg, timeout=timeout)
        _, body = await self._dispatch(spec)
        return [ContractResponse.from_dict(r) for r in _t.expect_list(body)]

    async def list_versions(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> List[VersionSummary]:
        spec = _t.build_list_versions_request(self._cfg, contract_id, timeout=timeout)
        _, body = await self._dispatch(spec)
        return [VersionSummary.from_dict(r) for r in _t.expect_list(body)]

    async def get_version(
        self,
        contract_id: str,
        version: str,
        *,
        timeout: Optional[float] = None,
    ) -> VersionResponse:
        spec = _t.build_get_version_request(
            self._cfg, contract_id, version, timeout=timeout
        )
        _, body = await self._dispatch(spec)
        return VersionResponse.from_dict(body)

    async def get_latest_stable(
        self,
        contract_id: str,
        *,
        timeout: Optional[float] = None,
    ) -> VersionResponse:
        spec = _t.build_latest_stable_request(self._cfg, contract_id, timeout=timeout)
        _, body = await self._dispatch(spec)
        return VersionResponse.from_dict(body)

    # ------------------------------------------------------------------
    # Playground
    # ------------------------------------------------------------------

    async def playground_validate(
        self,
        *,
        yaml_content: str,
        event: Any,
        timeout: Optional[float] = None,
    ) -> Any:
        spec = _t.build_playground_request(
            self._cfg, yaml_content, event, timeout=timeout
        )
        _, body = await self._dispatch(spec)
        return body

    # ------------------------------------------------------------------
    # Internals
    # ------------------------------------------------------------------

    async def _dispatch(self, spec: _t.RequestSpec):
        try:
            r = await self._http.request(
                spec.method,
                spec.url,
                headers=spec.headers,
                params=spec.params,
                json=spec.json_body,
                timeout=spec.timeout,
            )
        except httpx.HTTPError as e:
            raise _ConnectionError(str(e)) from e
        return _t.decode_response(r.status_code, r.content)
