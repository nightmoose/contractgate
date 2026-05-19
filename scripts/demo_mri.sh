#!/usr/bin/env bash
# demo_mri.sh — Deploy and smoke-test MRI contracts against ContractGate.
#
# Usage:
#   KEY=cg_live_... HOST=https://contractgate-api.fly.dev bash scripts/demo_mri.sh
#
# Defaults to localhost:3001 if HOST is unset.

set -euo pipefail

KEY="${KEY:-}"
HOST="${HOST:-http://localhost:3001}"

if [[ -z "$KEY" ]]; then
  echo "ERROR: set KEY=<your-api-key>" >&2
  exit 1
fi

H=(-H "x-api-key: $KEY" -H "Content-Type: application/json")

green() { printf '\e[32m%s\e[0m\n' "$1"; }
red()   { printf '\e[31m%s\e[0m\n' "$1"; }
bold()  { printf '\e[1m%s\e[0m\n'  "$1"; }

# ── 1. Deploy contracts ───────────────────────────────────────────────────────

deploy() {
  local name="$1" yaml="$2"
  bold "Deploying $name ..."
  local body
  body=$(jq -n --arg n "$name" --arg y "$yaml" \
    '{name: $n, yaml_content: $y, source: "mri", deployed_by: "demo_mri.sh"}')
  local resp
  resp=$(curl -s -w '\n__STATUS__%{http_code}' \
    -X POST "$HOST/contracts/deploy" "${H[@]}" -d "$body")
  local status="${resp##*__STATUS__}"
  local json="${resp%__STATUS__*}"
  if [[ "$status" == "201" ]]; then
    green "  ✓ deployed ($(echo "$json" | jq -r '.version_id'))"
  elif [[ "$status" == "409" ]]; then
    green "  ✓ already deployed (v1.0 exists — skipping)"
  else
    red "  ✗ deploy failed ($status)"
    echo "$json" | jq . 2>/dev/null || echo "$json"
    exit 1
  fi
}

PROPERTY_YAML=$(cat <<'YAML'
version: "1.0"
name: "mri_property_listing"
description: "Contract for MRI MIX API property unit listing responses (Findigs integration)."

envelope:
  records_path: data
  validate_wrapper: true

ontology:
  entities:
    - name: unit_id
      type: string
      required: true
      pattern: "^[A-Za-z0-9_-]+$"
    - name: property_id
      type: string
      required: true
    - name: unit_number
      type: string
      required: true
    - name: bedrooms
      type: integer
      required: true
      min: 0
      max: 20
    - name: bathrooms
      type: number
      required: true
      min: 0
    - name: rent_amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]
    - name: status
      type: string
      required: true
      enum: ["available", "occupied", "maintenance", "reserved"]
    - name: floor_area_sqft
      type: number
      required: false
      min: 0
    - name: available_from
      type: string
      required: false
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"
YAML
)

TENANCY_YAML=$(cat <<'YAML'
version: "1.0"
name: "mri_tenancy_event"
description: "Contract for MRI MIX API tenancy records (Findigs integration)."

envelope:
  records_path: data
  validate_wrapper: true

ontology:
  entities:
    - name: tenancy_id
      type: string
      required: true
      pattern: "^[A-Za-z0-9_-]+$"
    - name: unit_id
      type: string
      required: true
    - name: property_id
      type: string
      required: true
    - name: tenant_contact_id
      type: string
      required: true
    - name: start_date
      type: string
      required: true
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"
    - name: end_date
      type: string
      required: false
      pattern: "^\\d{4}-\\d{2}-\\d{2}$"
    - name: rent_amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      enum: ["USD", "EUR", "GBP", "CAD", "AUD", "MXN"]
    - name: status
      type: string
      required: true
      enum: ["active", "pending", "ended", "terminated"]
    - name: deposit_amount
      type: number
      required: false
      min: 0
    - name: payment_frequency
      type: string
      required: false
      enum: ["weekly", "monthly", "quarterly", "annually"]
YAML
)

deploy "mri_property_listing" "$PROPERTY_YAML"
deploy "mri_tenancy_event"    "$TENANCY_YAML"

# ── 2. Get contract IDs ───────────────────────────────────────────────────────

bold "Fetching contract list ..."
CONTRACTS=$(curl -s "$HOST/contracts" "${H[@]}")
PROP_ID=$(echo "$CONTRACTS" | jq -r '.[] | select(.name=="mri_property_listing") | .id')
TEN_ID=$(echo  "$CONTRACTS" | jq -r '.[] | select(.name=="mri_tenancy_event")    | .id')

