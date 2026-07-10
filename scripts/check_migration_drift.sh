#!/usr/bin/env bash
# scripts/check_migration_drift.sh
#
# Compares the migrations tracked in prod's supabase_migrations.schema_migrations
# ledger against the *.sql files in supabase/migrations/. Exits non-zero if:
#   - a migration file exists with no corresponding ledger row, or
#   - a ledger row exists with no corresponding file.
#
# This is the check that would have caught the 2026-06-05 Stripe webhook
# incident (migration 026 had a file but was never applied/tracked in prod).
#
# Known cosmetic ledger/file name mismatches are hard-coded in ALIAS_MAP below
# (ledger name -> file basename, both without the .sql extension). Add new
# entries here if a future migration is applied by hand with a different name.
#
# Usage:
#   DATABASE_URL=postgres://... ./scripts/check_migration_drift.sh
#   ./scripts/check_migration_drift.sh --ledger-file /path/to/names.txt
#
# --ledger-file reads one ledger name per line and skips the DB entirely —
# use it to dry-run/unit-test this script's diff logic without touching prod.
# See docs/migration-drift-check-reference.md for the recipe and the
# read-only prod role this script needs in CI.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MIGRATIONS_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)/supabase/migrations"

# ledger_name -> file_basename (no .sql), for known cosmetic mismatches.
declare -A ALIAS_MAP=(
    ["create_early_access"]="030_early_access"
)

LEDGER_FILE=""
if [[ "${1:-}" == "--ledger-file" ]]; then
    LEDGER_FILE="${2:?--ledger-file requires a path}"
fi

if [[ -n "$LEDGER_FILE" ]]; then
    mapfile -t LEDGER_NAMES < "$LEDGER_FILE"
else
    : "${DATABASE_URL:?DATABASE_URL must be set (or use --ledger-file <path> for a dry run)}"
    mapfile -t LEDGER_NAMES < <(psql "$DATABASE_URL" -Atc \
        "SELECT name FROM supabase_migrations.schema_migrations ORDER BY version")
fi

mapfile -t FILE_BASENAMES < <(ls "$MIGRATIONS_DIR"/*.sql | xargs -n1 basename | sed 's/\.sql$//' | sort)

# Resolve ledger names through the alias map to the file basename they mean.
declare -A RESOLVED_LEDGER
for name in "${LEDGER_NAMES[@]}"; do
    [[ -z "$name" ]] && continue
    resolved="${ALIAS_MAP[$name]:-$name}"
    RESOLVED_LEDGER["$resolved"]=1
done

declare -A FILE_SET
for base in "${FILE_BASENAMES[@]}"; do
    FILE_SET["$base"]=1
done

FAIL=0

echo "== Files with no ledger row =="
for base in "${FILE_BASENAMES[@]}"; do
    if [[ -z "${RESOLVED_LEDGER[$base]:-}" ]]; then
        echo "  MISSING FROM LEDGER: ${base}.sql"
        FAIL=1
    fi
done

echo "== Ledger rows with no file =="
for base in "${!RESOLVED_LEDGER[@]}"; do
    if [[ -z "${FILE_SET[$base]:-}" ]]; then
        echo "  MISSING FILE FOR LEDGER ENTRY: ${base}"
        FAIL=1
    fi
done

if [[ "$FAIL" == "1" ]]; then
    echo "::error::Migration drift detected between the prod ledger and supabase/migrations/*.sql. See docs/migration-drift-check-reference.md."
    exit 1
fi

ALIAS_KEYS="${!ALIAS_MAP[*]}"
echo "OK: prod ledger matches supabase/migrations/*.sql (aliases applied: ${ALIAS_KEYS:-none})."
