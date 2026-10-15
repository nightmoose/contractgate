"""Round-trip test: DCG export -> OM import -> OM export -> DCG import.

Reports which DCG `x-contractgate-*` extensions survive the trip through OpenMetadata.

    OM_HOST=... OM_JWT=... DCG_API_URL=... DCG_API_KEY=... \
      python examples/roundtrip_check.py --contract-id <dcg uuid> --version 1.0.0 \
        --table-fqn dcg_demo.analytics.public.orders [--import-version 1.0.1]
"""
import argparse
import json
import os
import re
import sys
import urllib.parse

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from dcg_openmetadata import http  # noqa: E402
from dcg_openmetadata.dcg import DcgClient  # noqa: E402
from dcg_openmetadata.om import OmClient  # noqa: E402
from dcg_openmetadata.sync import dcg_odcs_for_om, diff_extensions  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--contract-id", required=True)
    ap.add_argument("--version", default="1.0.0")
    ap.add_argument("--table-fqn", required=True)
    ap.add_argument("--import-version", default="1.0.1",
                    help="version for the re-imported doc (must not exist on the contract)")
    ap.add_argument("--out", default="roundtrip-out")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    dcg = DcgClient(os.environ["DCG_API_URL"], os.environ.get("DCG_API_KEY"),
                    org_id=os.environ.get("DCG_ORG_ID"))
    om = OmClient(os.environ.get("OM_HOST", "http://localhost:8585"), os.environ["OM_JWT"])

    # 1. DCG -> ODCS
    dcg_yaml = dcg.export_odcs(a.contract_id, a.version)
    open(f"{a.out}/1-dcg-export.yaml", "w").write(dcg_yaml)
    om_input = dcg_odcs_for_om(dcg_yaml)  # string description -> {purpose: ...}

    # 2. OM: validate (import report), then create-or-replace the contract on the table
    table = om.get_table(a.table_fqn) or sys.exit(f"table not found: {a.table_fqn}")
    q = urllib.parse.urlencode({"entityId": table["id"], "entityType": "table",
                                "createTestCases": "false"})
    hdr = {"Authorization": f"Bearer {om.jwt}", "Content-Type": "application/yaml"}
    report = http.request("POST", f"{om.api}/v1/dataContracts/odcs/validate/yaml?{q}", hdr,
                          om_input)
    json.dump(report, open(f"{a.out}/2-om-import-report.json", "w"), indent=2)
    contract = http.request("PUT", f"{om.api}/v1/dataContracts/odcs/yaml?{q}&mode=replace",
                            hdr, om_input)
    om_id = contract["id"]

    # 3. OM -> ODCS
    om_yaml = om.get_contract_odcs_yaml(om_id)
    open(f"{a.out}/3-om-export.yaml", "w").write(om_yaml)

    # 4. ODCS -> DCG (new draft version on the same contract via target_contract_id)
    bumped = re.sub(r"^version:.*$", f'version: "{a.import_version}"', om_yaml, count=1,
                    flags=re.M)
    try:
        imported = dcg.import_odcs(bumped, target_contract_id=a.contract_id)
        note = "ok"
    except http.HttpError as e:
        imported, note = {"error": e.body}, f"HTTP {e.status}"
    json.dump(imported, open(f"{a.out}/4-dcg-import-response.json", "w"), indent=2)

    issues = [i for i in report.get("odcsImportReport", {}).get("issues", [])
              if i.get("severity") in ("blocking", "warning")]
    print(json.dumps({
        "om_contract_id": om_id,
        "om_valid": report.get("valid"),
        "om_issues": [f"{i['severity']}: {i['field']} - {i['message']}" for i in issues],
        "extensions": diff_extensions(dcg_yaml, om_yaml),
        "dcg_reimport": note,
    }, indent=2))


if __name__ == "__main__":
    main()