echo "  mri_property_listing → $PROP_ID"
echo "  mri_tenancy_event    → $TEN_ID"

# ── 3. Validate: valid MRI property listing envelope ─────────────────────────

bold "[1/4] Valid property listing (all 3 units pass) ..."
VALID_PROP=$(jq -n '{
  "success": true,
  "data": [
    {"unit_id":"U-101","property_id":"P-001","unit_number":"101",
     "bedrooms":2,"bathrooms":1.5,"rent_amount":2200,"currency":"USD","status":"available"},
    {"unit_id":"U-102","property_id":"P-001","unit_number":"102",
     "bedrooms":1,"bathrooms":1,"rent_amount":1800,"currency":"USD","status":"occupied"},
    {"unit_id":"U-103","property_id":"P-001","unit_number":"103",
     "bedrooms":3,"bathrooms":2,"rent_amount":3100,"currency":"CAD","status":"reserved",
     "floor_area_sqft":1200,"available_from":"2025-07-01"}
  ],
  "pagination": {"page":1,"limit":10,"total":3,"hasMore":false}
}')
RESP=$(curl -s -X POST "$HOST/ingest/$PROP_ID" "${H[@]}" -d "$VALID_PROP")
PASSED=$(echo "$RESP" | jq -r '.passed // empty')
if [[ "$PASSED" == "3" ]]; then
  green "  ✓ 3 passed, 0 quarantined"
else
  red "  ✗ unexpected response:"
  echo "$RESP" | jq .
fi

# ── 4. Validate: bad enum in one record ───────────────────────────────────────

bold "[2/4] Invalid property listing (bad currency on record 1) ..."
BAD_PROP=$(jq -n '{
  "success": true,
  "data": [
    {"unit_id":"U-201","property_id":"P-002","unit_number":"201",
     "bedrooms":2,"bathrooms":1,"rent_amount":2500,"currency":"XYZ","status":"available"}
  ],
  "pagination": {"page":1,"limit":10,"total":1,"hasMore":false}
}')
RESP=$(curl -s -X POST "$HOST/ingest/$PROP_ID" "${H[@]}" -d "$BAD_PROP")
Q=$(echo "$RESP" | jq -r '.quarantined // empty')
if [[ "$Q" == "1" ]]; then
  green "  ✓ 0 passed, 1 quarantined (invalid currency 'XYZ')"
  echo "$RESP" | jq '.violations[]'
else
  red "  ✗ unexpected response:"
  echo "$RESP" | jq .
fi

# ── 5. Validate: valid tenancy event ─────────────────────────────────────────

bold "[3/4] Valid tenancy event ..."
VALID_TEN=$(jq -n '{
  "success": true,
  "data": [
    {"tenancy_id":"T-5001","unit_id":"U-101","property_id":"P-001",
     "tenant_contact_id":"C-9001","start_date":"2024-01-15","end_date":"2025-01-14",
     "rent_amount":2200,"currency":"USD","status":"active",
     "deposit_amount":4400,"payment_frequency":"monthly"}
  ],
  "pagination": {"page":1,"limit":10,"total":1,"hasMore":false}
}')
RESP=$(curl -s -X POST "$HOST/ingest/$TEN_ID" "${H[@]}" -d "$VALID_TEN")
PASSED=$(echo "$RESP" | jq -r '.passed // empty')
if [[ "$PASSED" == "1" ]]; then
  green "  ✓ 1 passed, 0 quarantined"
else
  red "  ✗ unexpected response:"
  echo "$RESP" | jq .
fi

# ── 6. Validate: missing required field in tenancy ────────────────────────────

bold "[4/4] Invalid tenancy (missing tenant_contact_id) ..."
BAD_TEN=$(jq -n '{
  "success": true,
  "data": [
    {"tenancy_id":"T-5002","unit_id":"U-102","property_id":"P-001",
     "start_date":"2024-06-01","rent_amount":1800,"currency":"USD","status":"pending"}
  ],
  "pagination": {"page":1,"limit":10,"total":1,"hasMore":false}
}')
RESP=$(curl -s -X POST "$HOST/ingest/$TEN_ID" "${H[@]}" -d "$BAD_TEN")
Q=$(echo "$RESP" | jq -r '.quarantined // empty')
if [[ "$Q" == "1" ]]; then
  green "  ✓ 0 passed, 1 quarantined (missing tenant_contact_id)"
  echo "$RESP" | jq '.violations[]'
else
  red "  ✗ unexpected response:"
  echo "$RESP" | jq .
fi

bold "Demo complete."
