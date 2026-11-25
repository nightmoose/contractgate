"""``contractgate-py`` command line (Python SDK).

Installed as the ``contractgate-py`` console script and also runnable as
``python -m contractgate``. It is deliberately not called ``contractgate``:
that name belongs to the Rust CLI binary, and both can sit on one PATH.

One subcommand for now:

    contractgate-py synth CONTRACT.yaml --rows 100 [--invalid] [--seed N]
        [--anchor 2026-01-01T00:00:00Z] [--envelope] [--format json|csv] [-o FILE]
    contractgate-py synth --contract-id UUID [--version 1.2.0] ...

``synth`` calls the gateway's ``POST /contracts/synth`` (RFC-117, RFC-118).
Rows come only from the contract's rules; no stored data is read.

The API key comes from ``--api-key`` or ``CONTRACTGATE_API_KEY``; the
gateway URL from ``--base-url`` or ``CONTRACTGATE_BASE_URL``.

Exit codes:
    0  rows written
    1  a contract rule cannot be satisfied (the error names the rule)
    2  bad arguments, unreadable contract, or any other API error
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import sys
from typing import Any, Dict, List, Optional, Sequence, Tuple

import httpx

from contractgate._version import __version__
from contractgate.client import Client
from contractgate.exceptions import ContractGateError, HTTPError

DEFAULT_BASE_URL = "https://api.datacontractgate.com"


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="contractgate-py",
        description="DataContractGate command line (Python SDK).",
    )
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    sub = p.add_subparsers(dest="command", required=True)

    s = sub.add_parser(
        "synth",
        help="Generate synthetic rows from a contract's rules via the API.",
        description="Generate synthetic rows from a contract's rules via the API.",
    )
    s.add_argument("contract", nargs="?", help="Path to a contract YAML file.")
    s.add_argument("--contract-id", help="A stored contract in your org (instead of a file).")
    s.add_argument("--version", dest="contract_version", help="Version of --contract-id.")
    s.add_argument("--rows", type=int, default=100, help="1 to 10000 (default 100).")
    s.add_argument(
        "--invalid",
        action="store_true",
        help="Rows that each break exactly one rule, labeled with it.",
    )
    s.add_argument("--seed", type=int, help="Seed for repeatable output.")
    s.add_argument(
        "--anchor",
        metavar="RFC3339",
        help="Time freshness rules are measured from (default now). "
        "Set it with --seed for output that repeats exactly.",
    )
    s.add_argument(
        "--envelope",
        action="store_true",
        help="Wrap rows in the contract's envelope (JSON only).",
    )
    s.add_argument("--format", choices=["json", "csv"], default="json")
    s.add_argument("-o", "--out", metavar="FILE", help="Write here instead of stdout.")
    s.add_argument("--api-key", help="API key (default: $CONTRACTGATE_API_KEY).")
    s.add_argument(
        "--base-url",
        help=f"Gateway URL (default: $CONTRACTGATE_BASE_URL or {DEFAULT_BASE_URL}).",
    )
    s.add_argument("--timeout", type=float, default=120.0, help="Seconds (default 120).")
    return p


def _flatten(value: Any, prefix: str, out: List[Tuple[str, str]]) -> None:
    """Nested objects become dot-path columns; arrays are JSON text.

    Matches the Rust CLI's CSV layout.
    """
    if isinstance(value, dict) and (value or not prefix):
        for k, v in value.items():
            _flatten(v, f"{prefix}.{k}" if prefix else k, out)
        return
    if value is None:
        cell = ""
    elif isinstance(value, bool):
        cell = "true" if value else "false"
    elif isinstance(value, str):
        cell = value
    elif isinstance(value, (list, dict)):
        cell = json.dumps(value, separators=(",", ":"))
    else:
        cell = json.dumps(value)
    out.append((prefix, cell))


def to_csv(body: Dict[str, Any]) -> str:
    rows: List[List[Tuple[str, str]]] = []
    for r in body.get("rows") or []:
        cells: List[Tuple[str, str]] = []
        if body.get("mode") == "invalid":
            _flatten(r.get("record"), "", cells)
            br = r.get("broken_rule") or {}
            cells.append(("_broken_field", str(br.get("field", ""))))
            cells.append(("_broken_rule", str(br.get("rule", ""))))
        else:
            _flatten(r, "", cells)
        rows.append(cells)
    header: List[str] = []
    for cells in rows:
        for k, _ in cells:
            if k not in header:
                header.append(k)
    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(header)
    for cells in rows:
        d = dict(cells)
        w.writerow([d.get(h, "") for h in header])
    return buf.getvalue()


def _render(body: Dict[str, Any], args: argparse.Namespace) -> str:
    if args.format == "csv":
        return to_csv(body)
    if args.envelope:
        payload = body.get("envelope_payload")
        if args.invalid:
            labels = [r.get("broken_rule") for r in body.get("rows") or []]
            out: Any = {"payload": payload, "labels": labels}
        else:
            out = payload
    else:
        out = body.get("rows")
    return json.dumps(out, indent=2) + "\n"


def _synth(args: argparse.Namespace, transport: Optional[httpx.BaseTransport]) -> int:
    if (args.contract is None) == (args.contract_id is None):
        print("error: pass a contract file or --contract-id (not both)", file=sys.stderr)
        return 2
    if args.envelope and args.format == "csv":
        print("error: --envelope needs --format json", file=sys.stderr)
        return 2
    yaml_content = None
    if args.contract is not None:
        try:
            with open(args.contract, encoding="utf-8") as f:
                yaml_content = f.read()
        except OSError as e:
            print(f"error: cannot read {args.contract}: {e}", file=sys.stderr)
            return 2
    api_key = args.api_key or os.environ.get("CONTRACTGATE_API_KEY")
    base_url = args.base_url or os.environ.get("CONTRACTGATE_BASE_URL") or DEFAULT_BASE_URL
    try:
        with Client(
            base_url=base_url, api_key=api_key, timeout=args.timeout, transport=transport
        ) as cg:
            body = cg.synth(
                yaml_content=yaml_content,
                contract_id=args.contract_id,
                version=args.contract_version,
                rows=args.rows,
                mode="invalid" if args.invalid else "valid",
                seed=args.seed,
                anchor_time=args.anchor,
                envelope=args.envelope,
            )
    except HTTPError as e:
        b = e.body if isinstance(e.body, dict) else {}
        if b.get("error") == "unsatisfiable_rule":
            field = f" on '{b['field']}'" if b.get("field") else ""
            print(f"error: rule '{b.get('rule')}'{field}: {b.get('message')}", file=sys.stderr)
            return 1
        detail = b.get("message") or b.get("error") or str(e)
        print(f"error: HTTP {e.status}: {detail}", file=sys.stderr)
        return 2
    except (ContractGateError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2

    if args.seed is None:
        print(f"seed: {body.get('seed')}", file=sys.stderr)
    for s in body.get("skipped_rules") or []:
        print(
            f"note: no row breaks the {s.get('rule')} rule on '{s.get('field')}': "
            f"{s.get('reason')}",
            file=sys.stderr,
        )
    text = _render(body, args)
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


def main(
    argv: Optional[Sequence[str]] = None,
    *,
    transport: Optional[httpx.BaseTransport] = None,
) -> int:
    """Entry point. ``transport`` lets tests swap in ``httpx.MockTransport``."""
    args = _parser().parse_args(argv)
    if args.command == "synth":
        return _synth(args, transport)
    return 2  # pragma: no cover - argparse enforces a subcommand


def main_exit() -> None:  # pragma: no cover - console script shim
    sys.exit(main())


if __name__ == "__main__":  # pragma: no cover
    main_exit()
