#!/usr/bin/env bash
# hero_demo.sh — ContractGate 15-minute hero demo (RFC-081 quarantine→replay).
#
# Tells the whole value story over plain HTTP:
#   1. Deploy a STRICT contract (v1.0.0) and a RELAXED one (v1.1.0).
#   2. Good events pass.
#   3. A producer still on the old rules (CONNECT method) is validated against
#      v1.0.0 → QUARANTINED before it hits the warehouse.
#   4. Inspect the quarantined events + their violations.
#   5. REPLAY them against v1.1.0 → they pass. Backlog drained, nothing lost.
#
# Why two versions up front: ContractGate blocks deploying a new version while
# events are quarantined (a safety feature). So the real workflow is to register
# the corrected version, then replay the backlog against it — which is exactly
# what this shows.
#
# Each run uses a fresh, timestamped contract name, so it's safe to re-run and
# never collides with existing contracts.
#
# Requires: curl, jq. Points at any running gateway.
#
# Usage:
#   # Hosted / keyed gateway (service-role key needed for deploy):
#   KEY=cg_live_xxx HOST=https://contractgate-api.fly.dev bash scripts/hero_demo.sh
#
#   # Local `make demo` stack (dev-no-auth, no key — pass the demo org id):
#   ORG_ID=cccccccc-cccc-cccc-cccc-cccccccccccc HOST=http://localhost:8080 \
#       bash scripts/hero_demo.sh
#
# Exit: 0 on the full loop passing, 1 on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIX="$ROOT/demo/hero"

HOST="${HOST:-http://localhost:8080}"
NAME="hero_events_$(date +%s)"   # fresh per run → re-runnable, collision-free

# ── auth: service-role key (hosted) OR x-org-id (local dev-no-auth) ──────────
if [[ -n "${KEY:-}" ]]; then
    AUTH=(-H "x-api-key: $KEY")
elif [[ -n "${ORG_ID:-}" ]]; then
    AUTH=(-H "x-org-id: $ORG_ID")
else
    echo "ERROR: set KEY=<service-role key> (hosted) or ORG_ID=<uuid> (local dev-no-auth)." >&2
    exit 1
fi
JSON=(-H "Content-Type: application/json")

green() { printf '\e[32m%s\e[0m\n' "$1"; }
red()   { printf '\e[31m%s\e[0m\n' "$1"; }
bold()  { printf '\n\e[1m%s\e[0m\n' "$1"; }

command -v jq >/dev/null || { red "jq is required"; exit 1; }

# POST helper: prints body, exits on unexpected status. Args: url datafile [expected]
post_file() {
    local url="$1" file="$2" expect="${3:-201}"
    local resp status json
    resp=$(curl -s -w $'\n%{http_code}' -X POST "$url" "${AUTH[@]}" "${JSON[@]}" --data-binary @"$file")
    status=$(printf '%s' "$resp" | tail -n1)
    json=$(printf '%s' "$resp" | sed '$d')
    if [[ "$status" != "$expect" ]]; then
        red "  ✗ $url → HTTP $status (expected $expect)"; echo "$json" | jq . 2>/dev/null || echo "$json"; exit 1
    fi
    printf '%s' "$json"
}

# ── 1. Deploy both contract versions (before any quarantine exists) ─────────
bold "[1/5] Deploying hero_events v1.0.0 (strict) + v1.1.0 (relaxed) as '$NAME'"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
for v in 1.0.0 1.1.0; do
    # Rename the contract in the YAML to the unique per-run name.
    sed "s/\"hero_events\"/\"$NAME\"/" "$FIX/contract_$v.yaml" > "$tmp/c_$v.yaml"
    body=$(jq -n --arg n "$NAME" --arg y "$(cat "$tmp/c_$v.yaml")" \
        '{name:$n, yaml_content:$y, source:"hero-demo", deployed_by:"hero_demo.sh"}')
    echo "$body" > "$tmp/deploy_$v.json"
    resp=$(post_file "$HOST/contracts/deploy" "$tmp/deploy_$v.json" 201)
    green "  ✓ deployed v$v ($(echo "$resp" | jq -r '.version'))"
    [[ "$v" == "1.0.0" ]] && CID=$(echo "$resp" | jq -r '.contract_id')
done
echo "  contract_id: $CID"

# ── 2. Good events pass (validated against the latest stable, v1.1.0) ───────
bold "[2/5] Ingesting ${FIX##*/}/events_pass.json (expect all pass)"
resp=$(post_file "$HOST/ingest/$CID" "$FIX/events_pass.json" 200)
PASSED=$(echo "$resp" | jq -r '.passed // .total // "?"')
green "  ✓ $PASSED events passed"

# ── 3. Legacy CONNECT traffic, pinned to strict v1.0.0 → quarantined ────────
bold "[3/5] Ingesting events_quarantine.json PINNED to v1.0.0 (expect quarantine)"
# @1.0.0 forces validation against the strict version even though 1.1.0 is stable.
post_file "$HOST/ingest/$CID@1.0.0" "$FIX/events_quarantine.json" 200 >/dev/null
green "  ✓ ingest accepted (validated against v1.0.0)"

# ── 4. Inspect the quarantine (RFC-081 GET /quarantine) ─────────────────────
bold "[4/5] Listing quarantined events for this contract"
Q=$(curl -s "${AUTH[@]}" "$HOST/quarantine?contract_id=$CID&limit=100")
QCOUNT=$(printf '%s' "$Q" | jq 'length')
if [[ "$QCOUNT" -lt 1 ]]; then
    red "  ✗ expected quarantined events, found $QCOUNT"; printf '%s' "$Q" | jq . 2>/dev/null; exit 1
fi
green "  ✓ $QCOUNT events quarantined"
echo "  first event's violations:"
printf '%s' "$Q" | jq -c '.[0].violation_details'
IDS=$(printf '%s' "$Q" | jq '[.[].id]')

# ── 5. Replay against v1.1.0 (RFC-081 POST /quarantine/replay) → pass ───────
bold "[5/5] Replaying the backlog against v1.1.0 (expect all pass)"
jq -n --argjson ids "$IDS" --arg v "1.1.0" --arg c "$CID" \
    '{event_ids:$ids, version:$v, contract_id:$c}' > "$tmp/replay.json"
RESP=$(post_file "$HOST/quarantine/replay" "$tmp/replay.json" 200)
REPLAYED=$(echo "$RESP" | jq -r '.replayed')
if [[ "$REPLAYED" != "$QCOUNT" ]]; then
    red "  ✗ replayed $REPLAYED / $QCOUNT"; echo "$RESP" | jq '.outcomes'; exit 1
fi
green "  ✓ $REPLAYED/$QCOUNT events replayed and PASSED against v1.1.0"

bold "Hero demo complete — bad events were stopped at ingest, then drained clean via replay. ✓"
