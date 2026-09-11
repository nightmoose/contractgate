#!/usr/bin/env bash
# RFC-092 — bombproof partner demo runner
#
# Brings up Redpanda, gate bridge, producers, Kafi join_print, injects bad
# events, inspects quarantine, optionally warms MCP + runs a query.
#
# Usage (from demo/ralph):
#   ./scripts/run_demo.sh              # join_print path (default, proven)
#   ./scripts/run_demo.sh --with-mcp   # also start MCP + client query
#   ./scripts/run_demo.sh --keep       # leave processes running at end
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export KAFKA_BOOTSTRAP="${KAFKA_BOOTSTRAP:-127.0.0.1:9092}"
export PYTHONPATH="${ROOT}${PYTHONPATH:+:$PYTHONPATH}"
export MCP_PORT="${MCP_PORT:-8000}"

WITH_MCP=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --with-mcp) WITH_MCP=1 ;;
    --keep) KEEP=1 ;;
    -h|--help)
      sed -n '1,12p' "$0"
      exit 0
      ;;
  esac
done

PIDS=()
cleanup() {
  if [[ "$KEEP" -eq 1 ]]; then
    echo "[demo] --keep: leaving bridge/producers/join/mcp running"
    echo "[demo] PIDs: ${PIDS[*]:-none}"
    return
  fi
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

echo "==> 0. venv"
if [[ ! -d .venv ]]; then
  python3 -m venv .venv
fi
# shellcheck disable=SC1091
source .venv/bin/activate
if ! python -c "import contractgate, confluent_kafka" 2>/dev/null; then
  pip install -q -r requirements.txt
fi
if [[ "$WITH_MCP" -eq 1 ]] || ! python -c "import kafi" 2>/dev/null; then
  pip install -q -r requirements-downstream.txt
fi

echo "==> 1. Redpanda"
docker compose up -d
for i in $(seq 1 60); do
  if docker exec cg-ralph-redpanda rpk cluster health 2>/dev/null | grep -q 'Healthy:.*true'; then
    break
  fi
  sleep 1
done
python -c "from produce.producers import ensure_topics; ensure_topics('${KAFKA_BOOTSTRAP}')"

echo "==> 2. ContractGate bridge"
python bridge/gate_bridge.py > /tmp/cg-ralph-demo-bridge.log 2>&1 &
PIDS+=($!)
sleep 3
if ! kill -0 "${PIDS[0]}" 2>/dev/null; then
  echo "FAIL: bridge died"; tail -40 /tmp/cg-ralph-demo-bridge.log; exit 1
fi

echo "==> 3. Producers"
python produce/producers.py > /tmp/cg-ralph-demo-prod.log 2>&1 &
PIDS+=($!)
sleep 2

echo "==> 4. Kafi join_print (warmup 50s)"
python downstream/join_print.py > /tmp/cg-ralph-demo-join.log 2>&1 &
PIDS+=($!)
for i in $(seq 1 50); do
  JOINS=$(grep -c '^\[join\]' /tmp/cg-ralph-demo-join.log 2>/dev/null || true)
  if [[ "${JOINS}" -ge 10 ]]; then
    echo "    joins warming… ${JOINS}"
    break
  fi
  sleep 1
done
JOINS=$(grep -c '^\[join\]' /tmp/cg-ralph-demo-join.log 2>/dev/null || true)
if [[ "${JOINS}" -lt 1 ]]; then
  echo "FAIL: no Kafi joins after warmup"
  tail -60 /tmp/cg-ralph-demo-join.log
  tail -40 /tmp/cg-ralph-demo-bridge.log
  exit 1
fi
echo "    joins_so_far=${JOINS}"

echo "==> 5. Inject bad events"
python produce/inject_bad.py
sleep 4

echo "==> 6. Inspect quarantine"
python scripts/inspect_topics.py --n 2

QCOUNT=$(python - <<'PY'
import os
from confluent_kafka import Consumer, TopicPartition
bootstrap=os.environ["KAFKA_BOOTSTRAP"]
c=Consumer({"bootstrap.servers":bootstrap,"group.id":"cg-qcheck","enable.auto.commit":False})
md=c.list_topics("orders.quarantine", timeout=10)
total=0
for p in md.topics["orders.quarantine"].partitions:
    low,high=c.get_watermark_offsets(TopicPartition("orders.quarantine", p), timeout=10)
    total += max(0, high-low)
c.close()
print(total)
PY
)
if [[ "${QCOUNT}" -lt 1 ]]; then
  echo "FAIL: orders.quarantine empty"
  exit 1
fi
echo "    orders.quarantine≈${QCOUNT} ✓"

if [[ "$WITH_MCP" -eq 1 ]]; then
  echo "==> 7. MCP server + query (may take a minute for embeddings)"
  # Free prior demo MCP if still bound (macOS: lsof)
  if command -v lsof >/dev/null 2>&1; then
    OLD=$(lsof -tiTCP:"${MCP_PORT}" -sTCP:LISTEN 2>/dev/null || true)
    if [[ -n "${OLD}" ]]; then
      echo "    freeing port ${MCP_PORT} (pids ${OLD})"
      # shellcheck disable=SC2086
      kill ${OLD} 2>/dev/null || true
      sleep 1
    fi
  fi
  (cd downstream && python app.py) > /tmp/cg-ralph-demo-mcp.log 2>&1 &
  MCP_PID=$!
  PIDS+=("$MCP_PID")
  # wait for uvicorn (macOS bash 3.2: no ${PIDS[-1]})
  for i in $(seq 1 60); do
    if grep -q 'Uvicorn running' /tmp/cg-ralph-demo-mcp.log 2>/dev/null; then
      break
    fi
    if ! kill -0 "$MCP_PID" 2>/dev/null; then
      echo "FAIL: MCP exited"; tail -50 /tmp/cg-ralph-demo-mcp.log; exit 1
    fi
    sleep 1
  done
  if ! grep -q 'Uvicorn running' /tmp/cg-ralph-demo-mcp.log 2>/dev/null; then
    echo "FAIL: MCP did not bind"; tail -50 /tmp/cg-ralph-demo-mcp.log; exit 1
  fi
  # wait for at least one merge insert
  for i in $(seq 1 90); do
    if grep -q 'Merge inserting' /tmp/cg-ralph-demo-mcp.log 2>/dev/null; then
      echo "    LanceDB merge observed"
      sleep 3
      break
    fi
    sleep 1
  done
  echo "    querying MCP…"
  set +e
  OUT=$(python downstream/client.py --query "delivered" 2>&1)
  RC=$?
  set -e
  echo "$OUT" | head -40
  if [[ $RC -ne 0 ]]; then
    echo "WARN: MCP client exit $RC (server may still be warming)"
  elif [[ -z "${OUT// }" ]]; then
    echo "WARN: empty MCP result — try again in ~30s: python downstream/client.py --query delivered"
  else
    echo "    MCP query returned content ✓"
  fi
fi

JOINS=$(grep -c '^\[join\]' /tmp/cg-ralph-demo-join.log 2>/dev/null || true)
echo ""
echo "============================================"
echo " DEMO PASS"
echo "   Kafi joins:     ${JOINS}"
echo "   Quarantine:     orders.quarantine≈${QCOUNT}"
echo "   Console:        http://localhost:8088"
echo "   Logs:           /tmp/cg-ralph-demo-*.log"
if [[ "$WITH_MCP" -eq 1 ]]; then
  echo "   MCP:            http://127.0.0.1:${MCP_PORT}/mcp"
fi
echo "============================================"
echo "Story: bad CDC never reaches joins / agent memory."
