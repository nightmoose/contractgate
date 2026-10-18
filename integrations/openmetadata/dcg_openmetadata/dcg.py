"""Minimal DataContractGate API client."""

from __future__ import annotations

import urllib.parse
from typing import Optional

from . import http


class DcgClient:
    def __init__(self, base_url: str, api_key: Optional[str] = None, timeout: float = 30.0,
                 org_id: Optional[str] = None):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout
        # Only honoured by a local gateway running with CONTRACTGATE_DEV_NO_AUTH=1.
        self.org_id = org_id

    def _headers(self) -> dict:
        h = {"x-api-key": self.api_key} if self.api_key else {}
        if self.org_id:
            h["x-org-id"] = self.org_id
        return h

    def _get(self, path: str, params: Optional[dict] = None, raw: bool = False):
        url = self.base_url + path
        if params:
            url += "?" + urllib.parse.urlencode({k: v for k, v in params.items() if v is not None})
        return http.request("GET", url, self._headers(), raw=raw, timeout=self.timeout)

    def get_contract(self, contract_id: str) -> dict:
        """GET /contracts/{id} -> {id, name, description, latest_stable_version, ...}."""
        return self._get(f"/contracts/{contract_id}")

    def list_versions(self, contract_id: str) -> list:
        return self._get(f"/contracts/{contract_id}/versions")

    def get_version(self, contract_id: str, version: str) -> dict:
        return self._get(f"/contracts/{contract_id}/versions/{http.quote(version)}")

    def report(self, contract_id: str, since: Optional[str] = None, until: Optional[str] = None) -> dict:
        """GET /contracts/{id}/report -> windowed totals + by_version + top_violations."""
        return self._get(
            f"/contracts/{contract_id}/report", {"from": since, "to": until, "format": "json"}
        )

    def export_odcs(self, contract_id: str, version: str) -> str:
        """GET /contracts/{id}/versions/{version}/export -> ODCS v3.1.0 YAML."""
        return self._get(f"/contracts/{contract_id}/versions/{http.quote(version)}/export", raw=True)

    def import_odcs(
        self,
        odcs_yaml: str,
        target_contract_id: Optional[str] = None,
        name_override: Optional[str] = None,
    ) -> dict:
        """POST /contracts/import. With target_contract_id the doc becomes a new draft version."""
        body = {"odcs_yaml": odcs_yaml}
        if target_contract_id:
            body["target_contract_id"] = target_contract_id
        if name_override:
            body["name_override"] = name_override
        return http.request(
            "POST", self.base_url + "/contracts/import", self._headers(), body, timeout=self.timeout
        )
