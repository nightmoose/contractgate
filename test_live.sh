#!/usr/bin/env bash
# ============================================================
# ContractGate — Live API smoke test + data seeder
# Usage: bash test_live.sh [BASE_URL]
# Default: https://contractgate-api.fly.dev
# ============================================================

set -euo pipefail

BASE="${1:-https://contractgate-api.fly.dev}"
PASS=0; FAIL=0

green()  { echo -e "\033[32m✔ $*\033[0m"; }
red()    { echo -e "\033[31m✘ $*\033[0m"; }
yellow() { echo -e "\033[33m» $*\033[0m"; }
header() { echo -e "\n\033[1;34m══ $* ══\033[0m"; }

check() {
  local label="$1" got="$2" want="$3"
  if echo "$got" | grep -q "$want"; then
    green "$label"
    ((PASS++)) || true
  else
    red "$label  (wanted: '$want'  got: '$got')"
    ((FAIL++)) || true
  fi
}

# ── 1. Health ────────────────────────────────────────────────
header "Health Check"
HEALTH=$(curl -sf "$BASE/health")
check "GET /health → ok" "$HEALTH" '"status":"ok"'
echo "   $HEALTH"

# ── 2. Create a contract ────────────────────────────────────
header "Create Contract"
CONTRACT_YAML='version: "1.0"
name: "user_events_test"
description: "Smoke-test contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]+$"
    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
    - name: timestamp
      type: integer
      required: true
      min: 0
    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: amount
    description: "Monetary amount in USD"
    constraints: "must be non-negative"

metrics:
  - name: total_revenue
    formula: "sum(amount) where event_type = '"'"'purchase'"'"'"
'

CREATE_RESP=$(curl -sf -X POST "$BASE/contracts" \
  -H "Content-Type: application/json" \
  -d "{\"yaml_content\": $(echo "$CONTRACT_YAML" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')}")

check "POST /contracts → created" "$CREATE_RESP" '"name":"user_events_test"'
CONTRACT_ID=$(echo "$CREATE_RESP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
yellow "Contract ID: $CONTRACT_ID"

# ── 3. List contracts ────────────────────────────────────────
header "List Contracts"
LIST_RESP=$(curl -sf "$BASE/contracts")
check "GET /contracts → contains our contract" "$LIST_RESP" "user_events_test"

# ── 4. Ingest VALID events ───────────────────────────────────
header "Ingest Valid Events"

ingest() {
  curl -sf -X POST "$BASE/ingest/$CONTRACT_ID" \
    -H "Content-Type: application/json" \
    -d "$1"
}

R1=$(ingest '[{"user_id":"alice_01","event_type":"click","timestamp":1712000001}]')
check "Valid click event → passed" "$R1" '"passed":1'

R2=$(ingest '[{"user_id":"bob-99","event_type":"purchase","timestamp":1712000002,"amount":49.99}]')
check "Valid purchase event → passed" "$R2" '"passed":1'

R3=$(ingest '[{"user_id":"carol_x","event_type":"login","timestamp":1712000003},{"user_id":"dave_7","event_type":"view","timestamp":1712000004}]')
check "Batch of 2 valid events → passed" "$R3" '"passed":2'

# ── 5. Ingest INVALID events (should fail validation) ────────
header "Ingest Invalid Events (Expect Violations)"

R4=$(ingest '[{"event_type":"click","timestamp":1712000005}]')
check "Missing user_id → violation" "$R4" '"failed":1'

R5=$(ingest '[{"user_id":"eve_1","event_type":"explode","timestamp":1712000006}]')
check "Bad enum event_type → violation" "$R5" '"failed":1'

R6=$(ingest '[{"user_id":"!invalid!","event_type":"view","timestamp":1712000007}]')
check "Bad pattern user_id → violation" "$R6" '"failed":1'

R7=$(ingest '[{"user_id":"frank_2","event_type":"purchase","timestamp":1712000008,"amount":-5}]')
check "Negative amount → violation" "$R7" '"failed":1'

# ── 6. Mixed batch ───────────────────────────────────────────
header "Mixed Batch (2 valid, 2 invalid)"
R8=$(ingest '[
  {"user_id":"grace_1","event_type":"click","timestamp":1712000010},
  {"user_id":"","event_type":"purchase","timestamp":1712000011},
  {"user_id":"henry_2","event_type":"login","timestamp":1712000012},
  {"user_id":"ivan_3","event_type":"badtype","timestamp":1712000013}
]')
check "Mixed batch → 2 passed 2 failed" "$R8" '"passed":2'

# ── 7. Dry run ───────────────────────────────────────────────
header "Dry Run (no DB writes)"
R9=$(curl -sf -X POST "$BASE/ingest/$CONTRACT_ID?dry_run=true" \
  -H "Content-Type: application/json" \
  -d '[{"user_id":"dryrun_user","event_type":"view","timestamp":1712000020}]')
check "Dry run → dry_run:true in response" "$R9" '"dry_run":true'

# ── 8. Playground validate (no DB) ───────────────────────────
header "Playground Validate Endpoint"
PG=$(curl -sf -X POST "$BASE/playground/validate" \
  -H "Content-Type: application/json" \
  -d "{\"yaml_content\": $(echo "$CONTRACT_YAML" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'), \"event\": {\"user_id\":\"test_99\",\"event_type\":\"click\",\"timestamp\":1712000099}}")
check "Playground validate → passed" "$PG" '"passed":true'

# ── 9. Audit log ─────────────────────────────────────────────
header "Audit Log"
AUDIT=$(curl -sf "$BASE/audit?contract_id=$CONTRACT_ID&limit=20")
check "GET /audit → has entries" "$AUDIT" '"contract_id"'

# ── 10. Stats ────────────────────────────────────────────────
header "Stats"
STATS=$(curl -sf "$BASE/ingest/$CONTRACT_ID/stats")
check "GET /ingest/:id/stats → has total_events" "$STATS" '"total_events"'
echo "   $STATS"

# ── Summary ──────────────────────────────────────────────────
echo ""
echo -e "\033[1m══ Results: \033[32m$PASS passed\033[0m\033[1m  \033[31m$FAIL failed\033[0m\033[1m ══\033[0m"
if [ "$FAIL" -eq 0 ]; then
  green "All tests passed! ContractGate is live and working."
else
  red "$FAIL test(s) failed. Check output above."
  exit 1
fi
