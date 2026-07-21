#!/usr/bin/env python3
"""
Cloud path: create contract, promote version, ingest pass/fail, fetch report.

Requires CG_API_KEY (and optional CG_API_URL).
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import httpx
import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (
    FINDINGS,
    env_api,
    fixture_dir,
    load_scenario,
    read_ndjson,
    write_json,
)


class CloudClient:
    def __init__(self, base: str, key: str):
        self.base = base.rstrip("/")
        self.headers = {
            "X-Api-Key": key,
            "Accept": "application/json",
            "Content-Type": "application/json",
        }

    def request(self, method: str, path: str, **kwargs) -> Any:
        url = f"{self.base}{path}"
        with httpx.Client(timeout=60.0, headers=self.headers) as client:
            r = client.request(method, url, **kwargs)
            if r.status_code >= 400:
                raise RuntimeError(f"{method} {path} → {r.status_code}: {r.text[:500]}")
            if not r.content:
                return None
            ct = r.headers.get("content-type", "")
            if "json" in ct:
                return r.json()
            return r.text


def ensure_batches(sid: str) -> Path:
    d = fixture_dir(sid)
    if not (d / "pass.ndjson").exists():
        raise FileNotFoundError("run run_local.py first to materialize batches")
    return d


def run_scenario(sc: dict[str, Any], client: CloudClient) -> dict[str, Any]:
    sid = sc["id"]
    d = ensure_batches(sid)
    cpath = Path(__file__).resolve().parents[1] / "contracts" / Path(sc["contract"]["path"]).name
    yaml_content = cpath.read_text()
    name = sc["contract"]["name"]

    # Create contract identity
    created = client.request(
        "POST",
        "/contracts",
        json={"name": name, "yaml_content": yaml_content},
    )
    # API shapes vary slightly — be defensive
    contract_id = (
        created.get("id")
        or created.get("contract_id")
        or (created.get("contract") or {}).get("id")
    )
    if not contract_id:
        # list and find by name
        listing = client.request("GET", "/contracts")
        items = listing if isinstance(listing, list) else listing.get("contracts") or listing.get("items") or []
        match = next((c for c in items if c.get("name") == name), None)
        if not match:
            raise RuntimeError(f"create ok but no id: {created}")
        contract_id = match["id"]

    # Ensure a version exists / promote
    # Try deploy endpoint first (atomic to stable)
    try:
        client.request(
            "POST",
            "/contracts/deploy",
            json={
                "name": name,
                "yaml_content": yaml_content,
                "source": "dogfood",
            },
        )
    except Exception as deploy_err:
        # Fallback: create version + promote if endpoints exist
        try:
            ver = client.request(
                "POST",
                f"/contracts/{contract_id}/versions",
                json={"yaml_content": yaml_content, "version": "1.0.0"},
            )
            version = ver.get("version") or "1.0.0"
            try:
                client.request(
                    "POST",
                    f"/contracts/{contract_id}/versions/{version}/promote",
                    json={},
                )
            except Exception:
                pass
        except Exception as ver_err:
            print(f"  warn: deploy/version: {deploy_err} / {ver_err}")

    def ingest(ndjson_path: Path, *, expect_all_pass: bool = True, dry_run: bool = False) -> Any:
        body = ndjson_path.read_text()
        q = "?dry_run=true" if dry_run else ""
        url = f"{client.base}/v1/ingest/{contract_id}{q}"
        with httpx.Client(timeout=120.0) as http:
            r = http.post(
                url,
                content=body.encode(),
                headers={
                    "X-Api-Key": client.headers["X-Api-Key"],
                    "Content-Type": "application/x-ndjson",
                    "Accept": "application/json",
                },
            )
            # Gateway may return 200 (all pass), 207 (mixed), or 422 (all fail).
            if r.status_code >= 500:
                raise RuntimeError(f"ingest {ndjson_path.name} → {r.status_code}: {r.text[:500]}")
            try:
                data = r.json()
            except Exception as e:
                raise RuntimeError(f"ingest {ndjson_path.name} non-JSON {r.status_code}: {r.text[:300]}") from e
            if expect_all_pass and r.status_code >= 400:
                raise RuntimeError(
                    f"ingest {ndjson_path.name} → {r.status_code}: expected all pass, "
                    f"got passed={data.get('passed')} failed={data.get('failed')}"
                )
            return data

    # Ensure a stable version exists before ingest
    try:
        client.request(
            "POST",
            f"/contracts/{contract_id}/versions/1.0.0/promote",
            json={},
        )
    except Exception:
        # already stable / different version label — list and promote first draft
        try:
            versions = client.request("GET", f"/contracts/{contract_id}/versions")
            items = versions if isinstance(versions, list) else versions.get("versions") or []
            draft = next((v for v in items if v.get("state") == "draft"), None)
            if draft and draft.get("version"):
                client.request(
                    "POST",
                    f"/contracts/{contract_id}/versions/{draft['version']}/promote",
                    json={},
                )
        except Exception as prom_err:
            print(f"  warn: promote: {prom_err}")

    pass_res = ingest(d / "pass.ndjson", expect_all_pass=True)
    fail_res = ingest(d / "fail.ndjson", expect_all_pass=False)
    mixed_res = ingest(d / "mixed.ndjson", expect_all_pass=False)

    report = None
    try:
        report = client.request("GET", f"/contracts/{contract_id}/report")
    except Exception as e:
        report = {"error": str(e)}

    usage = None
    try:
        usage = client.request("GET", "/usage")
    except Exception as e:
        usage = {"error": str(e)}

    summary = {
        "scenario": sid,
        "contract_id": contract_id,
        "contract_name": name,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "ingest": {
            "pass": _slim_ingest(pass_res),
            "fail": _slim_ingest(fail_res),
            "mixed": _slim_ingest(mixed_res),
        },
        "report": report,
        "usage": usage,
    }
    write_json(d / "cloud_result.json", summary)
    print(
        f"[CLOUD] {sid} id={contract_id} "
        f"pass={summary['ingest']['pass']} fail={summary['ingest']['fail']} "
        f"mixed={summary['ingest']['mixed']}"
    )
    return summary


def _slim_ingest(res: Any) -> dict[str, Any]:
    if not isinstance(res, dict):
        return {"raw": str(res)[:200]}
    return {
        "total": res.get("total"),
        "passed": res.get("passed"),
        "failed": res.get("failed"),
        "resolved_version": res.get("resolved_version"),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", required=True, help="scenario id (not all — intentional)")
    args = ap.parse_args()

    key, url = env_api()
    if not key:
        print("CG_API_KEY is required for cloud runs", file=sys.stderr)
        print("Export a key from app.datacontractgate.com → API keys", file=sys.stderr)
        return 2

    sc = load_scenario(args.scenario)
    client = CloudClient(url, key)
    try:
        summary = run_scenario(sc, client)
    except Exception as e:
        print(f"ERROR: {e}", file=sys.stderr)
        return 1

    run_dir = FINDINGS / "runs" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir.mkdir(parents=True, exist_ok=True)
    write_json(run_dir / f"cloud_{args.scenario}.json", summary)
    print(f"wrote {run_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
