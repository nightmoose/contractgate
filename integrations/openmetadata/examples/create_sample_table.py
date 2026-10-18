"""Create a demo service/database/schema/table in OpenMetadata (idempotent).

    OM_HOST=http://localhost:8585 OM_JWT=... python examples/create_sample_table.py
Prints the table FQN: dcg_demo.analytics.public.orders
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from dcg_openmetadata.http import HttpError  # noqa: E402
from dcg_openmetadata.om import OmClient  # noqa: E402

SERVICE, DB, SCHEMA, TABLE = "dcg_demo", "analytics", "public", "orders"
COLUMNS = [
    {"name": "order_id", "dataType": "VARCHAR", "dataLength": 64},
    {"name": "customer_id", "dataType": "VARCHAR", "dataLength": 64},
    {"name": "amount", "dataType": "DOUBLE"},
    {"name": "currency", "dataType": "VARCHAR", "dataLength": 3},
    {"name": "created_at", "dataType": "BIGINT"},
]


def put(om, path, body):
    # PUT = create-or-update in OpenMetadata, so re-running is safe.
    try:
        return om._req("PUT", path, body)
    except HttpError as e:
        sys.exit(str(e))


def main():
    om = OmClient(os.environ.get("OM_HOST", "http://localhost:8585"), os.environ["OM_JWT"])
    put(om, "/v1/services/databaseServices", {
        "name": SERVICE, "serviceType": "CustomDatabase",
        "description": "Demo service for the DataContractGate integration",
        "connection": {"config": {"type": "CustomDatabase", "sourcePythonClass": "none"}},
    })
    put(om, "/v1/databases", {"name": DB, "service": SERVICE})
    put(om, "/v1/databaseSchemas", {"name": SCHEMA, "database": f"{SERVICE}.{DB}"})
    t = put(om, "/v1/tables", {
        "name": TABLE, "databaseSchema": f"{SERVICE}.{DB}.{SCHEMA}", "columns": COLUMNS,
        "description": "Orders events, enforced at ingest by DataContractGate",
    })
    print(t["fullyQualifiedName"])


if __name__ == "__main__":
    main()
