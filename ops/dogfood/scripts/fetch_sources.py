#!/usr/bin/env python3
"""Fetch real public data for dogfood scenarios."""
from __future__ import annotations

import argparse
import json
import random
import re
import sys
from datetime import date, timedelta
from pathlib import Path
from typing import Any

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import FIXTURES, fixture_dir, list_scenarios, load_scenario, write_json


def fetch_url(url: str, headers: dict[str, str] | None = None) -> Any:
    h = {"User-Agent": "contractgate-dogfood/1.0"}
    if headers:
        h.update(headers)
    with httpx.Client(timeout=60.0, headers=h, follow_redirects=True) as client:
        r = client.get(url)
        r.raise_for_status()
        return r.json()


def flatten_usgs(geojson: dict[str, Any], max_records: int) -> list[dict[str, Any]]:
    out = []
    for f in geojson.get("features", [])[:max_records]:
        p = dict(f.get("properties") or {})
        coords = (f.get("geometry") or {}).get("coordinates") or [None, None, None]
        p["longitude"] = coords[0] if len(coords) > 0 else None
        p["latitude"] = coords[1] if len(coords) > 1 else None
        p["depth_km"] = coords[2] if len(coords) > 2 else None
        p["id"] = f.get("id") or p.get("code")
        # Drop heavy nested / list noise for contract focus
        for k in ("ids", "sources", "types", "products"):
            p.pop(k, None)
        out.append(p)
    return out


def flatten_open_meteo(payload: dict[str, Any], max_records: int) -> list[dict[str, Any]]:
    h = payload["hourly"]
    rows = []
    for i, t in enumerate(h["time"][:max_records]):
        rows.append(
            {
                "time": t,
                "temperature_2m": h["temperature_2m"][i],
                "precipitation": h["precipitation"][i],
                "wind_speed_10m": h["wind_speed_10m"][i],
                "latitude": payload.get("latitude"),
                "longitude": payload.get("longitude"),
                "timezone": payload.get("timezone"),
            }
        )
    return rows


def flatten_github(events: list[dict[str, Any]], max_records: int) -> list[dict[str, Any]]:
    out = []
    for e in events[:max_records]:
        out.append(
            {
                "id": e.get("id"),
                "type": e.get("type"),
                "public": e.get("public"),
                "created_at": e.get("created_at"),
                "actor_login": (e.get("actor") or {}).get("login"),
                "repo_name": (e.get("repo") or {}).get("name"),
                "org_login": (e.get("org") or {}).get("login"),
            }
        )
    return out


def slim_fields(rows: list[dict[str, Any]], fields: list[str] | None) -> list[dict[str, Any]]:
    if not fields:
        return rows
    return [{k: r.get(k) for k in fields} for r in rows]


def synthetic_mri(n: int) -> list[dict[str, Any]]:
    rng = random.Random(7)
    statuses = ["active", "pending", "ended", "terminated"]
    currencies = ["USD", "EUR", "GBP", "CAD"]
    freqs = ["weekly", "monthly", "quarterly", "annually"]
    rows = []
    base = date(2024, 1, 1)
    for i in range(n):
        start = base + timedelta(days=rng.randint(0, 800))
        end = None if rng.random() < 0.6 else (start + timedelta(days=rng.randint(30, 700))).isoformat()
        rent = round(rng.uniform(900, 4500), 2)
        rows.append(
            {
                "tenancy_id": f"T-{10000 + i}",
                "unit_id": f"U-{rng.randint(1, 500)}",
                "property_id": f"P-{rng.randint(1, 80)}",
                "tenant_contact_id": f"C-{rng.randint(1000, 9999)}",
                "start_date": start.isoformat(),
                "end_date": end,
                "rent_amount": rent,
                "currency": rng.choice(currencies),
                "status": rng.choice(statuses),
                "deposit_amount": round(rent * rng.choice([0.5, 1.0, 1.5]), 2),
                "payment_frequency": rng.choice(freqs),
            }
        )
    return rows


