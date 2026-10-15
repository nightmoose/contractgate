"""Minimal OpenMetadata REST client (just what the integration needs)."""

from __future__ import annotations

from typing import Optional

from . import http

TEST_DEFINITION = "dcgIngestContract"


class OmClient:
    def __init__(self, host: str, jwt: str, timeout: float = 30.0):
        host = host.rstrip("/")
        self.api = host if host.endswith("/api") else host + "/api"
        self.jwt = jwt
        self.timeout = timeout

    def _req(self, method: str, path: str, body=None, raw: bool = False,
             extra_headers: Optional[dict] = None):
        headers = {"Authorization": f"Bearer {self.jwt}"}
        headers.update(extra_headers or {})
        return http.request(method, self.api + path, headers, body, raw=raw, timeout=self.timeout)

    def _get_or_none(self, path: str) -> Optional[dict]:
        try:
            return self._req("GET", path)
        except http.HttpError as e:
            if e.status == 404:
                return None
            raise

    # -- entities -------------------------------------------------------
    def get_table(self, fqn: str) -> Optional[dict]:
        return self._get_or_none(f"/v1/tables/name/{http.quote(fqn)}")

    def ensure_test_definition(self, name: str = TEST_DEFINITION) -> dict:
        existing = self._get_or_none(f"/v1/dataQuality/testDefinitions/name/{http.quote(name)}")
        if existing:
            return existing
        return self._req(
            "POST",
            "/v1/dataQuality/testDefinitions",
            {
                "name": name,
                "displayName": "DataContractGate ingest contract",
                "description": (
                    "Results pushed by DataContractGate (DCG). DCG validates every event "
                    "against its contract at ingest time and quarantines violations before "
                    "they land. Each result is one validation window."
                ),
                "entityType": "TABLE",
                "testPlatforms": ["Other"],
                "parameterDefinition": [
                    {"name": "dcgContractId", "dataType": "STRING", "required": False,
                     "description": "DataContractGate contract UUID"},
                    {"name": "maxViolationRate", "dataType": "NUMBER", "required": False,
                     "description": "Violation rate above which the result is Failed"},
                ],
            },
        )

    def ensure_test_case(
        self, table_fqn: str, name: str, contract_id: str, max_violation_rate: float,
        description: str = "", definition: str = TEST_DEFINITION,
    ) -> dict:
        fqn = f"{table_fqn}.{name}"
        existing = self._get_or_none(f"/v1/dataQuality/testCases/name/{http.quote(fqn)}")
        if existing:
            return existing
        return self._req(
            "POST",
            "/v1/dataQuality/testCases",
            {
                "name": name,
                "displayName": name,
                "description": description,
                "entityLink": f"<#E::table::{table_fqn}>",
                "testDefinition": definition,
                "parameterValues": [
                    {"name": "dcgContractId", "value": contract_id},
                    {"name": "maxViolationRate", "value": str(max_violation_rate)},
                ],
            },
        )

    def post_result(self, test_case_fqn: str, result: dict) -> dict:
        return self._req(
            "POST",
            f"/v1/dataQuality/testCases/testCaseResults/{http.quote(test_case_fqn)}",
            result,
        )

    # -- data contracts (ODCS) -------------------------------------------
    def get_contract_odcs_yaml(self, contract_id: str) -> str:
        return self._req("GET", f"/v1/dataContracts/{contract_id}/odcs/yaml", raw=True,
                         extra_headers={"Accept": "application/yaml"})
