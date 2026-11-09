#!/usr/bin/env python3
"""Live public-catalog smoke test (read-only).

Checks production after migration 048 (public catalog seed refresh):
  1. waits until ACS shows B19013_001M + the acs-5year.html source URL
  2. all 12 slugs: JSON, /yaml, /odcs on the API and /catalog/<slug> on the app -> 200
  3. the review fixes (Shopify expired/unpaid, Generic Sheets citation,
     Stripe ch_/py_, Square ids, Xero /Date()
  4. every published sample row validates against its published contract
     with the repo's real validator (`contractgate test`), and no sample value
     is an `example_<field>` placeholder
  5. live contract YAML / samples match catalog/seeds in --repo (skipped if absent)

Usage (from the repo root, after `cargo build --bin contractgate`):
  python3 scripts/catalog_live_smoke.py [--wait 1800] [--repo /path/to/main/checkout]
Exit 0 = all checks pass, 1 = at least one failure.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

import yaml

SLUGS = [
    "acs-5year-2023-county", "ga4-bigquery-events", "generic-sheets-transactions-csv",
    "hubspot-contacts", "quickbooks-invoices", "salesforce-account",
    "salesforce-opportunity", "shopify-orders", "square-payments",
    "stripe-charges", "stripe-customers", "xero-invoices",
]
ACS_URL = "https://www.census.gov/data/developers/data-sets/acs-5year.html"

results = []  # (ok, name, detail)


def check(ok, name, detail=""):
    results.append((bool(ok), name, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  — {detail}" if detail and not ok else ""))
    return ok


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "contractgate-catalog-smoke/1.0"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:  # network error
        return 0, str(e)


def fields(contract):
    return {f["name"]: f for f in (contract.get("ontology") or {}).get("entities", [])}


def acs_ready(api):
    s, body = get(f"{api}/public-contracts/acs-5year-2023-county")
    if s != 200:
        return False, f"HTTP {s}"
    d = json.loads(body)
    names = fields(yaml.safe_load(d["contract_yaml"]))
    has_m = "B19013_001M" in names
    url_ok = d.get("verified_source_url") == ACS_URL
    return has_m and url_ok, f"B19013_001M field={has_m}, verified_source_url={d.get('verified_source_url')}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--api", default="https://api.datacontractgate.com")
    ap.add_argument("--app", default="https://app.datacontractgate.com")
    root = Path(__file__).resolve().parent.parent
    ap.add_argument("--cg", default=str(root / "target/debug/contractgate"),
                    help="path to the built `contractgate` CLI binary")
    ap.add_argument("--repo", default=str(root),
                    help="checkout to compare live rows against (needs catalog/seeds)")
    ap.add_argument("--wait", type=int, default=0, help="seconds to wait for the ACS refresh (0 = check once)")
    ap.add_argument("--interval", type=int, default=30)
    a = ap.parse_args()

    # 1. wait for the seed refresh
    deadline = time.time() + a.wait
    while True:
        ok, detail = acs_ready(a.api)
        if ok or time.time() >= deadline:
            break
        print(f"waiting for migration 048 on prod… ({detail})")
        time.sleep(a.interval)
    check(ok, "1. ACS shows B19013_001M + acs-5year.html", detail)
    if not ok:
        print("      (continuing so you can see everything that still differs)")

    # 2. endpoints
    s, body = get(f"{a.api}/public-contracts")
    listed = [x["slug"] for x in json.loads(body)] if s == 200 else []
    check(s == 200 and sorted(listed) == sorted(SLUGS), "2. list has exactly the 12 slugs",
          f"HTTP {s}, missing={sorted(set(SLUGS) - set(listed))}, extra={sorted(set(listed) - set(SLUGS))}")
    live = {}
    for slug in SLUGS:
        bad = []
        for path in ["", "/yaml", "/odcs"]:
            st, b = get(f"{a.api}/public-contracts/{slug}{path}")
            if st != 200:
                bad.append(f"api{path or ' json'}={st}")
            elif path == "":
                live[slug] = json.loads(b)
        st, _ = get(f"{a.app}/catalog/{slug}")
        if st != 200:
            bad.append(f"app page={st}")
        check(not bad, f"2. {slug} endpoints 200", ", ".join(bad))

    def c(slug):
        return yaml.safe_load(live[slug]["contract_yaml"]) if slug in live else {}

    # 3. review fixes
    fin = fields(c("shopify-orders")).get("Financial Status", {}).get("enum") or []
    check("expired" in fin and "unpaid" not in fin, "3. Shopify Financial Status has expired, not unpaid", f"enum={fin}")

    g = live.get("generic-sheets-transactions-csv", {})
    gtext = json.dumps(g)
    bad = [p for p in ["app.datacontractgate.com/catalog/", "not affiliated with DataContractGate",
                       "compatible with DataContractGate"] if p in gtext]
    check(g and not bad, "3. Generic Sheets not self-referential", f"contains {bad}")

    pat = fields(c("stripe-charges")).get("id", {}).get("pattern")
    ok = bool(pat) and all(re.search(pat, v) for v in ["ch_3Oexample0001AbCdEfGh", "py_3Oexample0002IjKlMnOp"]) \
        and not re.search(pat, "src_example")
    check(ok, "3. Stripe charge id accepts ch_/py_, rejects src_", f"pattern={pat}")

    sq = [r.get("id", "") for r in live.get("square-payments", {}).get("sample_json") or []]
    check(sq and not [i for i in sq if i.startswith(("pmt_", "ord_"))], "3. Square sample ids have no pmt_/ord_", f"ids={sq}")

    x = c("xero-invoices")
    xd = fields(x).get("Date", {})
    ok = "/Date(" in (x.get("description") or "") and xd.get("type") == "string" and xd.get("pattern") \
        and re.search(xd["pattern"], "/Date(1768003200000+0000)/") and not re.search(xd["pattern"], "2026-01-10")
    check(ok, "3. Xero uses /Date(...)/ strings", f"Date field={xd}")

    # 4. validate samples with the real validator
    with tempfile.TemporaryDirectory() as tmp:
        for slug, d in live.items():
            rows = d.get("sample_json") or []
            placeholders = [f"{k}={v}" for r in rows for k, v in r.items()
                            if isinstance(v, str) and v.lower().startswith("example_")]
            cp, dp = Path(tmp, f"{slug}.yaml"), Path(tmp, f"{slug}.json")
            cp.write_text(d["contract_yaml"])
            dp.write_text(json.dumps(rows))
            p = subprocess.run([a.cg, "test", "--contract", str(cp), "--data", str(dp)],
                               capture_output=True, text=True)
            out = (p.stdout + p.stderr).strip().replace("\n", " | ")[:400]
            check(len(rows) == 3 and p.returncode == 0 and not placeholders,
                  f"4. {slug} {len(rows)} samples validate",
                  f"exit={p.returncode} placeholders={placeholders[:3]} {out}")

    # drift: live rows vs repo seeds
    if not Path(a.repo, "catalog/seeds").is_dir():
        print(f"SKIP  5. seed comparison — no catalog/seeds in {a.repo}")
    else:
        for slug, d in live.items():
            seed = Path(a.repo, "catalog/seeds", slug)
            same_yaml = yaml.safe_load((seed / "contract.yaml").read_text()) == yaml.safe_load(d["contract_yaml"])
            same_samples = json.loads((seed / "samples.json").read_text()) == d.get("sample_json")
            check(same_yaml and same_samples, f"5. {slug} matches catalog/seeds on main",
                  f"contract_same={same_yaml} samples_same={same_samples}")

    failed = [r for r in results if not r[0]]
    print(f"\n{len(results) - len(failed)}/{len(results)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
