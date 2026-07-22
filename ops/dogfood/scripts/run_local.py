#!/usr/bin/env python3
"""
Local dogfood gate: build pass/fail/mixed batches and validate with the
Python SDK (semantic rules aligned with the gateway for core types).
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (
    FINDINGS,
    build_batches,
    fixture_dir,
    list_scenarios,
    load_scenario,
    read_json,
    write_json,
    write_ndjson,
)

from contractgate import Contract


def validate_batch(compiled, rows: list[dict[str, Any]]) -> dict[str, Any]:
    passed = 0
    failed = 0
    details = []
    for i, row in enumerate(rows):
        # Strip harness metadata
        event = {k: v for k, v in row.items() if not k.startswith("_")}
        vr = compiled.validate(event)
        if vr.passed:
            passed += 1
        else:
            failed += 1
            details.append(
                {
                    "index": i,
                    "mutation": row.get("_mutation"),
                    "violations": [
                        {
                            "field": getattr(v, "field", None) or (v.get("field") if isinstance(v, dict) else None),
                            "kind": getattr(v, "kind", None) or (v.get("kind") if isinstance(v, dict) else None),
                            "message": str(v),
                        }
                        for v in (vr.violations or [])
                    ],
                }
            )
    return {
        "total": len(rows),
        "passed": passed,
        "failed": failed,
        "pass_rate": (passed / len(rows)) if rows else 0.0,
        "failures": details[:20],
    }


def run_scenario(sc: dict[str, Any]) -> dict[str, Any]:
    sid = sc["id"]
    cpath = Path(__file__).resolve().parents[1] / sc["contract"]["path"]
    if not cpath.exists():
        # also try contracts/<name>
        cpath = Path(__file__).resolve().parents[1] / "contracts" / Path(sc["contract"]["path"]).name
    events_path = fixture_dir(sid) / "events.json"
    if not events_path.exists():
        raise FileNotFoundError(f"missing fixtures — run fetch_sources: {events_path}")
    if not cpath.exists():
        raise FileNotFoundError(f"missing contract — run author_contract: {cpath}")

    rows = read_json(events_path)
    yaml_text = cpath.read_text()
    compiled = Contract.from_yaml(yaml_text).compile()

    batches = build_batches(rows, sc.get("mutations") or [])
    d = fixture_dir(sid)
    write_ndjson(d / "pass.ndjson", batches["pass"])
    write_ndjson(d / "fail.ndjson", batches["fail"])
    write_ndjson(d / "mixed.ndjson", batches["mixed"])

    res_pass = validate_batch(compiled, batches["pass"])
    res_fail = validate_batch(compiled, batches["fail"])
    res_mixed = validate_batch(compiled, batches["mixed"])

    # Expectations
    # - pass batch: all clean fixtures must pass
    # - fail batch: every mutated record must fail (strict)
    # - mixed: mostly pass; used for pilot realism, not a hard gate on ratio
    errors: list[str] = []
    if res_pass["failed"] > 0:
        errors.append(
            f"pass batch: expected 0 fails, got {res_pass['failed']} "
            f"(first: {res_pass['failures'][:2]})"
        )
    if res_fail["passed"] > 0:
        errors.append(
            f"fail batch: expected 0 passes, got {res_fail['passed']} "
            f"(mutations may not violate contract — tighten rules or mutations)"
        )
    if res_fail["failed"] == 0:
        errors.append("fail batch: no failures detected — mutations ineffective")

    summary = {
        "scenario": sid,
        "contract": str(cpath),
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "pass": res_pass,
        "fail": res_fail,
        "mixed": res_mixed,
        "ok": not errors,
        "errors": errors,
        "product_questions": sc.get("product_questions"),
    }
    write_json(d / "local_result.json", summary)

    status = "PASS" if summary["ok"] else "FAIL"
    print(
        f"[{status}] {sid}: pass {res_pass['passed']}/{res_pass['total']} · "
        f"fail-caught {res_fail['failed']}/{res_fail['total']} · "
        f"mixed {res_mixed['passed']}/{res_mixed['total']}"
    )
    for e in errors:
        print(f"       ! {e}")
    return summary


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", default="all")
    args = ap.parse_args()
    scenarios = list_scenarios(status="ready") if args.scenario == "all" else [load_scenario(args.scenario)]

    results = []
    for sc in scenarios:
        try:
            results.append(run_scenario(sc))
        except Exception as e:
            print(f"[ERROR] {sc['id']}: {e}", file=sys.stderr)
            results.append({"scenario": sc["id"], "ok": False, "errors": [str(e)]})

    run_dir = FINDINGS / "runs" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir.mkdir(parents=True, exist_ok=True)
    write_json(run_dir / "local_summary.json", results)

    failed = [r for r in results if not r.get("ok")]
    print(f"\n{len(results) - len(failed)}/{len(results)} scenarios ok · log {run_dir}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
