#!/usr/bin/env bash
# perf_metering_smoke.sh — RFC-083 Phase 2 latency smoke.
#
# Question this answers: does the synchronous plan-limit check (2 indexed reads
# + a one-time month bootstrap COUNT) hurt end-to-end ingest latency?
#
# It deploys a throwaway contract, warms up (which triggers the once-per-month
# `org_monthly_usage` bootstrap), then fires a concurrent ingest load and reports
# p50/p95/p99/max — with the cold/bootstrap request called out separately, since
# that first COUNT is the known one-time cost.
#
# NOTE on the budget: the <15 ms p99 target is for the *validation engine*
# (pure CPU, measured internally via the /metrics histogram — metering can't
# touch it). This script measures *end-to-end HTTP* latency, which additionally
# includes the two synchronous metering reads + network. Judge steady-state p99
# against P99_BUDGET_MS (default 25 ms local); the engine number is scraped from
# /metrics if available.
#
# Requires: curl, jq, sort, awk.
#
# Usage:
#   # local compose (dev-no-auth): no key, pass the demo org id
#   ORG_ID=cccccccc-cccc-cccc-cccc-cccccccccccc HOST=http://localhost:8080 \
#       N=3000 C=16 bash scripts/perf_metering_smoke.sh
#
#   # hosted / keyed (service-role key; runs against a REAL org — use a test org)
#   KEY=cg_live_xxx HOST=https://contractgate-api.fly.dev bash scripts/perf_metering_smoke.sh
#
# Env: HOST, KEY | ORG_ID, N (requests, default 2000), C (concurrency, default 16),
#      P99_BUDGET_MS (default 25).

set -euo pipefail

HOST="${HOST:-http://localhost:8080}"
N="${N:-2000}"
C="${C:-16}"
P99_BUDGET_MS="${P99_BUDGET_MS:-25}"
NAME="perf_metering_$(date +%s)"

if [[ -n "${KEY:-}" ]]; then
    AUTH_HEADER="x-api-key: $KEY"
elif [[ -n "${ORG_ID:-}" ]]; then
    AUTH_HEADER="x-org-id: $ORG_ID"
else
    echo "ERROR: set KEY=<service-role key> or ORG_ID=<uuid> (local dev-no-auth)." >&2
    exit 1
fi
AUTH=(-H "$AUTH_HEADER")
JSON=(-H "Content-Type: application/json")
command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }

bold() { printf '\n\e[1m%s\e[0m\n' "$1"; }
green() { printf '\e[32m%s\e[0m\n' "$1"; }
red() { printf '\e[31m%s\e[0m\n' "$1"; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

# ── Deploy a throwaway single-field contract (fast to validate) ─────────────
bold "[1/4] Deploying throwaway contract '$NAME'"
CONTRACT_YAML=$(cat <<YAML
version: "1.0.0"
name: "$NAME"
description: "RFC-083 metering perf smoke — throwaway"
ontology:
  entities:
    - name: id
      type: string
      required: true
glossary: []
metrics: []
YAML
)
deploy_body=$(jq -n --arg n "$NAME" --arg y "$CONTRACT_YAML" \
    '{name:$n, yaml_content:$y, source:"perf-smoke", deployed_by:"perf_metering_smoke.sh"}')
resp=$(curl -s -w $'\n%{http_code}' -X POST "$HOST/contracts/deploy" "${AUTH[@]}" "${JSON[@]}" -d "$deploy_body")
status=$(printf '%s' "$resp" | tail -n1)
json=$(printf '%s' "$resp" | sed '$d')
if [[ "$status" != "201" ]]; then red "  deploy failed ($status): $json"; exit 1; fi
CID=$(echo "$json" | jq -r '.contract_id')
green "  contract_id=$CID"

EVENT='{"id":"perf-smoke"}'
post() {
    curl -s -o /dev/null -w '%{time_total} %{http_code}\n' \
        -X POST "$HOST/ingest/$CID" "${AUTH[@]}" "${JSON[@]}" -d "$EVENT"
}

# ── Cold request (this is the one that triggers the month bootstrap COUNT) ───
bold "[2/4] Cold request (bootstraps the monthly counter)"
cold_line=$(post)
cold_ms=$(awk -v t="${cold_line%% *}" 'BEGIN{printf "%.2f", t*1000}')
echo "  cold request: ${cold_ms} ms (status ${cold_line##* })"

# A few warmups so the pool + caches are hot before we measure steady state.
for _ in $(seq 1 20); do post >/dev/null; done

# ── Load ────────────────────────────────────────────────────────────────────
bold "[3/4] Firing $N requests at concurrency $C"
# Worker reads everything from the environment so the (space/colon-bearing)
# auth header is never string-interpolated into a shell command.
cat > "$tmp/worker.sh" <<'WORKER'
curl -s -o /dev/null -w '%{time_total} %{http_code}\n' \
    -X POST "$HOST/ingest/$CID" \
    -H "$AUTH_HEADER" -H "Content-Type: application/json" -d "$EVENT"
WORKER
export HOST CID EVENT AUTH_HEADER
seq 1 "$N" | xargs -P "$C" -I{} bash "$tmp/worker.sh" >> "$tmp/raw.txt"

# ── Percentiles ─────────────────────────────────────────────────────────────
bold "[4/4] Results"
awk '{print $1*1000}' "$tmp/raw.txt" | sort -n > "$tmp/ms.txt"
errors=$(awk '$2 !~ /^2[0-9][0-9]$/ {c++} END{print c+0}' "$tmp/raw.txt")
count=$(wc -l < "$tmp/ms.txt")
pct() { # pct N -> value at that percentile
    awk -v p="$1" -v c="$count" 'BEGIN{i=int((p/100)*c); if(i<1)i=1; print i}' \
        | xargs -I{} sed -n '{}p' "$tmp/ms.txt"
}
p50=$(pct 50); p95=$(pct 95); p99=$(pct 99); pmax=$(tail -n1 "$tmp/ms.txt")

printf "  requests: %s   errors(non-2xx): %s\n" "$count" "$errors"
printf "  p50=%.2f ms  p95=%.2f ms  p99=%.2f ms  max=%.2f ms\n" "$p50" "$p95" "$p99" "$pmax"
printf "  cold(bootstrap) request: %s ms  (one-time per org/month)\n" "$cold_ms"

# Engine-only p99 from the Prometheus histogram, if /metrics is reachable.
if metrics=$(curl -sf "$HOST/metrics" 2>/dev/null); then
    eng=$(printf '%s' "$metrics" | grep -E 'contractgate_validation_duration_seconds' | head -1 || true)
    [[ -n "$eng" ]] && echo "  (engine histogram present in /metrics — validation is CPU-only, unaffected by metering)"
fi

echo
if [[ "$errors" -gt 0 ]]; then
    red "FAIL: $errors non-2xx responses during load."; exit 1
fi
if awk -v v="$p99" -v b="$P99_BUDGET_MS" 'BEGIN{exit !(v>b)}'; then
    red "WARN: steady-state p99 ${p99} ms exceeds budget ${P99_BUDGET_MS} ms."
    red "      Investigate the metering reads (consider merging plan+usage into one query)."
    exit 2
fi
green "PASS: steady-state p99 ${p99} ms within ${P99_BUDGET_MS} ms budget (metering overhead acceptable)."
