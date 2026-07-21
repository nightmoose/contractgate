#!/usr/bin/env bash
# One full local dogfood iteration (fetch → author → validate).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
source .venv/bin/activate
SCENARIO="${1:-all}"
echo "== fetch =="
python scripts/fetch_sources.py --scenario "$SCENARIO"
echo "== author =="
python scripts/author_contract.py --scenario "$SCENARIO"
echo "== local validate =="
python scripts/run_local.py --scenario "$SCENARIO"
echo "Done. For cloud: CG_API_KEY=… python scripts/run_cloud.py --scenario usgs_earthquake"