# Fields where digit-looking strings should become numbers (Socrata etc.).
# Never coerce id-like keys — USGS `code` and GitHub `id` are strings that
# happen to look numeric.
NUMERIC_STRING_FIELDS = {
    "latitude",
    "longitude",
    "lat",
    "lon",
    "lng",
    "depth_km",
    "mag",
    "temperature_2m",
    "precipitation",
    "wind_speed_10m",
    "rent_amount",
    "deposit_amount",
}


def coerce_value(key: str, v: Any) -> Any:
    """Best-effort JSON typing for messy open-data producers (e.g. Socrata)."""
    if isinstance(v, str):
        s = v.strip()
        if s == "":
            return None
        if key in NUMERIC_STRING_FIELDS:
            try:
                if re.fullmatch(r"-?\d+", s):
                    return int(s)
                if re.fullmatch(r"-?\d+\.\d+", s):
                    return float(s)
            except Exception:
                pass
        if s.lower() in ("true", "false") and key in {"public", "tsunami"}:
            return s.lower() == "true"
    # Some APIs return id as JSON number — stringify known id-like keys
    if key in {"id", "unique_key", "code", "tenancy_id", "unit_id", "property_id", "tenant_contact_id"}:
        if isinstance(v, (int, float)) and not isinstance(v, bool):
            # Keep integers that are clearly mag-like out of this set
            return str(int(v)) if float(v) == int(v) else str(v)
    return v


def normalize_row(row: dict[str, Any], *, drop_nulls: bool = True) -> dict[str, Any]:
    """
    Producer-side hygiene for ContractGate.

    Product note: gateway treats JSON null as a present value and type-checks it.
    Optional fields with null fail type_mismatch. Real producers should OMIT
    optional keys rather than send null — we normalize fixtures that way.
    """
    out: dict[str, Any] = {}
    for k, v in row.items():
        if k.startswith("_"):
            continue
        v = coerce_value(k, v)
        if drop_nulls and v is None:
            continue
        out[k] = v
    return out


def fetch_scenario(sc: dict[str, Any]) -> list[dict[str, Any]]:
    sid = sc["id"]
    src = sc["source"]
    kind = src["kind"]
    max_n = int(sc.get("sample", {}).get("max_records") or 50)
    transform = sc.get("sample", {}).get("transform")
    fields = sc.get("sample", {}).get("fields")
    d = fixture_dir(sid)

    if kind == "synthetic" or transform == "mri_synthetic":
        rows = synthetic_mri(max_n)
        rows = [normalize_row(r) for r in rows]
        write_json(d / "events.json", rows)
        print(f"  {sid}: {len(rows)} synthetic events → {d / 'events.json'}")
        return rows

    raw = fetch_url(src["url"], src.get("headers"))
    write_json(d / "raw.json", raw)

    if kind == "http_geojson_features":
        rows = flatten_usgs(raw, max_n)
    elif transform == "open_meteo_hourly":
        rows = flatten_open_meteo(raw, max_n)
    elif transform == "github_flat":
        rows = flatten_github(raw, max_n)
    elif kind in ("http_json", "socrata"):
        if isinstance(raw, list):
            rows = raw[:max_n]
        else:
            raise ValueError(f"{sid}: expected JSON array, got {type(raw)}")
        rows = slim_fields(rows, fields)
    else:
        raise ValueError(f"unsupported source kind: {kind}")

    rows = [normalize_row(r) for r in rows]
    write_json(d / "events.json", rows)
    print(f"  {sid}: {len(rows)} events → {d / 'events.json'}")
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", default="all", help="scenario id or 'all'")
    ap.add_argument("--include-draft", action="store_true")
    args = ap.parse_args()

    if args.scenario == "all":
        status = None if args.include_draft else "ready"
        scenarios = list_scenarios(status=status)
    else:
        scenarios = [load_scenario(args.scenario)]

    FIXTURES.mkdir(parents=True, exist_ok=True)
    errors = 0
    for sc in scenarios:
        print(f"fetch {sc['id']}…")
        try:
            fetch_scenario(sc)
        except Exception as e:
            errors += 1
            print(f"  ERROR {sc['id']}: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
