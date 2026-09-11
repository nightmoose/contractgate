#!/usr/bin/env python3
"""MCP client for Driftless (RFC-092) — retries until results or timeout."""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
from typing import Any, Dict

from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client


async def run_mcp_tool(
    server_url: str,
    tool_name: str,
    arguments: Dict[str, Any],
) -> list[str]:
    texts: list[str] = []
    async with streamable_http_client(server_url) as (read, write, *_):
        async with ClientSession(read, write) as session:
            await session.initialize()
            callToolResult = await session.call_tool(tool_name, arguments=arguments)
            for content in callToolResult.content:
                if getattr(content, "type", None) == "text":
                    texts.append(content.text)
                else:
                    texts.append(str(content))
    return texts


async def main_async() -> int:
    server_url_str = os.getenv("MCP_SERVER_URL", "http://127.0.0.1:8000/mcp")
    parser = argparse.ArgumentParser(description="Driftless MCP client (gated demo)")
    parser.add_argument("--url", default=server_url_str)
    parser.add_argument("--id", default=None, help="Order ID")
    parser.add_argument("--customer-id", default=None)
    parser.add_argument("--customer-name", default=None)
    parser.add_argument("--query", default=None, help="Vector search query")
    parser.add_argument("--retries", type=int, default=8)
    parser.add_argument("--delay", type=float, default=3.0)
    args = parser.parse_args()

    tool_args = {
        "id": args.id,
        "customer_id": args.customer_id,
        "customer_name": args.customer_name,
        "query": args.query,
    }
    # Drop Nones — some MCP stacks dislike explicit nulls
    tool_args = {k: v for k, v in tool_args.items() if v is not None}
    if not tool_args:
        tool_args = {"query": "order"}

    last: list[str] = []
    for attempt in range(1, args.retries + 1):
        try:
            last = await run_mcp_tool(args.url, "search_customer_context", tool_args)
        except Exception as exc:  # noqa: BLE001
            print(f"[client] attempt {attempt}/{args.retries} error: {exc}", flush=True)
            await asyncio.sleep(args.delay)
            continue
        nonempty = [t for t in last if t and t.strip() and t.strip() not in ("[]", "null")]
        if nonempty:
            for t in nonempty:
                print(t)
            return 0
        print(
            f"[client] attempt {attempt}/{args.retries}: empty result "
            f"(LanceDB still warming?)",
            flush=True,
        )
        await asyncio.sleep(args.delay)

    print(
        "[client] no results after retries. "
        "Confirm app.py logged 'Merge inserting' then retry.",
        file=sys.stderr,
    )
    if last:
        print(f"[client] last raw: {last!r}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main_async()))
