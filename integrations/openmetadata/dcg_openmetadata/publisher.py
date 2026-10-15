"""Publish DCG contract validation stats to OpenMetadata as test case results."""

from __future__ import annotations

import json
import logging
import re
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import List, Optional

from .dcg import DcgClient
from .om import TEST_DEFINITION, OmClient

log = logging.getLogger("dcg_openmetadata")

# Contract-metadata mapping: a top-level `x-openmetadata-table: <fqn>` key in the
# contract YAML, or `openmetadata-table: <fqn>` anywhere in the description.
_YAML_KEY = re.compile(r"^x-openmetadata-table:\s*['\"]?([^'\"\s#]+)", re.M)
_DESC_KEY = re.compile(r"openmetadata-table:\s*['\"]?([^'\"\s]+)")


@dataclass
class Mapping:
    contract_id: str
    table_fqn: Optional[str] = None
    test_case_name: Optional[str] = None
    max_violation_rate: float = 0.0


@dataclass
class Config:
    mappings: List[Mapping] = field(default_factory=list)
    window_minutes: Optional[int] = None  # None = all-time totals
    skip_empty: bool = True
    test_definition: str = TEST_DEFINITION


def load_config(path: str) -> Config:
    with open(path) as f:
        raw = json.load(f)
    defaults = raw.get("defaults", {})
    maps = []
    for m in raw.get("contracts", []):
        if "contract_id" not in m:
            raise ValueError(f"mapping entry missing contract_id: {m}")
        maps.append(Mapping(
            contract_id=m["contract_id"],
            table_fqn=m.get("table_fqn"),
            test_case_name=m.get("test_case_name"),
            max_violation_rate=float(m.get("max_violation_rate",
                                           defaults.get("max_violation_rate", 0.0))),
        ))
    return Config(
        mappings=maps,
        window_minutes=raw.get("window_minutes", defaults.get("window_minutes")),
        skip_empty=bool(raw.get("skip_empty", True)),
        test_definition=raw.get("test_definition", TEST_DEFINITION),
    )


def fqn_from_metadata(contract: dict, yaml_content: Optional[str]) -> Optional[str]:
    if yaml_content:
        m = _YAML_KEY.search(yaml_content)
        if m:
            return m.group(1)
    m = _DESC_KEY.search(contract.get("description") or "")
    return m.group(1) if m else None


def default_test_name(contract_name: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9_]+", "_", contract_name).strip("_")
    return f"dcg_{slug}"[:256]


def pick_version(report: dict, contract: dict) -> str:
    """Version that saw the most traffic in the window, else latest stable."""
    rows = [r for r in report.get("by_version", []) if r.get("contract_version")]
    if rows:
        return max(rows, key=lambda r: r.get("total", 0))["contract_version"]
    return contract.get("latest_stable_version") or "unknown"


def build_result(report: dict, version: str, max_violation_rate: float,
                 now_ms: Optional[int] = None) -> dict:
    t = report.get("totals", {})
    total = int(t.get("total", 0))
    passed = int(t.get("passed", 0))
    quarantined = int(t.get("quarantined", 0))
    rate = quarantined / total if total else 0.0
    status = "Failed" if rate > max_violation_rate else "Success"
    top = report.get("top_violations", [])[:3]
    top_txt = ", ".join(f"{v.get('field')}:{v.get('kind')} x{v.get('count')}" for v in top)
    msg = (f"DCG contract {report.get('contract_name')} v{version}: {passed}/{total} passed, "
           f"{quarantined} quarantined (violation rate {rate:.2%}, threshold "
           f"{max_violation_rate:.2%}).")
    if top_txt:
        msg += f" Top violations: {top_txt}."
    return {
        "timestamp": now_ms if now_ms is not None else int(time.time() * 1000),
        "testCaseStatus": status,
        "result": msg,
        "testResultValue": [
            {"name": "total", "value": str(total)},
            {"name": "passed", "value": str(passed)},
            {"name": "quarantined", "value": str(quarantined)},
            {"name": "violation_rate", "value": f"{rate:.6f}"},
            {"name": "contract_version", "value": version},
        ],
        "passedRows": passed,
        "failedRows": quarantined,
        "passedRowsPercentage": (passed / total * 100) if total else 100.0,
        "failedRowsPercentage": rate * 100,
    }


def publish(cfg: Config, dcg: DcgClient, om: OmClient, dry_run: bool = False,
            now: Optional[datetime] = None) -> List[dict]:
    """Publish one result per mapping. Returns a summary row per contract."""
    now = now or datetime.now(timezone.utc)
    since = None
    if cfg.window_minutes:
        since = (now - timedelta(minutes=cfg.window_minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")
    if not dry_run:
        om.ensure_test_definition(cfg.test_definition)
    out = []
    for m in cfg.mappings:
        row = {"contract_id": m.contract_id}
        try:
            contract = dcg.get_contract(m.contract_id)
            fqn = m.table_fqn
            if not fqn:
                yaml_content = None
                if contract.get("latest_stable_version"):
                    v = dcg.get_version(m.contract_id, contract["latest_stable_version"])
                    yaml_content = v.get("yaml_content")
                fqn = fqn_from_metadata(contract, yaml_content)
            if not fqn:
                raise ValueError("no table_fqn in mapping and no x-openmetadata-table in contract")
            report = dcg.report(m.contract_id, since=since)
            total = int(report.get("totals", {}).get("total", 0))
            name = m.test_case_name or default_test_name(contract.get("name", m.contract_id))
            row.update(table_fqn=fqn, test_case=f"{fqn}.{name}")
            if total == 0 and cfg.skip_empty:
                row["status"] = "skipped (no events in window)"
                out.append(row)
                continue
            result = build_result(report, pick_version(report, contract),
                                  m.max_violation_rate, int(now.timestamp() * 1000))
            row["status"] = result["testCaseStatus"]
            row["result"] = result["result"]
            if not dry_run:
                if om.get_table(fqn) is None:
                    raise ValueError(f"OpenMetadata table not found: {fqn}")
                om.ensure_test_case(
                    fqn, name, m.contract_id, m.max_violation_rate,
                    description=f"Ingest-time enforcement of DataContractGate contract "
                                f"`{contract.get('name')}` ({m.contract_id}).",
                    definition=cfg.test_definition,
                )
                om.post_result(f"{fqn}.{name}", result)
        except Exception as e:  # keep going for the other contracts
            log.error("contract %s: %s", m.contract_id, e)
            row["status"] = "error"
            row["error"] = str(e)
        out.append(row)
    return out
