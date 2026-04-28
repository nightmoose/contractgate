#!/usr/bin/env bash
# compose_smoke.sh — RFC-017 CI smoke test (default profile, no demo seeder).
#
# Spins up the ContractGate stack, waits for the gateway to be healthy,
# posts a contract + validates an event, then tears down.
#
# Requires: docker compose v2, curl, jq
# Usage: bash tests/compose_smoke.sh
# Exit: 0 on pass, 1 on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TAG="${TAG:-ci}"

cleanup() {
    echo "--- teardown ---"
    docker compose -f "$ROOT/docker-compose.yml" down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

echo "=== compose_smoke: building gateway image (tag=$TAG) ==="
docker build -t "ghcr.io/contractgate/gateway:$TAG" "$ROOT"

echo "=== compose_smoke: starting stack ==="
TAG="$TAG" docker compose -f "$ROOT/docker-compose.yml" up -d postgres gateway prometheus grafana

echo "=== compose_smoke: waiting for gateway /health ==="
max_attempts=30
attempt=0
until curl -sf "http://localhost:8080/health" > /dev/null; do
    attempt=$((attempt + 1))
    if [[ $attempt -ge $max_attempts ]]; then
        echo "ERROR: gateway did not become healthy after ${max_attempts} attempts"
        docker compose -f "$ROOT/docker-compose.yml" logs gateway
        exit 1
    fi
    echo "  ... waiting ($attempt/${max_attempts})"
    sleep 2
done
echo "  gateway healthy ✓"

echo "=== compose_smoke: posting starter contract ==="
CREATE_RESP=$(curl -sf -X POST "http://localhost:8080/contracts" \
    -H "Content-Type: application/json" \
    -d '{
        "name": "smoke_test_contract",
        "yaml_content": "version: \"1.0\"\nname: smoke_test\ndescription: \"CI smoke test\"\nontology:\n  entities:\n    - name: id\n      type: string\n      required: true\n    - name: ts\n      type: integer\n      required: true\n"
    }')
CONTRACT_ID=$(echo "$CREATE_RESP" | jq -r '.id')
echo "  created contract id=$CONTRACT_ID ✓"

echo "=== compose_smoke: promoting contract to stable ==="
curl -sf -X POST "http://localhost:8080/contracts/$CONTRACT_ID/versions/1.0.0/promote" > /dev/null
echo "  promoted ✓"

echo "=== compose_smoke: posting valid event — expecting pass ==="
INGEST_RESP=$(curl -sf -X POST "http://localhost:8080/ingest/$CONTRACT_ID" \
    -H "Content-Type: application/json" \
    -d '{"id": "abc123", "ts": 1700000000}')
PASSED=$(echo "$INGEST_RESP" | jq -r '.passed')
if [[ "$PASSED" != "1" ]]; then
    echo "ERROR: expected passed=1, got: $INGEST_RESP"
    exit 1
fi
echo "  event passed ✓"

echo "=== compose_smoke: posting invalid event — expecting fail ==="
FAIL_RESP=$(curl -s -X POST "http://localhost:8080/ingest/$CONTRACT_ID" \
    -H "Content-Type: application/json" \
    -d '{"ts": 1700000000}')
FAILED=$(echo "$FAIL_RESP" | jq -r '.failed')
if [[ "$FAILED" != "1" ]]; then
    echo "ERROR: expected failed=1, got: $FAIL_RESP"
    exit 1
fi
echo "  event failed as expected ✓"

echo "=== compose_smoke: PASS ==="
