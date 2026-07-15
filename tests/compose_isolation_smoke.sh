#!/usr/bin/env bash
# compose_isolation_smoke.sh — RFC-075 auth-on cross-tenant isolation lane.
#
# Proves tenant isolation END-TO-END against a gateway with auth ON, which the
# demo/compose stack (CONTRACTGATE_DEV_NO_AUTH=1) structurally cannot do. See
# docs/rfcs/075-auth-on-isolation-test-lane.md for why RFC-073's compose-smoke
# wiring was a false green.
#
# Three assertions, in order:
#   0. SANITY GATE — a request with NO api key returns 401. If it returns 200,
#      the instance is misconfigured (auth actually off) and we fail LOUDLY
#      rather than pass a meaningless isolation check. This is the guard whose
#      absence let RFC-073 mislead us.
#   1. ISOLATION   — org B's key ingesting into org A's contract → 403/404.
#   2. POSITIVE    — org A's key ingesting into org A's contract → 2xx. Without
#      this, "everything is rejected" would also pass assertion 1; the positive
#      path proves auth-on accepts the RIGHT key, so the rejection is meaningful.
#
# Fixtures come from ops/postgres/seed/098_isolation_test.sql (applied at
# Postgres initdb). Raw keys below match the committed key_hash values there;
# they grant access to nothing but this throwaway compose database.
#
# Requires: docker compose v2, curl
# Usage: bash tests/compose_isolation_smoke.sh
# Exit: 0 on pass, 1 on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TAG="${TAG:-ci}"

BASE="http://localhost:8081"
KEY_A="cg_live_orgA_isolationtest_000000000001"
KEY_B="cg_live_orgB_isolationtest_000000000002"
CONTRACT_A="a0000000-0000-0000-0000-000000000001"
EVENT='[{"user_id":"u1","event_type":"login","timestamp":1714000000}]'

COMPOSE=(docker compose -f "$ROOT/docker-compose.yml" -f "$ROOT/tests/compose.isolation.yml")

cleanup() {
    echo "--- teardown ---"
    "${COMPOSE[@]}" down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

echo "=== isolation_smoke: building gateway image (tag=$TAG) ==="
docker build -t "ghcr.io/contractgate/gateway:$TAG" "$ROOT"

echo "=== isolation_smoke: starting postgres + auth-on gateway ==="
TAG="$TAG" "${COMPOSE[@]}" up -d postgres gateway-authon

echo "=== isolation_smoke: waiting for auth-on gateway /ready ==="
max_attempts=30
attempt=0
until curl -sf "$BASE/ready" > /dev/null; do
    attempt=$((attempt + 1))
    if [[ $attempt -ge $max_attempts ]]; then
        echo "ERROR: auth-on gateway did not become ready"
        "${COMPOSE[@]}" logs gateway-authon
        exit 1
    fi
    echo "  ... waiting ($attempt/${max_attempts})"
    sleep 2
done
echo "  auth-on gateway ready ✓"

# helper: POST the event with an optional api key, echo the HTTP status code.
post_status() {
    local key="$1"
    if [[ -n "$key" ]]; then
        curl -s -o /dev/null -w '%{http_code}' \
            -X POST "$BASE/v1/ingest/$CONTRACT_A" \
            -H "x-api-key: $key" \
            -H "content-type: application/json" \
            -d "$EVENT"
    else
        curl -s -o /dev/null -w '%{http_code}' \
            -X POST "$BASE/v1/ingest/$CONTRACT_A" \
            -H "content-type: application/json" \
            -d "$EVENT"
    fi
}

# ── 0. Sanity gate: no key → 401 (auth is actually ON) ──────────────────────
echo "=== isolation_smoke: [0] sanity gate — no key must 401 ==="
NOKEY_STATUS=$(post_status "")
if [[ "$NOKEY_STATUS" != "401" ]]; then
    echo "ERROR: no-key request returned $NOKEY_STATUS, expected 401."
    echo "The gateway-authon instance is NOT enforcing auth — the isolation"
    echo "assertion below would be meaningless. Failing loudly (RFC-075 guard)."
    "${COMPOSE[@]}" logs gateway-authon --tail=50
    exit 1
fi
echo "  no-key → 401 ✓ (auth confirmed on)"

# ── 1. Isolation: org B key → org A contract → 403/404 ──────────────────────
echo "=== isolation_smoke: [1] cross-org ingest must be rejected ==="
XORG_STATUS=$(post_status "$KEY_B")
if [[ "$XORG_STATUS" != "403" && "$XORG_STATUS" != "404" ]]; then
    echo "ERROR: org B key → org A contract returned $XORG_STATUS,"
    echo "expected 403 or 404. Cross-tenant isolation is BROKEN."
    "${COMPOSE[@]}" logs gateway-authon --tail=50
    exit 1
fi
echo "  org B → org A contract → $XORG_STATUS ✓ (rejected)"

# ── 2. Positive: org A key → org A contract → 2xx ───────────────────────────
echo "=== isolation_smoke: [2] correct key must be accepted ==="
OWN_STATUS=$(post_status "$KEY_A")
if [[ "$OWN_STATUS" -lt 200 || "$OWN_STATUS" -ge 300 ]]; then
    echo "ERROR: org A key → org A contract returned $OWN_STATUS, expected 2xx."
    echo "Auth-on gateway is rejecting the CORRECT key — assertion [1] would"
    echo "pass for the wrong reason (everything rejected). Failing."
    "${COMPOSE[@]}" logs gateway-authon --tail=50
    exit 1
fi
echo "  org A → org A contract → $OWN_STATUS ✓ (accepted)"

echo "=== isolation_smoke: PASS ==="
