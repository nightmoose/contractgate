"""Remove the demo artifacts from OpenMetadata (hard delete).

    OM_HOST=http://localhost:8585 OM_JWT=... python examples/cleanup_om.py [--keep-definition]

Deletes, in order: the DCG test case(s) on the demo table, the `dcgIngestContract`
test definition, and the `dcg_demo` database service (recursively: database, schema,
table, and the table's data contract). Missing entities are skipped.
"""
import argparse
import os
import sys
import urllib.parse

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from dcg_openmetadata.http import HttpError, quote  # noqa: E402
from dcg_openmetadata.om import TEST_DEFINITION, OmClient  # noqa: E402

TABLE = "dcg_demo.analytics.public.orders"
SERVICE = "dcg_demo"


def delete(om, label, get_path, delete_path, params):
    try:
        ent = om._req("GET", get_path)
    except HttpError as e:
        if e.status == 404:
            print(f"skip    {label} (not found)")
            return
        raise
    om._req("DELETE", f"{delete_path}/{ent['id']}?{urllib.parse.urlencode(params)}")
    print(f"deleted {label}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--table-fqn", default=TABLE)
    ap.add_argument("--service", default=SERVICE)
    ap.add_argument("--keep-definition", action="store_true",
                    help="keep the dcgIngestContract test definition (other tables use it)")
    a = ap.parse_args()
    om = OmClient(os.environ.get("OM_HOST", "http://localhost:8585"), os.environ["OM_JWT"])
    hard = {"hardDelete": "true", "recursive": "true"}

    link = f"<#E::table::{a.table_fqn}>"
    cases = om._req("GET", "/v1/dataQuality/testCases?" + urllib.parse.urlencode(
        {"entityLink": link, "limit": 1000})).get("data", [])
    for c in cases:
        if c.get("testDefinition", {}).get("name") in (TEST_DEFINITION, None):
            om._req("DELETE", f"/v1/dataQuality/testCases/{c['id']}?hardDelete=true")
            print(f"deleted test case {c['fullyQualifiedName']}")
    if not a.keep_definition:
        delete(om, f"test definition {TEST_DEFINITION}",
               f"/v1/dataQuality/testDefinitions/name/{quote(TEST_DEFINITION)}",
               "/v1/dataQuality/testDefinitions", {"hardDelete": "true"})
    delete(om, f"database service {a.service} (+ database, schema, table, contract)",
           f"/v1/services/databaseServices/name/{quote(a.service)}",
           "/v1/services/databaseServices", hard)


if __name__ == "__main__":
    main()
