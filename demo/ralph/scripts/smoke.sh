#!/usr/bin/env bash
# RFC-092 Stage A smoke: broker up → produce → bridge → quarantine gets bad events
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export KAFKA_BOOTSTRAP="${KAFKA_BOOTSTRAP:-127.0.0.1:9092}"

if [[ ! -d .venv ]]; then
  python3 -m venv .venv
  .venv/bin/pip install -q -r requirements.txt
fi
# shellcheck disable=SC1091
source .venv/bin/activate

echo "[smoke] compose up"
docker compose up -d
echo "[smoke] waiting for Redpanda..."
for i in $(seq 1 40); do
  if docker exec cg-ralph-redpanda rpk cluster health 2>/dev/null | grep -q 'Healthy:.*true'; then
    break
  fi
  sleep 1
done

echo "[smoke] ensure topics (producers helper)"
python -c "from produce.producers import ensure_topics; import os; ensure_topics(os.environ.get('KAFKA_BOOTSTRAP','127.0.0.1:9092'))"

echo "[smoke] start bridge"
python bridge/gate_bridge.py > /tmp/cg-ralph-bridge.log 2>&1 &
BRIDGE_PID=$!
cleanup() {
  kill "$BRIDGE_PID" 2>/dev/null || true
  kill "$PROD_PID" 2>/dev/null || true
}
trap cleanup EXIT
sleep 3
if ! kill -0 "$BRIDGE_PID" 2>/dev/null; then
  echo "[smoke] FAIL: bridge exited early"
  tail -50 /tmp/cg-ralph-bridge.log || true
  exit 1
fi

echo "[smoke] start producers briefly"
python produce/producers.py > /tmp/cg-ralph-prod.log 2>&1 &
PROD_PID=$!
sleep 8
kill "$PROD_PID" 2>/dev/null || true
wait "$PROD_PID" 2>/dev/null || true
PROD_PID=

echo "[smoke] inject bad"
python produce/inject_bad.py
sleep 5

echo "[smoke] check quarantine topics via rpk"
Q=$(docker exec cg-ralph-redpanda timeout 15 rpk topic consume orders.quarantine -n 1 -o start --format '%v\n' 2>/dev/null | head -1 || true)
if [[ -z "$Q" ]]; then
  echo "[smoke] FAIL: no message on orders.quarantine"
  echo "--- bridge log ---"
  tail -80 /tmp/cg-ralph-bridge.log || true
  exit 1
fi
echo "[smoke] OK quarantine sample: ${Q:0:120}..."
echo "[smoke] PASS"
