#!/usr/bin/env bash
# compose_demo_smoke.sh — RFC-017 CI smoke test (demo profile with seeder).
#
# Spins up the full stack including the demo-seeder service, waits for the
# seeder to exit (it's one-shot), then asserts that:
#   1. The gateway /health is still up.
#   2. The audit_log has > 1000 rows (seeder posted ~3000 events at 10/sec
#      for 5m; CI uses a shorter run — see SEEDER_DURATION below).
#   3. GET /contracts returns the three starter contracts.
#
# In CI we run the seeder for 30s at 50/sec (~1500 events) to keep wall-clock
# time short.  Override via SEEDER_RATE and SEEDER_DURATION env vars.
#
# Requires: docker compose v2, curl, jq
# Usage: bash tests/compose_demo_smoke.sh
# Exit: 0 on pass, 1 on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TAG="${TAG:-ci}"
SEEDER_RATE="${SEEDER_RATE:-50}"
SEEDER_DURATION="${SEEDER_DURATION:-30s}"
MIN_AUDIT_ROWS="${MIN_AUDIT_ROWS:-500}"
# Same fixed-UUID demo org used by the seeder + the postgres init seed.
# Required because /contracts and /audit are org-scoped — without it
# the post-seed assertions would query a different (empty) scope.
DEMO_ORG_ID="${DEMO_ORG_ID:-cccccccc-cccc-cccc-cccc-cccccccccccc}"

cleanup() {
    echo "--- teardown ---"
    docker compose -f "$ROOT/docker-compose.yml" --profile demo down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

echo "=== compose_demo_smoke: building gateway image (tag=$TAG) ==="
docker build -t "ghcr.io/contractgate/gateway:$TAG" "$ROOT"

echo "=== compose_demo_smoke: starting stack + demo profile ==="
# Override seeder flags via the command in the Compose service is not directly
# supported, so we set env vars that the seeder binary reads.
TAG="$TAG" \
SEEDER_RATE="$SEEDER_RATE" \
SEEDER_DURATION="$SEEDER_DURATION" \
docker compose -f "$ROOT/docker-compose.yml" --profile demo up -d

echo "=== compose_demo_smoke: waiting for gateway /health ==="
max_attempts=30
attempt=0
until curl -sf "http://localhost:8080/health" > /dev/null; do
    attempt=$((attempt + 1))
    if [[ $attempt -ge $max_attempts ]]; then
        echo "ERROR: gateway did not become healthy"
        docker compose -f "$ROOT/docker-compose.yml" logs gateway
        exit 1
    fi
    echo "  ... waiting ($attempt/${max_attempts})"
    sleep 2
done
echo "  gateway healthy ✓"

echo "=== compose_demo_smoke: waiting for demo-seeder to exit ==="
# Give the seeder up to (SEEDER_DURATION + 60s) to finish.
duration_secs="${SEEDER_DURATION%s}"
duration_secs="${duration_secs%m}"  # strip trailing m if present
seeder_timeout=$(( ${duration_secs:-30} + 60 ))
SEEDER_CONTAINER="cg-demo-seeder"  # matches container_name in docker-compose.yml
seeder_exit=1
for i in $(seq 1 "$seeder_timeout"); do
    status=$(docker inspect --format='{{.State.Status}}' "$SEEDER_CONTAINER" 2>/dev/null || echo "")
    if [[ "$status" == "exited" ]]; then
        exit_code=$(docker inspect --format='{{.State.ExitCode}}' "$SEEDER_CONTAINER" 2>/dev/null || echo "1")
        if [[ "$exit_code" != "0" ]]; then
            echo "ERROR: demo-seeder exited with code $exit_code"
            docker compose -f "$ROOT/docker-compose.yml" --profile demo logs demo-seeder
            exit 1
        fi
        seeder_exit=0
        break
    fi
    sleep 1
done

if [[ "$seeder_exit" != "0" ]]; then
    echo "ERROR: demo-seeder did not exit within ${seeder_timeout}s"
    docker compose -f "$ROOT/docker-compose.yml" --profile demo logs demo-seeder
    exit 1
fi
echo "  demo-seeder exited cleanly ✓"

echo "=== compose_demo_smoke: checking starter contracts published ==="
CONTRACTS=$(curl -sf -H "x-org-id: $DEMO_ORG_ID" "http://localhost:8080/contracts")
for name in "rest_event" "kafka_event" "dbt_model_row"; do
    # Gateway returns either a bare array or `{"contracts": [...]}`
    # depending on version — handle both with `(.contracts // .)`.
    count=$(echo "$CONTRACTS" | jq --arg n "$name" '[(.contracts // .)[] | select(.name == $n)] | length' 2>/dev/null || echo "0")
    if [[ "$count" -lt "1" ]]; then
        echo "ERROR: contract '$name' not found in gateway"
        echo "Contracts: $CONTRACTS"
        exit 1
    fi
    echo "  contract '$name' present ✓"
done

echo "=== compose_demo_smoke: checking audit_log row count >= $MIN_AUDIT_ROWS ==="
# /audit returns a JSON array (no `total` field), so we can't count from it
# directly.  /stats returns IngestionStats { total_events, ... } scoped to
# the caller's org — perfect for this assertion.
STATS=$(curl -sf -H "x-org-id: $DEMO_ORG_ID" "http://localhost:8080/stats")
TOTAL=$(echo "$STATS" | jq -r '.total_events // 0')
if [[ "$TOTAL" -lt "$MIN_AUDIT_ROWS" ]]; then
    echo "ERROR: audit_log has $TOTAL rows, expected >= $MIN_AUDIT_ROWS"
    echo "Seeder may not have completed or rate was too low."
    echo "Stats response: $STATS"
    exit 1
fi
echo "  audit_log total_events=$TOTAL ✓"

echo "=== compose_demo_smoke: PASS ==="
