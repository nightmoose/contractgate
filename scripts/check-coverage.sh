#!/usr/bin/env bash
# RFC-071 — ratchet-style coverage gate.
#
# Measures total line coverage of the unit-test suite with cargo-llvm-cov and
# compares it to a committed baseline (coverage-baseline.txt). The gate is a
# ratchet: it fails only when coverage DROPS more than TOLERANCE below the
# baseline. It never requires hitting a fixed high bar — the cheapest way to
# make coverage monotonic without an upfront test-writing push.
#
# First run (no baseline file): measures, writes the baseline, and passes. This
# makes the gate non-blocking to land; the seeded number is committed in a
# follow-up so future PRs ratchet against it.
#
# Env:
#   COVERAGE_BASELINE_FILE  path to the baseline file (default: coverage-baseline.txt)
#   COVERAGE_TOLERANCE      allowed drop in percentage points (default: 0.5)
#
# Exit codes: 0 = pass (or seeded), 1 = coverage regressed, 2 = tooling/parse error.
set -euo pipefail

BASELINE_FILE="${COVERAGE_BASELINE_FILE:-coverage-baseline.txt}"
TOLERANCE="${COVERAGE_TOLERANCE:-0.5}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::error::cargo-llvm-cov not installed. Run: cargo install cargo-llvm-cov --locked"
  exit 2
fi

# --summary-only prints a table; the TOTAL row's last numeric column is the
# line-coverage percentage. Capture the whole summary for the log, then parse.
echo "Measuring unit-test line coverage (cargo llvm-cov)..."
SUMMARY="$(cargo llvm-cov --summary-only 2>&1)" || {
  echo "$SUMMARY"
  echo "::error::cargo llvm-cov failed."
  exit 2
}
echo "$SUMMARY"

# The TOTAL line looks like:
#   TOTAL   1234   210   82.97%   ...   <line%>%   ...
# llvm-cov's column order is regions/functions/lines/branches. We extract the
# "Lines" coverage column robustly by reading the percentage that follows the
# line counts. Simpler + stable: grep the TOTAL row and take the 4th percentage.
COVERAGE="$(printf '%s\n' "$SUMMARY" \
  | awk '/^TOTAL/ { for (i=1;i<=NF;i++) if ($i ~ /%$/) { gsub(/%/,"",$i); pct[++n]=$i } }
         END { if (n>=3) print pct[3]; else if (n>0) print pct[n] }')"

if [[ -z "${COVERAGE:-}" ]]; then
  echo "::error::Could not parse coverage percentage from llvm-cov summary."
  exit 2
fi
echo "Measured line coverage: ${COVERAGE}%"

if [[ ! -f "$BASELINE_FILE" ]]; then
  printf '%s\n' "$COVERAGE" > "$BASELINE_FILE"
  echo "No baseline found — seeded ${BASELINE_FILE} with ${COVERAGE}%. Commit this file."
  echo "::notice::Coverage baseline seeded at ${COVERAGE}%."
  exit 0
fi

BASELINE="$(tr -d '[:space:]' < "$BASELINE_FILE")"
echo "Baseline coverage: ${BASELINE}% (tolerance ${TOLERANCE}pp)"

# Pass if COVERAGE >= BASELINE - TOLERANCE. Use awk for float comparison.
if awk -v c="$COVERAGE" -v b="$BASELINE" -v t="$TOLERANCE" 'BEGIN { exit !(c + t >= b) }'; then
  echo "Coverage OK: ${COVERAGE}% >= ${BASELINE}% - ${TOLERANCE}pp."
  # Ratchet up: if coverage improved beyond the baseline, nudge the baseline so
  # the gain is locked in. Only rewrites the file in CI when it actually rises.
  if awk -v c="$COVERAGE" -v b="$BASELINE" 'BEGIN { exit !(c > b) }'; then
    printf '%s\n' "$COVERAGE" > "$BASELINE_FILE"
    echo "::notice::Coverage rose to ${COVERAGE}% — baseline ratcheted up (commit ${BASELINE_FILE} to lock it in)."
  fi
  exit 0
fi

echo "::error::Coverage regressed: ${COVERAGE}% is more than ${TOLERANCE}pp below baseline ${BASELINE}%."
echo "Add tests or, if the drop is intentional, lower ${BASELINE_FILE} in this PR with a rationale."
exit 1
