#!/usr/bin/env bash
# RFC-092 Stage A+B: broker → produce → bridge → Kafi join_print (bounded)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export KAFKA_BOOTSTRAP="${KAFKA_BOOTSTRAP:-127.0.0.1:9092}"
export PYTHONPATH="${ROOT}:${PYTHONPATH:-}"

if [[ ! -d .venv ]]; then
  python3 -m venv .venv
  .venv/bin/pip install -q -r requirements-downstream.txt
fi
# shellcheck disable=SC1091
source .venv/bin/activate

echo "[e2e] compose up"
docker compose up -d
for i in $(seq 1 40); do
  if docker exec cg-ralph-redpanda rpk cluster health 2>/dev/null | grep -q 'Healthy:.*true'; then
    break
  fi
  sleep 1
done

python -c "from produce.producers import ensure_topics; ensure_topics('${KAFKA_BOOTSTRAP}')"

echo "[e2e] bridge"
python bridge/gate_bridge.py > /tmp/cg-ralph-bridge.log 2>&1 &
BRIDGE_PID=$!
sleep 3
if ! kill -0 "$BRIDGE_PID" 2>/dev/null; then
  echo "[e2e] FAIL bridge"; tail -40 /tmp/cg-ralph-bridge.log; exit 1
fi

echo "[e2e] producers"
python produce/producers.py > /tmp/cg-ralph-prod.log 2>&1 &
PROD_PID=$!

echo "[e2e] join_print (60s — three-way Kafi join needs warmup)"
python downstream/join_print.py > /tmp/cg-ralph-join.log 2>&1 &
JOIN_PID=$!

cleanup() {
  kill "$BRIDGE_PID" "$PROD_PID" "$JOIN_PID" 2>/dev/null || true
}
trap cleanup EXIT

sleep 45
python produce/inject_bad.py
sleep 8

kill "$PROD_PID" 2>/dev/null || true
PROD_PID=
sleep 5

JOINS=$(grep -c '^\[join\]' /tmp/cg-ralph-join.log || true)
Q=$(docker exec cg-ralph-redpanda timeout 10 rpk topic consume orders.quarantine -n 1 -o start --format '%v\n' 2>/dev/null | head -1 || true)

echo "[e2e] joins_seen=${JOINS}"
echo "[e2e] quarantine_sample=${Q:0:100}"

if [[ "${JOINS}" -lt 1 ]]; then
  echo "[e2e] FAIL: no Kafi joins"
  echo "--- join log ---"; tail -80 /tmp/cg-ralph-join.log || true
  echo "--- bridge log ---"; tail -40 /tmp/cg-ralph-bridge.log || true
  exit 1
fi
if [[ -z "$Q" ]]; then
  echo "[e2e] FAIL: no quarantine"
  exit 1
fi

echo "[e2e] PASS (joins=${JOINS}, quarantine ok)"
