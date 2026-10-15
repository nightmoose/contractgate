"""CLI: python -m dcg_openmetadata {publish,sync} ..."""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys

from .dcg import DcgClient
from .om import OmClient
from .publisher import load_config, publish
from .sync import om_to_dcg


def _env(name: str, required: bool = True) -> str:
    v = os.environ.get(name, "")
    if required and not v:
        sys.exit(f"missing env var {name}")
    return v


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="dcg_openmetadata")
    sub = p.add_subparsers(dest="cmd", required=True)
    pub = sub.add_parser("publish", help="push DCG stats to OM test case results")
    pub.add_argument("--config", default=os.environ.get("DCG_OM_CONFIG", "mapping.json"))
    pub.add_argument("--dry-run", action="store_true", help="read DCG, print results, skip OM")
    sy = sub.add_parser("sync", help="import an OM data contract into DCG via ODCS")
    sy.add_argument("--om-contract-id", required=True)
    sy.add_argument("--target-contract-id", help="existing DCG contract to add a draft version to")
    sy.add_argument("--name-override")
    a = p.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")

    dcg = DcgClient(_env("DCG_API_URL"), _env("DCG_API_KEY", required=False) or None,
                    org_id=_env("DCG_ORG_ID", required=False) or None)
    if a.cmd == "publish":
        om = OmClient(_env("OM_HOST", required=a.dry_run is False) or "http://localhost:8585",
                      _env("OM_JWT", required=not a.dry_run))
        rows = publish(load_config(a.config), dcg, om, dry_run=a.dry_run)
        print(json.dumps(rows, indent=2))
        return 1 if any(r["status"] == "error" for r in rows) else 0
    om = OmClient(_env("OM_HOST"), _env("OM_JWT"))
    print(json.dumps(om_to_dcg(om, dcg, a.om_contract_id, a.target_contract_id,
                               a.name_override), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
