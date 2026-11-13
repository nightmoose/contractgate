#!/usr/bin/env python3
"""Catch upstream schema drift, flag dbt impact, and draft a fix for review.

The gate is the DataContractGate CLI (`contractgate test`), which runs the
same `validate()` engine as `POST /v1/ingest/{contract_id}`. No API key and
no database. Records that fail are written under `out/catch/` and are not
treated as landed.

Nothing in this script edits the contract, the dbt project, or git history.
The fix is a unified diff under `out/draft/` for a person to read.

An optional OpenAI-compatible chat call rewrites the pull-request text when
`SCHEMA_DRIFT_LLM_BASE_URL` is set. The demo is complete without it.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]

# Unix seconds stay below 1e11 until year 5138. Milliseconds for current
# dates are about 1e12. A value at or above this line is treated as ms.
MS_THRESHOLD = 100_000_000_000
ISO_RE = re.compile(r"^\d{4}-\d{2}-\d{2}[T ]")
MODEL_TYPES = {"model", "snapshot", "seed"}
WORD_RE_CACHE: dict[str, re.Pattern[str]] = {}


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    watched = fingerprint(source_files(args))
    cli = find_cli(args.contractgate)
    dbt_bin = find_dbt()
    if args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True)

    contract, fields, synonyms = load_contract(args.contract)
    good_records = load_records(args.good)
    drifted_records = load_records(args.drifted)

    good = gate(cli, args.contract, args.good)
    drifted = gate(cli, args.contract, args.drifted)
    drifted_by_index = {row["record"]: row for row in drifted["records"]}

    rename, extra_values = pair_renames(drifted_records, drifted_by_index, fields, synonyms)
    rules = build_rules(
        fields, synonyms, good_records, drifted_records, drifted_by_index, rename, extra_values
    )

    catch_md = render_catch(args, good, drifted, drifted_records, rules)
    (args.out / "catch").mkdir(parents=True)
    (args.out / "catch" / "report.md").write_text(catch_md)
    write_json(args.out / "catch" / "good.json", strip_timing(good))
    write_json(args.out / "catch" / "drifted.json", strip_timing(drifted))
    write_split(args.out / "catch", drifted_records, drifted_by_index)

    manifest = dbt_compile(dbt_bin, args.dbt_project)
    impact_md, impact = render_impact(args, manifest, rules)
    (args.out / "impact").mkdir(parents=True)
    (args.out / "impact" / "report.md").write_text(impact_md)

    draft_dir = args.out / "draft"
    draft_dir.mkdir(parents=True)
    proposed_contract = propose_contract(args.contract.read_text(), contract, fields, rules)
    (draft_dir / "orders.yaml").write_text(proposed_contract)
    staging_path = args.dbt_project / args.staging_model
    original_sql = staging_path.read_text()
    proposed_sql = propose_sql(original_sql, rules)
    (draft_dir / "stg_orders.sql").write_text(proposed_sql)

    contract_rel = rel_to_example(args.contract)
    staging_rel = rel_to_example(staging_path)
    patch = "".join(
        [
            unified(args.contract.read_text(), proposed_contract, contract_rel),
            unified(original_sql, proposed_sql, staging_rel),
        ]
    )
    (draft_dir / "proposed.patch").write_text(patch)

    normalized = [normalize_record(rec, rules) for rec in drifted_records]
    (draft_dir / "normalized.json").write_text(json.dumps(normalized, indent=2) + "\n")
    duck_rows = prove_sql(rules, drifted_records)
    expect_rows_match(normalized, duck_rows)

    staged = gate(cli, args.staged_contract, draft_dir / "normalized.json")
    proposed_check = gate(cli, draft_dir / "orders.yaml", args.drifted)
    pr_md = render_pr(
        args, rules, impact, staged, proposed_check, drifted_records, contract_rel, staging_rel
    )
    (draft_dir / "pr_description.md").write_text(pr_md)
    maybe_llm(pr_md, draft_dir / "pr_description.llm.md")

    emit("1. Catch", catch_md)
    emit("2. Downstream impact", impact_md)
    emit("3. Draft fix (not applied)", pr_md)
    if patch.strip():
        emit("Proposed patch", patch)
    else:
        print("No file changes proposed.")

    if fingerprint(source_files(args)) != watched:
        raise SystemExit("demo check failed: the agent modified a source file")

    if is_bundled(args):
        assert_bundled(
            args,
            good,
            drifted,
            drifted_records,
            impact_md,
            patch,
            pr_md,
            normalized,
            staged,
            proposed_check,
        )
        git_apply_check(draft_dir / "proposed.patch")

    print("Draft only. No contract, model, or branch was modified.")
    print(f"Reports written under {args.out}")


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, default=ROOT / "contracts" / "orders.yaml")
    parser.add_argument(
        "--staged-contract", type=Path, default=ROOT / "contracts" / "orders_staged.yaml"
    )
    parser.add_argument("--good", type=Path, default=ROOT / "fixtures" / "orders_good.json")
    parser.add_argument("--drifted", type=Path, default=ROOT / "fixtures" / "orders_drifted.json")
    parser.add_argument("--dbt-project", type=Path, default=ROOT / "dbt")
    parser.add_argument("--staging-model", default="models/staging/stg_orders.sql")
    parser.add_argument("--out", type=Path, default=ROOT / "out")
    parser.add_argument("--contractgate", default=os.environ.get("CONTRACTGATE_BIN"))
    return parser.parse_args(argv)


def is_bundled(args: argparse.Namespace) -> bool:
    return (
        args.contract.resolve() == (ROOT / "contracts" / "orders.yaml").resolve()
        and args.good.resolve() == (ROOT / "fixtures" / "orders_good.json").resolve()
        and args.drifted.resolve() == (ROOT / "fixtures" / "orders_drifted.json").resolve()
        and args.dbt_project.resolve() == (ROOT / "dbt").resolve()
    )


def source_files(args: argparse.Namespace) -> list[Path]:
    staging = args.dbt_project / args.staging_model
    return [args.contract, args.staged_contract, args.good, args.drifted, staging]


def fingerprint(paths: list[Path]) -> dict[str, str]:
    out = {}
    for path in paths:
        out[str(path.resolve())] = hashlib.sha256(path.read_bytes()).hexdigest()
    return out


def find_cli(explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit)
        if path.is_file() and os.access(path, os.X_OK):
            return path
        raise SystemExit(f"contractgate binary is not executable: {explicit}")
    candidates = [
        REPO / "target" / "debug" / "contractgate",
        REPO / "target" / "release" / "contractgate",
    ]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
    found = shutil.which("contractgate")
    if found:
        return Path(found)
    raise SystemExit("contractgate CLI not found. From this directory run: make demo")


def find_dbt() -> str:
    venv_dbt = Path(sys.prefix) / "bin" / "dbt"
    if venv_dbt.is_file():
        return str(venv_dbt)
    found = shutil.which("dbt")
    if found:
        return found
    raise SystemExit("dbt not found. From this directory run: make demo")


def load_contract(path: Path) -> tuple[dict, dict[str, dict], dict[str, list[str]]]:
    data = yaml.safe_load(path.read_text())
    if not isinstance(data, dict) or "ontology" not in data:
        raise SystemExit(f"{path} has no ontology")
    fields = {}
    for entity in data["ontology"].get("entities") or []:
        fields[entity["name"]] = entity
    synonyms: dict[str, list[str]] = {}
    for entry in data.get("glossary") or []:
        term = entry.get("field") or entry.get("term")
        if term:
            synonyms[str(term)] = [str(item) for item in (entry.get("synonyms") or [])]
    return data, fields, synonyms


def load_records(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    if isinstance(data, dict):
        return [data]
    if isinstance(data, list):
        return data
    raise SystemExit(f"{path} must be a JSON object or array")


def gate(cli: Path, contract: Path, data: Path) -> dict:
    proc = subprocess.run(
        [
            str(cli),
            "test",
            "--contract",
            str(contract),
            "--data",
            str(data),
            "--format",
            "json",
        ],
        text=True,
        capture_output=True,
    )
    if proc.returncode == 2:
        raise SystemExit(f"contractgate could not load the contract or the data:\n{proc.stderr}")
    if proc.returncode not in (0, 1):
        raise SystemExit(
            f"contractgate exit {proc.returncode}:\n{proc.stderr}\n{proc.stdout}"
        )
    try:
        # `--format json` still prints the per-record lines before the JSON
        # document. Both go to stdout. The document is the last top-level object.
        return parse_gate_json(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"contractgate did not return JSON: {exc}\n{proc.stdout}") from exc


def parse_gate_json(stdout: str) -> dict:
    marker = "\n{"
    index = stdout.rfind(marker)
    blob = stdout[index + 1 :] if index != -1 else stdout
    return json.loads(blob)


def strip_timing(summary: dict) -> dict:
    cleaned = dict(summary)
    cleaned.pop("elapsed_ms", None)
    return cleaned


def write_json(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload, indent=2) + "\n")


def write_split(directory: Path, records: list[dict], by_index: dict[int, dict]) -> None:
    quarantined = []
    landed = []
    for index, record in enumerate(records):
        row = by_index.get(index, {"status": "fail", "violations": []})
        entry = {
            "index": index,
            "order_id": order_id(record),
            "record": record,
            "violations": row.get("violations") or [],
        }
        if row.get("status") == "pass":
            landed.append(entry)
        else:
            quarantined.append(entry)
    (directory / "landed.ndjson").write_text(
        "".join(json.dumps(item, sort_keys=True) + "\n" for item in landed)
    )
    (directory / "quarantine.ndjson").write_text(
        "".join(json.dumps(item, sort_keys=True) + "\n" for item in quarantined)
    )


def order_id(record: dict) -> str:
    if isinstance(record, dict) and record.get("order_id") is not None:
        return str(record["order_id"])
    return "(no order_id)"


def json_kind(value: object) -> str:
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if value is None:
        return "null"
    if isinstance(value, dict):
        return "object"
    if isinstance(value, list):
        return "array"
    return "other"


def types_compatible(contract_type: str, kinds: set[str]) -> bool:
    present = {kind for kind in kinds if kind != "null"}
    if not present:
        return False
    kind = contract_type.lower()
    if kind in ("number", "float"):
        return present <= {"integer", "number"}
    if kind == "integer":
        return present <= {"integer"}
    if kind in ("string", "timestamp", "date"):
        return present <= {"string"}
    if kind == "boolean":
        return present <= {"boolean"}
    return False


def token_overlap(left: str, right: str) -> bool:
    stop = {"", "id"}
    left_tokens = set(re.split(r"[_\s]+", left.lower())) - stop
    right_tokens = set(re.split(r"[_\s]+", right.lower())) - stop
    return bool(left_tokens & right_tokens)


def pair_renames(
    records: list[dict],
    by_index: dict[int, dict],
    fields: dict[str, dict],
    synonyms: dict[str, list[str]],
) -> tuple[dict[str, str], dict[str, list[float]]]:
    counts: dict[tuple[str, str], int] = {}
    extra_kinds: dict[str, set[str]] = {}
    extra_values: dict[str, list[float]] = {}
    for index, record in enumerate(records):
        row = by_index.get(index) or {}
        if row.get("status") != "fail":
            continue
        missing = [
            item["field"]
            for item in row.get("violations") or []
            if item["kind"] == "missing_required_field" and item["field"] in fields
        ]
        extras = [
            item["field"]
            for item in row.get("violations") or []
            if item["kind"] == "undeclared_field"
        ]
        for missing_name in missing:
            for extra_name in extras:
                counts[(missing_name, extra_name)] = counts.get((missing_name, extra_name), 0) + 1
                extra_kinds.setdefault(extra_name, set()).add(json_kind(record.get(extra_name)))
                value = record.get(extra_name)
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    extra_values.setdefault(extra_name, []).append(float(value))

    rename: dict[str, str] = {}
    used: set[str] = set()
    missing_names = sorted({pair[0] for pair in counts})
    for missing_name in missing_names:
        candidates = []
        for (old, new), count in counts.items():
            if old != missing_name or new in used:
                continue
            if not types_compatible(str(fields[old].get("type", "")), extra_kinds.get(new, set())):
                continue
            score = 1
            if new in set(synonyms.get(old, [])):
                score += 5
            if token_overlap(old, new):
                score += 3
            candidates.append((score, count, new))
        if not candidates:
            continue
        candidates.sort(reverse=True)
        if len(candidates) == 1 or candidates[0][0] > candidates[1][0]:
            winner = candidates[0][2]
            rename[missing_name] = winner
            used.add(winner)
    return rename, extra_values


def numeric_values(records: list[dict], name: str) -> list[float]:
    values = []
    for record in records:
        value = record.get(name)
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            values.append(float(value))
    return values


def median(values: list[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def choose_factor(
    values: list[float],
    low: float | None,
    high: float | None,
    baseline: list[float],
) -> int | None:
    """Pick 100 (cents) or 1000 (milliseconds) when scaling lands in range."""
    if not values:
        return None
    fitting = []
    need = max(1, math.ceil(0.75 * len(values)))
    for factor in (100, 1000):
        fits = 0
        for value in values:
            scaled = value / factor
            if low is not None and scaled < low - 1e-9:
                continue
            if high is not None and scaled > high + 1e-9:
                continue
            fits += 1
        if fits >= need:
            fitting.append(factor)
    if not fitting:
        return None
    if len(fitting) == 1:
        return fitting[0]
    base = median(baseline)
    mid = median(values)
    if base in (None, 0) or mid is None:
        return fitting[0]
    return min(fitting, key=lambda factor: abs(factor - (mid / base)))


def bound(field_def: dict, key: str) -> float | None:
    value = field_def.get(key)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return None


@dataclass
class FieldFix:
    kinds: set[str] = field(default_factory=set)
    messages: list[str] = field(default_factory=list)
    ids_by_kind: dict[str, list[str]] = field(default_factory=dict)
    range_values: list[float] = field(default_factory=list)
    type_values: list[object] = field(default_factory=list)
    factor: int | None = None
    renamed_to: str | None = None
    dropped: bool = False
    iso_strings: bool = False


@dataclass
class Rules:
    fields: dict[str, FieldFix]
    order_total_factor: int | None = None
    amount_factor: int | None = None
    amount_max: float | None = None
    time_factor: int | None = None
    timestamp_strings: bool = False
    timestamp_ms: bool = False
    optional: set[str] = field(default_factory=set)
    new_fields: dict[str, dict] = field(default_factory=dict)


def build_rules(
    fields: dict[str, dict],
    synonyms: dict[str, list[str]],
    good_records: list[dict],
    drifted_records: list[dict],
    by_index: dict[int, dict],
    rename: dict[str, str],
    extra_values: dict[str, list[float]],
) -> Rules:
    rules = Rules(fields={})
    for index, record in enumerate(drifted_records):
        row = by_index.get(index) or {}
        if row.get("status") != "fail":
            continue
        seen = set()
        for item in row.get("violations") or []:
            name = item["field"]
            kind = item["kind"]
            fix = rules.fields.setdefault(name, FieldFix())
            fix.kinds.add(kind)
            key = (kind, item["message"])
            if key not in seen:
                seen.add(key)
                if item["message"] not in fix.messages:
                    fix.messages.append(item["message"])
            fix.ids_by_kind.setdefault(kind, [])
            oid = order_id(record)
            if oid not in fix.ids_by_kind[kind]:
                fix.ids_by_kind[kind].append(oid)
            if kind == "range_violation":
                value = record.get(name)
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    fix.range_values.append(float(value))
            if kind == "type_mismatch":
                fix.type_values.append(record.get(name))

    for old, new in rename.items():
        fix = rules.fields.setdefault(old, FieldFix())
        fix.renamed_to = new
        rules.fields.setdefault(new, FieldFix())

    for name, fix in rules.fields.items():
        if "missing_required_field" in fix.kinds and name not in rename:
            fix.dropped = True
            rules.optional.add(name)
        if name in rename:
            rules.optional.add(name)

    amount_def = fields.get("amount", {})
    amount_low, amount_high = bound(amount_def, "min"), bound(amount_def, "max")
    amount_base = numeric_values(good_records, "amount")
    amount_fix = rules.fields.get("amount")
    if amount_fix and amount_fix.range_values:
        amount_fix.factor = choose_factor(amount_fix.range_values, amount_low, amount_high, amount_base)
        if amount_fix.factor:
            rules.amount_factor = amount_fix.factor
            rules.amount_max = amount_high

    if "amount" in rename:
        new_name = rename["amount"]
        factor = choose_factor(extra_values.get(new_name, []), amount_low, amount_high, amount_base)
        if factor:
            rules.order_total_factor = factor
            new_fix = rules.fields.setdefault(new_name, FieldFix())
            new_fix.factor = factor
            high = None if amount_high is None else amount_high * factor
            low = 0 if amount_low is None else amount_low * factor
            rules.new_fields[new_name] = {
                "type": amount_def.get("type", "number"),
                "required": False,
                "min": low,
                "max": high,
                "note": f"New wire name for amount. Divide by {factor} in staging.",
            }

    ordered = fields.get("ordered_at", {})
    time_low, time_high = bound(ordered, "min"), bound(ordered, "max")
    time_base = numeric_values(good_records, "ordered_at")
    time_fix = rules.fields.get("ordered_at")
    if time_fix:
        if time_fix.range_values:
            factor = choose_factor(time_fix.range_values, time_low, time_high, time_base)
            if factor == 1000:
                time_fix.factor = 1000
                rules.time_factor = 1000
                rules.timestamp_ms = True
        strings = [value for value in time_fix.type_values if isinstance(value, str)]
        if strings and all(ISO_RE.match(value) for value in strings):
            time_fix.iso_strings = True
            rules.timestamp_strings = True
    return rules


def ids(fix: FieldFix, kind: str) -> str:
    return ", ".join(fix.ids_by_kind.get(kind, [])) or "none"


def render_catch(args: argparse.Namespace, good: dict, drifted: dict, records: list[dict], rules: Rules) -> str:
    contract = rel_display(args.contract)
    good_name = rel_display(args.good)
    drifted_name = rel_display(args.drifted)
    lines = [
        "# 1. Catch",
        "",
        f"Command: `contractgate test --contract {contract} --data {drifted_name} --format json`",
        "",
        "This is the DataContractGate CLI. It runs the same `validate()` engine as",
        "`POST /v1/ingest/{contract_id}`. Failed records are written to",
        "`out/catch/quarantine.ndjson` and are not written to `out/catch/landed.ndjson`.",
        "The CLI does not open Postgres, so those files are the local stand-in for",
        "`quarantine_events`. A running gateway stores the same failures when you POST",
        "the batch to ingest.",
        "",
        f"## Good batch — `{good_name}`",
        "",
        f"{good['pass']} passed, {good['fail']} failed.",
        "",
        f"## Drifted batch — `{drifted_name}`",
        "",
        f"{drifted['pass']} passed, {drifted['fail']} failed.",
        "",
        "## What failed",
        "",
    ]
    lines.extend(classification_lines(rules))
    lines.extend(["", "### Landed", ""])
    by_index = {row["record"]: row for row in drifted["records"]}
    landed = False
    for index, record in enumerate(records):
        row = by_index.get(index, {})
        if row.get("status") == "pass":
            landed = True
            lines.append(f"- record {index} `{order_id(record)}` passed and would land")
    if not landed:
        lines.append("- none")
    lines.extend(["", "### Quarantined", ""])
    any_fail = False
    for index, record in enumerate(records):
        row = by_index.get(index, {})
        if row.get("status") == "pass":
            continue
        any_fail = True
        lines.append(f"#### record {index} — `{order_id(record)}`")
        lines.append("")
        lines.append("| field | kind | why |")
        lines.append("| --- | --- | --- |")
        for item in row.get("violations") or []:
            why = item["message"].replace("|", "\\|")
            lines.append(f"| `{item['field']}` | `{item['kind']}` | {why} |")
        lines.append("")
    if not any_fail:
        lines.append("None.")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def classification_lines(rules: Rules) -> list[str]:
    lines = []
    amount = rules.fields.get("amount")
    if amount and amount.renamed_to:
        lines.append(
            f"- Renamed field: `amount` is missing where `{amount.renamed_to}` was sent ({ids(amount, 'missing_required_field')})."
        )
    customer = rules.fields.get("customer_id")
    if customer and customer.dropped:
        lines.append(f"- Dropped field: `customer_id` ({ids(customer, 'missing_required_field')}).")
    ordered = rules.fields.get("ordered_at")
    if ordered and ordered.iso_strings:
        lines.append(
            f"- Type change: `ordered_at` is an RFC3339 string, the contract wants an integer ({ids(ordered, 'type_mismatch')})."
        )
    semantic = []
    if amount and amount.factor == 100:
        semantic.append(
            f"`amount` is a number above the dollar max, at about 100× ({ids(amount, 'range_violation')})"
        )
    if ordered and ordered.factor == 1000:
        semantic.append(
            f"`ordered_at` is an integer above the seconds max, at about 1000× ({ids(ordered, 'range_violation')})"
        )
    if semantic:
        lines.append("- Schema-valid but wrong: " + "; ".join(semantic) + ".")
    if not lines:
        lines.append("- No rename, drop, type change, or unit shift detected.")
    return lines


def dbt_compile(dbt_bin: str, project: Path) -> dict:
    env = os.environ.copy()
    env["DBT_SEND_ANONYMOUS_USAGE_STATS"] = "false"
    env["DBT_USE_COLORS"] = "false"
    env["DO_NOT_TRACK"] = "1"
    base = [
        dbt_bin,
        "compile",
        "--project-dir",
        str(project),
        "--target",
        "dev",
    ]
    # This example ships a DuckDB profiles.yml next to the project. A project
    # that already uses ~/.dbt/profiles.yml is left on that default.
    if (project / "profiles.yml").is_file():
        base.extend(["--profiles-dir", str(project)])
    last = ""
    for extra in (["--no-partial-parse"], []):
        proc = subprocess.run(base + extra, cwd=project, env=env, text=True, capture_output=True)
        last = proc.stdout + "\n" + proc.stderr
        if proc.returncode == 0:
            manifest_path = project / "target" / "manifest.json"
            return json.loads(manifest_path.read_text())
        lowered = last.lower()
        if extra and "no-partial-parse" in lowered and (
            "unrecognized" in lowered or "no such option" in lowered
        ):
            continue
        raise SystemExit("dbt compile failed:\n" + last)
    raise SystemExit("dbt compile failed:\n" + last)


def node_text(node: dict) -> str:
    return "\n".join([node.get("raw_code") or "", node.get("compiled_code") or ""])


def word_in(text: str, name: str) -> bool:
    pattern = WORD_RE_CACHE.get(name)
    if pattern is None:
        pattern = re.compile(rf"\b{re.escape(name)}\b")
        WORD_RE_CACHE[name] = pattern
    return pattern.search(text) is not None


def test_column(node: dict) -> str | None:
    if node.get("column_name"):
        return str(node["column_name"])
    kwargs = (node.get("test_metadata") or {}).get("kwargs") or {}
    if kwargs.get("column_name"):
        return str(kwargs["column_name"])
    return None


def references_column(node: dict, column: str) -> bool:
    if test_column(node) == column:
        return True
    columns = node.get("columns") or {}
    if column in columns:
        return True
    return word_in(node_text(node), column)


def collect_nodes(manifest: dict) -> dict[str, dict]:
    nodes: dict[str, dict] = {}
    for uid, node in (manifest.get("nodes") or {}).items():
        nodes[uid] = node
    for uid, node in (manifest.get("sources") or {}).items():
        copied = dict(node)
        copied.setdefault("resource_type", "source")
        nodes[uid] = copied
    for uid, node in (manifest.get("exposures") or {}).items():
        copied = dict(node)
        copied.setdefault("resource_type", "exposure")
        nodes[uid] = copied
    return nodes


def children_of(manifest: dict, nodes: dict[str, dict]) -> dict[str, list[str]]:
    children: dict[str, list[str]] = {}
    raw = manifest.get("child_map") or {}
    for parent, kids in raw.items():
        children[parent] = list(kids)
    for uid, node in nodes.items():
        if node.get("resource_type") != "exposure":
            continue
        for parent in (node.get("depends_on") or {}).get("nodes") or []:
            kids = children.setdefault(parent, [])
            if uid not in kids:
                kids.append(uid)
    return children


def exposure_title(node: dict) -> str:
    description = (node.get("description") or "").strip()
    if description and "\n" not in description and len(description) <= 80:
        title = description
    else:
        title = (node.get("name") or "exposure").replace("_", " ")
    return f"exposure '{title}'"


def render_impact(args: argparse.Namespace, manifest: dict, rules: Rules) -> tuple[str, dict]:
    nodes = collect_nodes(manifest)
    children = children_of(manifest, nodes)
    parents: dict[str, list[str]] = {}
    for parent, kids in children.items():
        for kid in kids:
            parents.setdefault(kid, []).append(parent)

    n_models = sum(1 for node in nodes.values() if node.get("resource_type") == "model")
    n_tests = sum(1 for node in nodes.values() if node.get("resource_type") == "test")
    n_exposures = sum(1 for node in nodes.values() if node.get("resource_type") == "exposure")
    lines = [
        "# 2. Downstream impact",
        "",
        f"Manifest: `{rel_display(args.dbt_project / 'target' / 'manifest.json')}` "
        f"({n_models} models, {n_tests} tests, {n_exposures} exposures).",
        "",
        "A model is listed when its SQL or schema columns name the field. An exposure",
        "is listed when it depends on one of those models. A test is listed when its",
        "column or SQL names the field. dbt Core records model-level `child_map` edges;",
        "this scan is how a column is attached to those edges. It is not warehouse",
        "column lineage.",
        "",
    ]
    sections = []
    contract_order = list(rules.fields)
    # Stable: known contract fields first (amount, customer_id, ordered_at), then extras.
    preferred = ["amount", "customer_id", "ordered_at"]
    ordered_names = [name for name in preferred if name in rules.fields]
    ordered_names.extend(name for name in contract_order if name not in ordered_names)
    arrow_lines = []
    for name in ordered_names:
        fix = rules.fields[name]
        if name in rules.new_fields and not fix.kinds and not fix.renamed_to:
            pass
        section, arrows = impact_section(name, fix, rules, nodes, children, parents)
        sections.append(section)
        arrow_lines.extend(arrows)
    lines.extend(sections)
    return "\n".join(lines).rstrip() + "\n", {"arrows": arrow_lines}


def impact_section(
    name: str,
    fix: FieldFix,
    rules: Rules,
    nodes: dict[str, dict],
    children: dict[str, list[str]],
    parents: dict[str, list[str]],
) -> tuple[str, list[str]]:
    affected_models = {
        uid
        for uid, node in nodes.items()
        if node.get("resource_type") in MODEL_TYPES and references_column(node, name)
    }
    affected_exposures = set()
    for uid, node in nodes.items():
        if node.get("resource_type") != "exposure":
            continue
        depends = (node.get("depends_on") or {}).get("nodes") or []
        if any(parent in affected_models for parent in depends):
            affected_exposures.add(uid)

    lines = [f"## `{name}`", ""]
    lines.extend(field_prose(name, fix, rules))
    lines.append("")
    arrows = []
    if affected_models:
        anchors = [
            uid
            for uid in affected_models
            if not any(parent in affected_models for parent in parents.get(uid, []))
        ]
        anchors.sort(key=lambda uid: nodes[uid].get("name") or uid)
        reason = short_reason(fix)
        for anchor in anchors:
            arrows.extend(walk_paths(anchor, reason, name, affected_models, affected_exposures, nodes, children))
        arrows = list(dict.fromkeys(arrows))
        if arrows:
            lines.extend(arrows)
            lines.append("")
        else:
            model_names = ", ".join(sorted(nodes[uid]["name"] for uid in affected_models))
            lines.append(f"Models that name `{name}`: {model_names}.")
            lines.append("")
    else:
        lines.append(f"No model names `{name}`.")
        origin = next(
            (old for old, old_fix in rules.fields.items() if old_fix.renamed_to == name),
            None,
        )
        if origin:
            lines.append(f"It is the new name for `{origin}`. Follow `{origin}` for downstream impact.")
        lines.append("")

    tests = []
    for uid, node in nodes.items():
        if node.get("resource_type") != "test":
            continue
        if not references_column(node, name):
            continue
        tests.append(node)
    tests.sort(key=lambda node: node.get("name") or "")
    if tests:
        lines.append("Tests:")
        lines.append("")
        for node in tests:
            column = test_column(node)
            attached = nodes.get(node.get("attached_node") or "", {}).get("name")
            if column and attached:
                lines.append(f"- `{node.get('name')}` on `{attached}.{column}`")
            else:
                lines.append(f"- `{node.get('name')}`")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n", arrows


def field_prose(name: str, fix: FieldFix, rules: Rules) -> list[str]:
    lines = []
    if fix.renamed_to:
        lines.append(
            f"Renamed to `{fix.renamed_to}` on {ids(fix, 'missing_required_field')}."
        )
    if fix.dropped:
        lines.append(f"Dropped on {ids(fix, 'missing_required_field')}. No same-type replacement was paired.")
    if fix.iso_strings:
        lines.append(
            f"Changed type from integer to an RFC3339 string on {ids(fix, 'type_mismatch')}."
        )
    elif "type_mismatch" in fix.kinds:
        lines.append(
            f"Changed type on {ids(fix, 'type_mismatch')}. The values were not RFC3339, so the draft does not guess a cast."
        )
    if fix.factor == 100 and fix.ids_by_kind.get("range_violation"):
        lines.append(
            f"Changed meaning (dollars → cents, divide by 100) on {ids(fix, 'range_violation')}."
        )
    elif fix.factor == 1000 and fix.ids_by_kind.get("range_violation"):
        lines.append(
            f"Changed meaning (seconds → milliseconds, divide by 1000) on {ids(fix, 'range_violation')}."
        )
    elif "range_violation" in fix.kinds:
        lines.append(f"Out of range on {ids(fix, 'range_violation')}. No cents or milliseconds factor fitted.")
    if name in rules.new_fields and not lines:
        lines.append(rules.new_fields[name]["note"])
    if not lines:
        kinds = ", ".join(sorted(fix.kinds)) or "flagged"
        lines.append(f"Contract violations: {kinds}.")
    return lines


def short_reason(fix: FieldFix) -> str:
    bits = []
    if fix.iso_strings or "type_mismatch" in fix.kinds:
        bits.append("changed type")
    if fix.factor:
        bits.append("changed meaning")
    if not bits and fix.renamed_to:
        bits.append(f"renamed to `{fix.renamed_to}`")
    if not bits and fix.dropped:
        bits.append("dropped")
    if not bits:
        bits.append("failed the contract")
    return " / ".join(bits)


def walk_paths(
    anchor: str,
    reason: str,
    column: str,
    affected_models: set[str],
    affected_exposures: set[str],
    nodes: dict[str, dict],
    children: dict[str, list[str]],
) -> list[str]:
    paths: list[str] = []

    def walk(uid: str, names: list[str]) -> None:
        kids = sorted(children.get(uid, []), key=lambda item: nodes.get(item, {}).get("name") or item)
        for kid in kids:
            node = nodes.get(kid)
            if node is None:
                continue
            kind = node.get("resource_type")
            if kind in MODEL_TYPES and kid in affected_models:
                walk(kid, names + [node.get("name") or kid])
            elif kind == "exposure" and kid in affected_exposures:
                rendered = names + [exposure_title(node)]
                paths.append(f"- `{column}` {reason} -> " + " -> ".join(rendered))

    walk(anchor, [nodes[anchor].get("name") or anchor])
    return paths


def fmt_num(value: float) -> str:
    if float(value).is_integer():
        return str(int(value))
    return str(value)


def amount_expr(rules: Rules) -> str | None:
    branches = []
    if rules.order_total_factor:
        branches.append(
            f"when order_total is not null then order_total / {int(rules.order_total_factor)}.0"
        )
    if rules.amount_factor and rules.amount_max is not None:
        branches.append(
            f"when amount > {fmt_num(rules.amount_max)} then amount / {int(rules.amount_factor)}.0"
        )
    if not branches:
        return None
    body = "\n    ".join(branches)
    return f"case\n    {body}\n    else amount\nend"


def ordered_at_expr(rules: Rules) -> str | None:
    branches = []
    if rules.timestamp_strings:
        branches.append(
            "when try_cast(ordered_at as varchar) like '%-%' "
            "then cast(epoch(try_cast(ordered_at as timestamptz)) as bigint)"
        )
    if rules.timestamp_ms and rules.time_factor:
        branches.append(
            f"when try_cast(ordered_at as bigint) >= {MS_THRESHOLD} "
            f"then cast(try_cast(ordered_at as bigint) / {int(rules.time_factor)} as bigint)"
        )
    if not branches:
        return None
    body = "\n    ".join(branches)
    return f"case\n    {body}\n    else try_cast(ordered_at as bigint)\nend"


def propose_sql(original: str, rules: Rules) -> str:
    updated = original
    amount = amount_expr(rules)
    ordered = ordered_at_expr(rules)
    if amount:
        updated, _ = replace_bare_column(updated, "amount", amount)
    if ordered:
        updated, _ = replace_bare_column(updated, "ordered_at", ordered)
    banner = (
        "-- DRAFT — review before applying. schema-drift-agent does not apply this file.\n"
        "-- Puts amount back in US dollars and ordered_at back in unix seconds.\n"
        "-- customer_id is passed through and is null where the feed dropped it.\n"
    )
    if not updated.startswith("-- DRAFT"):
        updated = banner + updated
    if not updated.endswith("\n"):
        updated += "\n"
    return updated


def replace_bare_column(sql: str, column: str, expr: str) -> tuple[str, bool]:
    pattern = re.compile(rf"^([ \t]*){re.escape(column)}[ \t]*,?[ \t]*$", re.M)
    match = pattern.search(sql)
    if not match:
        comment = "\n".join("-- " + line for line in expr.splitlines())
        note = (
            f"-- DRAFT expression for {column} (the select item was not a bare column):\n"
            f"{comment}\n"
        )
        return note + sql, False
    indent = match.group(1)
    comma = match.group(0).rstrip().endswith(",")
    lines = [indent + line for line in expr.strip().splitlines()]
    lines[-1] = lines[-1] + f" as {column}" + ("," if comma else "")
    return sql[: match.start()] + "\n".join(lines) + sql[match.end() :], True


def propose_contract(original: str, contract: dict, fields: dict[str, dict], rules: Rules) -> str:
    text = original
    version = bump_version(str(contract.get("version", "1.0")))
    text = re.sub(r"^version:.*$", f'version: "{version}"', text, count=1, flags=re.M)
    description = (
        "Upstream orders feed. Legacy amount is US dollars. "
        "order_total, when present, is US cents. "
        "ordered_at stays unix seconds in this draft. "
        "customer_id may be absent on the raw feed."
    )
    text = re.sub(
        r"^description:.*$",
        f'description: "{description}"',
        text,
        count=1,
        flags=re.M,
    )
    for name in sorted(rules.optional):
        text = set_required(text, name, False)
    for name, spec in rules.new_fields.items():
        if field_block(text.splitlines(), name) is not None:
            continue
        block = render_field(name, spec)
        anchor = spec_anchor(rules, name)
        text = insert_after_field(text, anchor, block)
    if rules.timestamp_strings or rules.timestamp_ms:
        note = (
            "    # Draft keeps unix seconds. Drifted rows also sent RFC3339 strings\n"
            "    # and/or milliseconds. Those still fail this type and range.\n"
            "    # Staging casts both back to seconds. Switch to type:timestamp\n"
            "    # only if the producer standardizes on RFC3339.\n"
        )
        text = insert_before_field(text, "ordered_at", note)
    if not text.endswith("\n"):
        text += "\n"
    return text


def spec_anchor(rules: Rules, new_name: str) -> str:
    for old, fix in rules.fields.items():
        if fix.renamed_to == new_name:
            return old
    return "amount"


def bump_version(version: str) -> str:
    parts = version.split(".")
    if len(parts) >= 2 and parts[1].isdigit():
        parts[1] = str(int(parts[1]) + 1)
        return ".".join(parts)
    return version + ".1"


def field_block(lines: list[str], name: str) -> tuple[int, int] | None:
    start = None
    indent = ""
    for index, line in enumerate(lines):
        match = re.match(rf"^(\s*)- name: {re.escape(name)}\s*$", line)
        if match:
            start = index
            indent = match.group(1)
            break
    if start is None:
        return None
    end = start + 1
    while end < len(lines):
        line = lines[end]
        if re.match(rf"^{re.escape(indent)}- name:", line):
            break
        if line.strip() and (len(line) - len(line.lstrip())) < len(indent):
            break
        end += 1
    return start, end


def set_required(text: str, name: str, required: bool) -> str:
    lines = text.splitlines()
    span = field_block(lines, name)
    if span is None:
        return text
    start, end = span
    target = "true" if required else "false"
    for index in range(start, end):
        if re.match(r"^\s+required:\s+", lines[index]):
            lines[index] = re.sub(r"required:\s+\S+", f"required: {target}", lines[index], count=1)
            break
    return "\n".join(lines) + ("\n" if text.endswith("\n") else "")


def render_field(name: str, spec: dict) -> str:
    lines = []
    if spec.get("note"):
        lines.append(f"    # {spec['note']}")
    lines.append(f"    - name: {name}")
    lines.append(f"      type: {spec.get('type', 'number')}")
    lines.append(f"      required: {'true' if spec.get('required') else 'false'}")
    if spec.get("min") is not None:
        lines.append(f"      min: {fmt_num(float(spec['min']))}")
    if spec.get("max") is not None:
        lines.append(f"      max: {fmt_num(float(spec['max']))}")
    return "\n".join(lines) + "\n"


def insert_after_field(text: str, name: str, block: str) -> str:
    lines = text.splitlines(keepends=True)
    span = field_block([line.rstrip("\n") for line in lines], name)
    if span is None:
        return text + "\n" + block
    _start, end = span
    addition = block if block.endswith("\n") else block + "\n"
    # `end` is an index into logical lines; keepends means the same index.
    return "".join(lines[:end]) + addition + "".join(lines[end:])


def insert_before_field(text: str, name: str, block: str) -> str:
    lines = text.splitlines(keepends=True)
    span = field_block([line.rstrip("\n") for line in lines], name)
    if span is None:
        return text
    start, _end = span
    if start > 0 and "Draft keeps unix seconds" in lines[start - 1]:
        return text
    addition = block if block.endswith("\n") else block + "\n"
    return "".join(lines[:start]) + addition + "".join(lines[start:])


def scale(value: float, factor: int) -> int | float:
    quantized = (Decimal(str(value)) / Decimal(factor)).quantize(
        Decimal("0.01"), rounding=ROUND_HALF_UP
    )
    if quantized == quantized.to_integral_value():
        return int(quantized)
    return float(quantized)


def normalize_time(value: object, rules: Rules) -> object:
    if isinstance(value, str) and rules.timestamp_strings and ("-" in value or "T" in value):
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp())
    if (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and rules.timestamp_ms
        and rules.time_factor
        and value >= MS_THRESHOLD
    ):
        return int(value // rules.time_factor)
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return value


def normalize_record(record: dict, rules: Rules) -> dict:
    normalized: dict = {}
    for key in ("order_id", "status", "currency"):
        if key in record:
            normalized[key] = record[key]
    customer = record.get("customer_id")
    if customer not in (None, ""):
        normalized["customer_id"] = customer
    amount = None
    if record.get("order_total") is not None and rules.order_total_factor:
        amount = scale(float(record["order_total"]), rules.order_total_factor)
    elif record.get("amount") is not None:
        raw_amount = record["amount"]
        if (
            rules.amount_factor
            and rules.amount_max is not None
            and isinstance(raw_amount, (int, float))
            and not isinstance(raw_amount, bool)
            and raw_amount > rules.amount_max
        ):
            amount = scale(float(raw_amount), rules.amount_factor)
        else:
            amount = raw_amount
    if amount is not None:
        normalized["amount"] = amount
    if "ordered_at" in record:
        normalized["ordered_at"] = normalize_time(record["ordered_at"], rules)
    return normalized


def prove_sql(rules: Rules, records: list[dict]) -> list[dict]:
    import duckdb

    amount = amount_expr(rules) or "amount"
    ordered = ordered_at_expr(rules) or "try_cast(ordered_at as bigint)"
    connection = duckdb.connect()
    try:
        connection.execute("create table raw(ord integer, doc json)")
        connection.executemany(
            "insert into raw values (?, ?)",
            [[index, json.dumps(record)] for index, record in enumerate(records)],
        )
        query = f"""
        with src as (
          select
            ord,
            json_extract_string(doc, '$.order_id') as order_id,
            json_extract_string(doc, '$.customer_id') as customer_id,
            json_extract_string(doc, '$.status') as status,
            try_cast(json_extract_string(doc, '$.amount') as double) as amount,
            try_cast(json_extract_string(doc, '$.order_total') as double) as order_total,
            json_extract_string(doc, '$.currency') as currency,
            json_extract_string(doc, '$.ordered_at') as ordered_at
          from raw
        )
        select
          ord,
          order_id,
          customer_id,
          status,
          {amount} as amount,
          currency,
          {ordered} as ordered_at
        from src
        order by ord
        """
        rows = connection.execute(query).fetchall()
    finally:
        connection.close()
    out = []
    for row in rows:
        _ord, order_id_value, customer, status, amount_value, currency, ordered_value = row
        item = {
            "order_id": order_id_value,
            "status": status,
            "amount": coerce_number(amount_value),
            "currency": currency,
            "ordered_at": None if ordered_value is None else int(ordered_value),
        }
        if customer is not None:
            item["customer_id"] = customer
        out.append(item)
    return out


def coerce_number(value: object) -> object:
    if value is None:
        return None
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    number = float(value)
    if number.is_integer():
        return int(number)
    return round(number, 2)


def expect_rows_match(python_rows: list[dict], duck_rows: list[dict]) -> None:
    if len(python_rows) != len(duck_rows):
        raise SystemExit(
            f"demo check failed: Python normalized {len(python_rows)} rows, DuckDB returned {len(duck_rows)}"
        )
    for index, (left, right) in enumerate(zip(python_rows, duck_rows)):
        for key in ("order_id", "customer_id", "status", "currency", "ordered_at"):
            if left.get(key) != right.get(key):
                raise SystemExit(
                    f"demo check failed: row {index} {key} Python={left.get(key)!r} DuckDB={right.get(key)!r}"
                )
        if not numbers_close(left.get("amount"), right.get("amount")):
            raise SystemExit(
                f"demo check failed: row {index} amount Python={left.get('amount')!r} DuckDB={right.get('amount')!r}"
            )


def numbers_close(left: object, right: object) -> bool:
    if left is None or right is None:
        return left is right
    return abs(float(left) - float(right)) < 0.02


def unified(old: str, new: str, path: str) -> str:
    if not old.endswith("\n"):
        old += "\n"
    if not new.endswith("\n"):
        new += "\n"
    if old == new:
        return ""
    lines = difflib.unified_diff(
        old.splitlines(keepends=True),
        new.splitlines(keepends=True),
        fromfile=f"a/{path}",
        tofile=f"b/{path}",
    )
    return "".join(lines)


def rel_to_example(path: Path) -> str:
    resolved = path.resolve()
    try:
        return str(resolved.relative_to(ROOT))
    except ValueError:
        return str(path)


def rel_display(path: Path) -> str:
    return rel_to_example(path)


def render_pr(
    args: argparse.Namespace,
    rules: Rules,
    impact: dict,
    staged: dict,
    proposed: dict,
    records: list[dict],
    contract_rel: str,
    staging_rel: str,
) -> str:
    lines = [
        "# Draft: adapt orders staging to the upstream schema change",
        "",
        "Draft for review. Nothing was applied or merged. schema-drift-agent does not",
        "switch a branch or edit the working tree.",
        "",
        "## What the gate caught",
        "",
    ]
    lines.extend(classification_lines(rules))
    lines.extend(["", "## Downstream", ""])
    if impact["arrows"]:
        lines.extend(impact["arrows"])
    else:
        lines.append("No downstream model references the drifted columns.")
    lines.extend(
        [
            "",
            "## Proposed change",
            "",
            f"- `{staging_rel}` maps `order_total` and out-of-range `amount` back to dollars, and casts RFC3339 or millisecond `ordered_at` back to unix seconds. The SQL is DuckDB (`try_cast`, `epoch`), matching this dbt project.",
            f"- `{contract_rel}` declares `order_total` as cents, and stops requiring `amount` and `customer_id` on the raw feed.",
            f"- `{rel_display(args.staged_contract)}` is unchanged. It still requires dollars, unix seconds, and `customer_id`.",
            "",
            "## Staged contract preview",
            "",
            "The drifted batch was normalized with the same rules as the staging SQL and",
            f"checked with `contractgate test` against `{rel_display(args.staged_contract)}`.",
            "",
        ]
    )
    lines.extend(result_lines(staged, records))
    lines.extend(
        [
            "",
            "## Raw contract draft, still not published",
            "",
            "The drifted batch checked against the proposed contract:",
            "",
        ]
    )
    lines.extend(result_lines(proposed, records))
    lines.extend(
        [
            "",
            "## Review before accepting",
            "",
            "- `amount` values above the contract max are treated as cents. A cents value that still falls under that max would pass the range check and would not be divided. Prefer the renamed `order_total` field, which is always divided.",
            "- `ordered_at` stays an integer of unix seconds in the contract. The drifted batch mixes RFC3339 strings and integers, and one field has one type. Strings and milliseconds keep failing ingest until you pick a wire type. The staging cast is ready for both.",
            "- `customer_id` was dropped on some rows. The draft does not invent it. Those rows fail the staged contract.",
            "",
            "## Apply",
            "",
            "From `examples/schema-drift-agent`, if you accept the draft:",
            "",
            "```",
            "git apply --check out/draft/proposed.patch",
            "git apply out/draft/proposed.patch",
            "```",
            "",
            "The demo stops at `--check` only when it verifies the patch. It does not apply it.",
            "",
        ]
    )
    return "\n".join(lines).rstrip() + "\n"


def result_lines(summary: dict, records: list[dict]) -> list[str]:
    lines = []
    by_index = {row["record"]: row for row in summary["records"]}
    for index, record in enumerate(records):
        row = by_index.get(index, {})
        if row.get("status") == "pass":
            lines.append(f"- `{order_id(record)}` pass")
            continue
        kinds = ", ".join(
            f"`{item['field']}` {item['kind']}" for item in row.get("violations") or []
        )
        lines.append(f"- `{order_id(record)}` fail: {kinds}")
    return lines


def maybe_llm(heuristic: str, dest: Path) -> None:
    base = os.environ.get("SCHEMA_DRIFT_LLM_BASE_URL", "").strip()
    if not base:
        return
    model = os.environ.get("SCHEMA_DRIFT_LLM_MODEL", "gpt-4o-mini").strip() or "gpt-4o-mini"
    key = os.environ.get("SCHEMA_DRIFT_LLM_API_KEY", "")
    url = base.rstrip("/")
    if not url.endswith("/chat/completions"):
        url += "/chat/completions"
    payload = {
        "model": model,
        "temperature": 0,
        "messages": [
            {
                "role": "system",
                "content": (
                    "You edit a pull request description for a human reviewer. "
                    "Keep every column name, model name, and number from the draft. "
                    "Do not claim the change was applied or merged. "
                    "Do not add a conversion the draft does not already state. "
                    "Return only the revised markdown."
                ),
            },
            {"role": "user", "content": heuristic},
        ],
    }
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        method="POST",
    )
    request.add_header("Content-Type", "application/json")
    if key:
        request.add_header("Authorization", f"Bearer {key}")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = json.loads(response.read().decode())
        text = body["choices"][0]["message"]["content"].strip() + "\n"
    except (urllib.error.URLError, KeyError, IndexError, json.JSONDecodeError, TimeoutError) as exc:
        print(f"LLM draft skipped: {exc}", file=sys.stderr)
        return
    dest.write_text(text)
    print(f"Wrote an optional LLM revision to {dest}. The heuristic draft was not replaced.")


def git_apply_check(patch: Path) -> None:
    proc = subprocess.run(
        ["git", "apply", "--check", str(patch)],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        raise SystemExit(f"demo check failed: git apply --check rejected the patch:\n{proc.stderr}")


def assert_bundled(
    args: argparse.Namespace,
    good: dict,
    drifted: dict,
    records: list[dict],
    impact_md: str,
    patch: str,
    pr_md: str,
    normalized: list[dict],
    staged: dict,
    proposed: dict,
) -> None:
    expect(good["pass"] == 4 and good["fail"] == 0, f"good batch {good['pass']}/{good['fail']}")
    expect(drifted["pass"] == 1 and drifted["fail"] == 5, f"drifted batch {drifted['pass']}/{drifted['fail']}")
    quarantine = (args.out / "catch" / "quarantine.ndjson").read_text().strip().splitlines()
    landed = (args.out / "catch" / "landed.ndjson").read_text().strip().splitlines()
    expect(len(quarantine) == 5, f"quarantine lines {len(quarantine)}")
    expect(len(landed) == 1 and "ord_1001" in landed[0], "landed record")
    expect(
        "`amount` changed meaning -> stg_orders -> fct_revenue -> exposure 'Revenue dashboard'"
        in impact_md,
        "amount revenue path missing:\n" + impact_md,
    )
    expect(
        "`customer_id` dropped -> stg_orders -> fct_orders -> exposure 'Orders dashboard'"
        in impact_md,
        "customer_id path missing:\n" + impact_md,
    )
    expect("assert_amount_is_dollars" in impact_md, "singular test missing from impact")
    expect("not_null" in impact_md and "amount" in impact_md, "amount test missing from impact")
    expect("order_total / 100.0" in patch, "patch missing cents conversion")
    expect("/ 1000" in patch, "patch missing millisecond conversion")
    expect("epoch(try_cast(ordered_at as timestamptz))" in patch, "patch missing timestamp cast")
    expect("required: false" in patch and "order_total" in patch, "contract patch incomplete")
    expect("Nothing was applied or merged" in pr_md, "PR text does not say it is a draft")
    by_id = {row["order_id"]: row for row in normalized}
    expect(by_id["ord_2001"]["amount"] == 49.99, by_id["ord_2001"])
    expect(by_id["ord_2001"]["ordered_at"] == 1731369600, by_id["ord_2001"])
    expect("customer_id" not in by_id["ord_2001"], by_id["ord_2001"])
    expect(by_id["ord_2003"]["amount"] == 15.5, by_id["ord_2003"])
    expect(by_id["ord_2003"]["ordered_at"] == 1731456000, by_id["ord_2003"])
    expect(by_id["ord_2004"]["amount"] == 250, by_id["ord_2004"])
    expect(by_id["ord_2005"]["ordered_at"] == 1731628800, by_id["ord_2005"])
    staged_by_index = {row["record"]: row for row in staged["records"]}
    for index, record in enumerate(records):
        row = staged_by_index[index]
        if order_id(record) in {"ord_2001", "ord_2002"}:
            kinds = {(item["field"], item["kind"]) for item in row["violations"]}
            expect(kinds == {("customer_id", "missing_required_field")}, f"{order_id(record)} {kinds}")
        else:
            expect(row["status"] == "pass", f"staged {order_id(record)} {row}")
    proposed_by_index = {row["record"]: row for row in proposed["records"]}
    expect(proposed_by_index[0]["status"] == "pass", "control record should pass the draft contract")
    for index in (1, 2, 5):
        fields = {item["field"] for item in proposed_by_index[index]["violations"]}
        expect(fields == {"ordered_at"}, f"draft contract {order_id(records[index])} {fields}")
    expect(
        any(item["kind"] == "range_violation" and item["field"] == "amount" for item in proposed_by_index[3]["violations"]),
        proposed_by_index[3],
    )
    expect(
        any(item["kind"] == "range_violation" and item["field"] == "ordered_at" for item in proposed_by_index[3]["violations"]),
        proposed_by_index[3],
    )


def expect(condition: bool, detail: object) -> None:
    if not condition:
        raise SystemExit(f"demo check failed: {detail}")


def emit(title: str, body: str) -> None:
    print()
    print("=" * 72)
    print(title)
    print("=" * 72)
    print(body.rstrip())
    print()


if __name__ == "__main__":
    main()
