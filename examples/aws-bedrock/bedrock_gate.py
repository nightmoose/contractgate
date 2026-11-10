#!/usr/bin/env python3
"""Check Amazon Bedrock model output against a ContractGate contract.

Each sample note goes through three steps:

1. An extractor model on Bedrock (Amazon Nova Lite by default) turns the note
   into JSON.
2. ContractGate checks that JSON against the contract (types, required
   fields, enums, ranges, lengths).
3. If the check fails, a repairer model (Amazon Nova Pro by default) gets the
   violations and the contract and rewrites the JSON. The check runs again.

The result for each note is PASS, REPAIR, or QUARANTINE. Nothing that fails
the contract is forwarded.

By default the check runs in process with the `contractgate` Python package,
so no ContractGate account is needed. With `--hosted`, the check goes to the
hosted gateway (`POST /egress/{contract}` with dry_run) using
CONTRACTGATE_API_KEY.

Usage:
    python bedrock_gate.py samples/
    python bedrock_gate.py samples/ --json report.json
    python bedrock_gate.py samples/ --hosted   # needs CONTRACTGATE_API_KEY
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from pathlib import Path

import boto3
from contractgate import Contract
from contractgate.validator import validate

HERE = Path(__file__).resolve().parent
EXTRACTOR = os.environ.get("BEDROCK_EXTRACTOR_MODEL", "amazon.nova-lite-v1:0")
REPAIRER = os.environ.get("BEDROCK_REPAIRER_MODEL", "amazon.nova-pro-v1:0")
# USD per 1M tokens (input, output), us-east-1 on-demand list price.
PRICES = {
    "amazon.nova-micro-v1:0": (0.035, 0.14),
    "amazon.nova-lite-v1:0": (0.06, 0.24),
    "amazon.nova-pro-v1:0": (0.80, 3.20),
}

C = {"g": "\033[32m", "y": "\033[33m", "r": "\033[31m", "b": "\033[1m", "d": "\033[2m", "x": "\033[0m"}
if not sys.stdout.isatty() and not os.environ.get("FORCE_COLOR"):
    C = {k: "" for k in C}


def load_contracts() -> dict:
    out = {}
    for p in sorted((HERE / "contracts").glob("*.yaml")):
        c = Contract.from_yaml(p.read_text())
        out[c.name] = c.compile()
    return out


def contract_brief(compiled) -> str:
    lines = [f"Contract {compiled.contract.name}:"]
    for e in compiled.contract.entities:
        parts = [e.field_type.value]
        if e.allowed_values:
            parts.append("one of " + ", ".join(map(str, e.allowed_values)))
        if e.min is not None or e.max is not None:
            parts.append(f"range {e.min}..{e.max}")
        if e.min_length is not None or e.max_length is not None:
            parts.append(f"length {e.min_length}..{e.max_length}")
        lines.append(f"- {e.name} ({'required' if e.required else 'optional'}; {'; '.join(parts)})")
    return "\n".join(lines)


class Bedrock:
    def __init__(self, region: str):
        self.rt = boto3.client("bedrock-runtime", region_name=region)
        self.cost = 0.0
        self.tokens = [0, 0]

    def json_call(self, model: str, system: str, user: str) -> tuple[dict | None, str, float]:
        t0 = time.perf_counter()
        r = self.rt.converse(
            modelId=model,
            system=[{"text": system}],
            messages=[{"role": "user", "content": [{"text": user}]}],
            inferenceConfig={"maxTokens": 400, "temperature": 0},
        )
        ms = (time.perf_counter() - t0) * 1000
        u = r.get("usage", {})
        i, o = u.get("inputTokens", 0), u.get("outputTokens", 0)
        self.tokens[0] += i
        self.tokens[1] += o
        pi, po = PRICES.get(model, (0, 0))
        self.cost += (i * pi + o * po) / 1e6
        text = r["output"]["message"]["content"][0]["text"]
        m = re.search(r"\{.*\}", text, re.S)
        try:
            return (json.loads(m.group(0)) if m else None), text, ms
        except json.JSONDecodeError:
            return None, text, ms


class LocalCheck:
    name = "in-process contractgate validator"

    def __init__(self, compiled):
        self.compiled = compiled

    def __call__(self, contract: str, obj):
        r = validate(self.compiled[contract], obj if isinstance(obj, dict) else {})
        return r.passed, [(v.field, getattr(v.kind, "value", str(v.kind)), v.message) for v in r.violations], r.validation_us


class HostedCheck:
    name = "hosted ContractGate /egress (dry_run)"

    def __init__(self):
        from contractgate import Client
        self.client = Client(
            base_url=os.environ.get("CONTRACTGATE_URL", "https://api.datacontractgate.com"),
            api_key=os.environ["CONTRACTGATE_API_KEY"],
        )
        self.ids = {c.name: c.id for c in self.client.list_contracts()}

    def __call__(self, contract: str, obj):
        t0 = time.perf_counter()
        r = self.client.egress(contract_id=self.ids[contract], events=[obj], dry_run=True)
        us = int((time.perf_counter() - t0) * 1e6)
        o = r.outcomes[0]
        return o.passed, [(v.field, getattr(v.kind, "value", str(v.kind)), v.message) for v in o.violations], us


EXTRACT_SYSTEM = (
    "You turn a short business note into one JSON object. Reply with JSON only. "
    "Copy the values the way the note writes them. Use null when the note does not say."
)
REPAIR_SYSTEM = (
    "You fix a JSON object so it satisfies a data contract. Reply with the corrected JSON object only. "
    "Normalize values to the allowed forms (for example a currency symbol to its ISO code, a phrase to the closest allowed enum value, "
    "a formatted number to a plain number). Never invent a value the note does not support: if a required value is not in the note, leave it null."
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("samples", nargs="?", default=str(HERE / "samples"))
    ap.add_argument("--region", default=os.environ.get("AWS_REGION", os.environ.get("AWS_DEFAULT_REGION", "us-east-1")))
    ap.add_argument("--hosted", action="store_true")
    ap.add_argument("--json", dest="json_out")
    ap.add_argument("--slow", type=float, default=0.0, help="pause between steps (for screen recording)")
    a = ap.parse_args()

    compiled = load_contracts()
    check = HostedCheck() if a.hosted else LocalCheck(compiled)
    br = Bedrock(a.region)
    print(f"{C['b']}ContractGate x Amazon Bedrock{C['x']}  region={a.region}")
    print(f"extractor={EXTRACTOR}  repairer={REPAIRER}")
    print(f"check={check.name}\n")

    report = []
    for path in sorted(Path(a.samples).glob("*.txt")):
        note = path.read_text().strip()
        contract = next(n for n in compiled if n.split("_")[0] in path.stem)
        fields = ", ".join(e.name for e in compiled[contract].contract.entities)
        print(f"{C['b']}== {path.stem}{C['x']}  contract={contract}")
        print(f"{C['d']}note: {note}{C['x']}")
        time.sleep(a.slow)
        obj, raw, ms = br.json_call(EXTRACTOR, EXTRACT_SYSTEM, f"Keys: {fields}\nNote: {note}")
        print(f"  extract  {ms:6.0f} ms  {json.dumps(obj, ensure_ascii=False)}")
        ok, viol, us = check(contract, obj)
        row = {"sample": path.stem, "contract": contract, "extracted": obj, "violations_first": viol, "check_us_first": us}
        if ok:
            print(f"  check    {us:6d} µs  {C['g']}0 violations{C['x']}")
            print(f"  {C['g']}{C['b']}PASS{C['x']} -> forwarded\n")
            row["outcome"] = "pass"
            row["final"] = obj
        else:
            print(f"  check    {us:6d} µs  {C['y']}{len(viol)} violation(s){C['x']}")
            for f, k, m in viol:
                print(f"           {C['y']}- {f}: {k} ({m}){C['x']}")
            time.sleep(a.slow)
            prompt = (
                f"{contract_brief(compiled[contract])}\n\nOriginal note: {note}\n\n"
                f"JSON that failed: {json.dumps(obj, ensure_ascii=False)}\n\nViolations:\n"
                + "\n".join(f"- {f}: {m}" for f, _, m in viol)
            )
            fixed, raw2, ms2 = br.json_call(REPAIRER, REPAIR_SYSTEM, prompt)
            print(f"  repair   {ms2:6.0f} ms  {json.dumps(fixed, ensure_ascii=False)}")
            ok2, viol2, us2 = check(contract, fixed)
            row.update({"repaired": fixed, "violations_after_repair": viol2, "check_us_after_repair": us2})
            if ok2:
                print(f"  re-check {us2:6d} µs  {C['g']}0 violations{C['x']}")
                print(f"  {C['g']}{C['b']}REPAIR{C['x']} -> forwarded the repaired record\n")
                row["outcome"] = "repair"
                row["final"] = fixed
            else:
                print(f"  re-check {us2:6d} µs  {C['r']}{len(viol2)} violation(s){C['x']}")
                for f, k, m in viol2:
                    print(f"           {C['r']}- {f}: {k} ({m}){C['x']}")
                print(f"  {C['r']}{C['b']}QUARANTINE{C['x']} -> held back with the violation report, not forwarded\n")
                row["outcome"] = "quarantine"
        report.append(row)
        time.sleep(a.slow)

    counts = {k: sum(r["outcome"] == k for r in report) for k in ("pass", "repair", "quarantine")}
    print(f"{C['b']}Summary{C['x']}: {counts['pass']} pass, {counts['repair']} repair, {counts['quarantine']} quarantine")
    print(f"Bedrock tokens in/out: {br.tokens[0]}/{br.tokens[1]}  est. cost ${br.cost:.5f}")
    if a.json_out:
        Path(a.json_out).write_text(json.dumps({"extractor": EXTRACTOR, "repairer": REPAIRER, "check": check.name,
                                                "results": report, "tokens": br.tokens, "est_cost_usd": br.cost}, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
