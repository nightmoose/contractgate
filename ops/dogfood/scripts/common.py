"""Shared helpers for the dogfood harness."""
from __future__ import annotations

import json
import os
import random
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = ROOT / "scenarios"
CONTRACTS = ROOT / "contracts"
FIXTURES = ROOT / "fixtures"
FINDINGS = ROOT / "findings"


def load_scenario(scenario_id: str) -> dict[str, Any]:
    path = SCENARIOS / f"{scenario_id}.yaml"
    if not path.exists():
        raise FileNotFoundError(f"scenario not found: {path}")
    data = yaml.safe_load(path.read_text())
    data["_path"] = str(path)
    return data


def list_scenarios(status: str | None = "ready") -> list[dict[str, Any]]:
    out = []
    for path in sorted(SCENARIOS.glob("*.yaml")):
        if path.name.startswith("_"):
            continue
        data = yaml.safe_load(path.read_text())
        if status and data.get("status") != status:
            continue
        data["_path"] = str(path)
        out.append(data)
    return out


def fixture_dir(scenario_id: str) -> Path:
    d = FIXTURES / scenario_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def write_json(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, default=str) + "\n")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text())


def write_ndjson(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w") as f:
        for row in rows:
            f.write(json.dumps(row, default=str) + "\n")


def read_ndjson(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            rows.append(json.loads(line))
    return rows


def mutate_record(record: dict[str, Any], mutation: dict[str, Any]) -> dict[str, Any]:
    """Apply a single controlled mutation; returns a new dict."""
    out = json.loads(json.dumps(record, default=str))  # deep-ish copy via JSON
    field = mutation["field"]
    kind = mutation["kind"]
    if kind == "drop_required":
        out.pop(field, None)
    elif kind in ("bad_enum", "wrong_type", "below_min", "out_of_range"):
        out[field] = mutation.get("value")
    else:
        raise ValueError(f"unknown mutation kind: {kind}")
    out["_mutation"] = f"{field}:{kind}"
    return out


def build_batches(
    clean: list[dict[str, Any]],
    mutations: list[dict[str, Any]],
    *,
    fail_count: int = 5,
    mixed_fail_ratio: float = 0.1,
    seed: int = 42,
) -> dict[str, list[dict[str, Any]]]:
    rng = random.Random(seed)
    if not clean:
        raise ValueError("no clean records to build batches from")

    pass_rows = [json.loads(json.dumps(r, default=str)) for r in clean]

    fail_rows: list[dict[str, Any]] = []
    for i in range(fail_count):
        base = clean[i % len(clean)]
        mut = mutations[i % len(mutations)]
        fail_rows.append(mutate_record(base, mut))

    mixed: list[dict[str, Any]] = []
    n = min(len(clean), 40)
    n_fail = max(1, int(n * mixed_fail_ratio))
    indices = list(range(n))
    rng.shuffle(indices)
    fail_idx = set(indices[:n_fail])
    for i in range(n):
        if i in fail_idx:
            mut = mutations[i % len(mutations)]
            mixed.append(mutate_record(clean[i], mut))
        else:
            mixed.append(json.loads(json.dumps(clean[i], default=str)))

    return {"pass": pass_rows, "fail": fail_rows, "mixed": mixed}


def env_api() -> tuple[str, str]:
    key = os.environ.get("CG_API_KEY", "").strip()
    url = os.environ.get("CG_API_URL", "https://contractgate-api.fly.dev").rstrip("/")
    return key, url
