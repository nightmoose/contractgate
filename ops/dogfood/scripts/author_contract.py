#!/usr/bin/env python3
"""
Author reviewed contracts for scenarios.

Strategy:
  1. If seed_from points at a hand-authored YAML (e.g. MRI), copy/adapt it.
  2. Else profile fixtures and emit a tight draft, then write reviewed YAML.

These contracts are intentionally human-reviewed defaults — re-run after
fetching fresh data and tweak if inference would have drifted.
"""
from __future__ import annotations

import argparse
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import CONTRACTS, fixture_dir, list_scenarios, load_scenario, read_json

# Max distinct values before we refuse to emit an enum (high cardinality).
ENUM_MAX = 12
ENUM_MIN_SAMPLES = 8


def is_iso_date(s: str) -> bool:
    return bool(re.fullmatch(r"\d{4}-\d{2}-\d{2}", s))


def is_iso_dt(s: str) -> bool:
    return bool(re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?(Z|[+-]\d{2}:\d{2})?", s))


def profile_field(name: str, values: list[Any]) -> dict[str, Any]:
    non_null = [v for v in values if v is not None and v != ""]
    required = len(non_null) == len(values) and len(values) > 0
    entity: dict[str, Any] = {"name": name, "type": "string", "required": required}

    if not non_null:
        entity["required"] = False
        return entity

    types = {type(v).__name__ for v in non_null}
    if types <= {"bool"}:
        entity["type"] = "boolean"
    elif types <= {"int"}:
        entity["type"] = "integer"
        entity["min"] = min(non_null)
        entity["max"] = max(non_null)
    elif types <= {"int", "float"}:
        entity["type"] = "number"
        nums = [float(v) for v in non_null]
        entity["min"] = min(nums)
        entity["max"] = max(nums)
    elif types <= {"str"}:
        entity["type"] = "string"
        strs = [str(v) for v in non_null]
        if all(is_iso_date(s) for s in strs):
            entity["pattern"] = r"^\d{4}-\d{2}-\d{2}$"
        elif all(is_iso_dt(s) for s in strs):
            entity["pattern"] = r"^\d{4}-\d{2}-\d{2}T"
        else:
            uniq = sorted(set(strs))
            if (
                len(values) >= ENUM_MIN_SAMPLES
                and 2 <= len(uniq) <= ENUM_MAX
                and len(uniq) / max(len(strs), 1) < 0.5
            ):
                entity["enum"] = uniq
    else:
        entity["type"] = "string"  # fallback for mixed — operator should refine

    return entity


def author_from_samples(name: str, description: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    keys: list[str] = []
    seen = set()
    for r in rows:
        for k in r.keys():
            if k.startswith("_"):
                continue
            if k not in seen:
                seen.add(k)
                keys.append(k)

    entities = []
    for k in keys:
        vals = [r.get(k) for r in rows]
        entities.append(profile_field(k, vals))

    return {
        "version": "1.0",
        "name": name,
        "description": description,
        "ontology": {"entities": entities},
        "glossary": [],
        "metrics": [],
    }


def refine_usgs(contract: dict[str, Any]) -> dict[str, Any]:
    """Tighten known USGS fields after profiling — core fields only."""
    # Focused set: optional sparse fields (felt/cdi/mmi/alert/tz) omitted so
    # null-heavy open data does not create brittle contracts.
    core = {
        "id": {"type": "string", "required": True},
        "mag": {"type": "number", "required": True, "min": -1, "max": 10},
        "place": {"type": "string", "required": True},
        "time": {"type": "integer", "required": True},
        "updated": {"type": "integer", "required": False},
        "status": {
            "type": "string",
            "required": True,
            "enum": ["automatic", "reviewed", "deleted"],
        },
        "tsunami": {"type": "integer", "required": False, "min": 0, "max": 1},
        "sig": {"type": "integer", "required": False},
        "net": {"type": "string", "required": False},
        "code": {"type": "string", "required": False},
        "magType": {"type": "string", "required": False},
        "type": {"type": "string", "required": False},
        "title": {"type": "string", "required": False},
        "url": {"type": "string", "required": False},
        "latitude": {"type": "number", "required": False, "min": -90, "max": 90},
        "longitude": {"type": "number", "required": False, "min": -180, "max": 180},
        "depth_km": {"type": "number", "required": False},
    }
    entities = []
    for name, spec in core.items():
        e = {"name": name, **spec}
        entities.append(e)
    contract["ontology"]["entities"] = entities
    contract["description"] = (
        "USGS all_day earthquake properties (flattened core fields). "
        "Dogfood scenario usgs_earthquake."
    )
    return contract


def refine_nyc(contract: dict[str, Any]) -> dict[str, Any]:
    entities = [
        {"name": "unique_key", "type": "string", "required": True},
        {"name": "created_date", "type": "string", "required": True},
        {"name": "closed_date", "type": "string", "required": False},
        {"name": "agency", "type": "string", "required": True},
        {"name": "agency_name", "type": "string", "required": False},
        {"name": "complaint_type", "type": "string", "required": True},
        {"name": "descriptor", "type": "string", "required": False},
        {"name": "status", "type": "string", "required": True},
        {
            "name": "borough",
            "type": "string",
            "required": False,
            "enum": [
                "BRONX",
                "BROOKLYN",
                "MANHATTAN",
                "QUEENS",
                "STATEN ISLAND",
                "Unspecified",
            ],
        },
        {"name": "latitude", "type": "number", "required": False, "min": 40.0, "max": 41.5},
        {"name": "longitude", "type": "number", "required": False, "min": -75.0, "max": -73.0},
        {"name": "incident_zip", "type": "string", "required": False},
    ]
    contract["ontology"]["entities"] = entities
    contract["description"] = "NYC 311 service request slim fields. Dogfood scenario nyc_311."
    return contract


def refine_weather(contract: dict[str, Any]) -> dict[str, Any]:
    for e in contract["ontology"]["entities"]:
        if e["name"] == "time":
            e["required"] = True
            e["type"] = "string"
        if e["name"] == "temperature_2m":
            e["type"] = "number"
            e["required"] = True
            e["min"] = -80
            e["max"] = 60
        if e["name"] == "precipitation":
            e["type"] = "number"
            e["required"] = True
            e["min"] = 0
            e["max"] = 500
        if e["name"] == "wind_speed_10m":
            e["type"] = "number"
            e["required"] = True
            e["min"] = 0
            e["max"] = 200
    contract["description"] = "Open-Meteo hourly forecast rows (NYC). Dogfood scenario open_meteo."
    return contract


def refine_github(contract: dict[str, Any]) -> dict[str, Any]:
    # org_login often null — omit from required; after normalize_row it is absent.
    contract["ontology"]["entities"] = [
        {"name": "id", "type": "string", "required": True},
        {"name": "type", "type": "string", "required": True},
        {"name": "public", "type": "boolean", "required": True},
        {"name": "created_at", "type": "string", "required": True},
        {"name": "actor_login", "type": "string", "required": True},
        {"name": "repo_name", "type": "string", "required": True},
        {"name": "org_login", "type": "string", "required": False},
    ]
    contract["description"] = "GitHub public events flattened. Dogfood scenario github_events."
    return contract


def load_mri_seed(seed_rel: str, dest_name: str) -> dict[str, Any]:
    seed_path = (CONTRACTS.parent / seed_rel).resolve() if not Path(seed_rel).is_absolute() else Path(seed_rel)
    # scenarios point at ../../../mri_tenancy_event.yaml relative to scenario file concept
    candidates = [
        Path(__file__).resolve().parents[3] / "mri_tenancy_event.yaml",
        CONTRACTS.parent.parent.parent / "mri_tenancy_event.yaml",
        Path(seed_rel),
    ]
    path = next((p for p in candidates if p.exists()), None)
    if path is None:
        raise FileNotFoundError(f"MRI seed not found: {seed_rel}")
    data = yaml.safe_load(path.read_text())
    # Drop envelope for flat synthetic events used in dogfood
    data.pop("envelope", None)
    data["name"] = dest_name
    data["description"] = (
        data.get("description") or "MRI MIX-style tenancy"
    ) + " [dogfood mri_tenancy]"
    return data


def author_scenario(sc: dict[str, Any]) -> Path:
    sid = sc["id"]
    cmeta = sc["contract"]
    out_path = CONTRACTS / Path(cmeta["path"]).name
    CONTRACTS.mkdir(parents=True, exist_ok=True)

    if cmeta.get("seed_from"):
        contract = load_mri_seed(cmeta["seed_from"], cmeta["name"])
        # Synthetic events are flat records — ensure required fields match
        out_path.write_text(yaml.safe_dump(contract, sort_keys=False))
        print(f"  {sid}: seeded → {out_path}")
        return out_path

    events_path = fixture_dir(sid) / "events.json"
    if not events_path.exists():
        raise FileNotFoundError(f"run fetch_sources first: missing {events_path}")
    rows = read_json(events_path)
    contract = author_from_samples(
        cmeta["name"],
        f"Auto-authored draft for {sid} — refine before production.",
        rows,
    )

    refiners = {
        "usgs_earthquake": refine_usgs,
        "nyc_311": refine_nyc,
        "open_meteo": refine_weather,
        "github_events": refine_github,
    }
    if sid in refiners:
        contract = refiners[sid](contract)

    # Annotate field counts for run log curiosity
    n_enum = sum(1 for e in contract["ontology"]["entities"] if e.get("enum"))
    print(f"  {sid}: {len(contract['ontology']['entities'])} fields, {n_enum} enums → {out_path}")
    out_path.write_text(yaml.safe_dump(contract, sort_keys=False))
    return out_path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", default="all")
    args = ap.parse_args()
    scenarios = list_scenarios(status="ready") if args.scenario == "all" else [load_scenario(args.scenario)]
    errors = 0
    for sc in scenarios:
        try:
            author_scenario(sc)
        except Exception as e:
            errors += 1
            print(f"  ERROR {sc['id']}: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
