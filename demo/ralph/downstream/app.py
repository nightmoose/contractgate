#!/usr/bin/env python3
"""Driftless MCP + Kafi Streams — consumes CLEAN topics only (RFC-092).

Adapted from xdgrulez/driftless (Apache-2.0). Producers write to *.raw;
ContractGate bridge validates and forwards to orders/customers/products.
This process must never subscribe to *.raw.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Optional, TypedDict

# Local packages
_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

from db.db import connect, create_table, get_sink_fun, search  # noqa: E402
from kafka.kafka import streams  # noqa: E402

# Prefer fastmcp / mcp — driftless upstream import path
try:
    from mcp.server.mcpserver import MCPServer
except ImportError:  # pragma: no cover
    try:
        from fastmcp import FastMCP as MCPServer  # type: ignore
    except ImportError as e:
        raise SystemExit(
            "Install downstream deps: pip install -r demo/ralph/requirements-downstream.txt"
        ) from e

os.environ.setdefault("KAFKA_BOOTSTRAP", "127.0.0.1:9092")

_overwrite = os.environ.get("CG_LANCEDB_OVERWRITE", "0") in ("1", "true", "TRUE", "yes")
dbConnection = connect()
table = create_table(dbConnection, overwrite=_overwrite)
sink_fun = get_sink_fun(table)
print(
    f"[downstream] LanceDB table ready (overwrite={_overwrite}); starting Kafi…",
    flush=True,
)
stop_fun = streams(sink_fun, emulated=False)

mcpServer = MCPServer("Driftless Agentic Memory in One Pod (gated by ContractGate)")


class CustomerContextResult(TypedDict):
    summary: str
    score: float


@mcpServer.tool()
def search_customer_context(
    query: Optional[str] = None,
    id: Optional[str] = None,
    customer_id: Optional[str] = None,
    customer_name: Optional[str] = None,
    limit: int = 3,
) -> list[CustomerContextResult]:
    return search(dbConnection, query, id, customer_id, customer_name, limit)


if __name__ == "__main__":
    port = int(os.environ.get("MCP_PORT", "8000"))
    print(
        f"[downstream] Kafi←clean topics; MCP on :{port}. "
        "Ctrl+C stops MCP (streams thread may need process kill).",
        flush=True,
    )
    mcpServer.run(transport="streamable-http", port=port)
