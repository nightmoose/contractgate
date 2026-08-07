#!/usr/bin/env bash
#
# Compare the prod migration ledger to supabase/migrations/.
#
# The ledger has lied in both directions: 024/025/027 were unapplied while
# looking fine (2026-07-09), and 033/034 were fully applied but untracked
# (2026-08-07). A file count in CI catches neither. This does.
#
#   bash scripts/check_migration_drift.sh                 # uses $SUPABASE_DB_URL
#   bash scripts/check_migration_drift.sh "postgres://…"  # or an explicit URL
#
# Exit 0 clean, 1 on drift, 2 on usage error.

set -euo pipefail

DB_URL="${1:-${SUPABASE_DB_URL:-}}"
if [[ -z "$DB_URL" ]]; then
    echo "usage: bash scripts/check_migration_drift.sh [DB_URL]  (or set SUPABASE_DB_URL)" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mig_dir="$repo_root/supabase/migrations"
unapplied_list="$repo_root/supabase/unapplied-migrations.txt"

# Ledger rows recorded under a name that doesn't match the file stem.
ledger_alias() {
    case "$1" in
        create_early_access) echo "030_early_access" ;;
        *) echo "$1" ;;
    esac
}

# Normalize a list: drop comments/blank lines, strip a .sql suffix, sort unique.
# The trailing `|| true` matters — grep exits 1 on empty input, and under
# `set -euo pipefail` that would abort the script instead of yielding an empty
# list (which is the normal, healthy case here).
clean() {
    sed -e 's/[[:space:]]*$//' -e 's/\.sql$//' |
        grep -vE '^[[:space:]]*(#.*)?$' |
        sort -u || true
}

ledger_raw=$(psql "$DB_URL" -Atc \
    "SELECT name FROM supabase_migrations.schema_migrations ORDER BY version")

ledger=$(while IFS= read -r name; do
    [[ -n "$name" ]] && ledger_alias "$name"
done <<<"$ledger_raw" | clean)

files=$(cd "$mig_dir" && ls -1 ./*.sql | sed 's|^\./||' | clean)

if [[ -f "$unapplied_list" ]]; then
    allowed=$(clean <"$unapplied_list")
else
    allowed=""
fi

missing=$(comm -23 <(printf '%s\n' "$files") <(printf '%s\n' "$ledger") | clean)
unexpected=$(comm -13 <(printf '%s\n' "$files") <(printf '%s\n' "$ledger") | clean)
known=$(comm -12 <(printf '%s\n' "$missing") <(printf '%s\n' "$allowed") | clean)
missing=$(comm -23 <(printf '%s\n' "$missing") <(printf '%s\n' "$allowed") | clean)

status=0

if [[ -n "$known" ]]; then
    echo "Intentionally unapplied (listed in supabase/unapplied-migrations.txt):"
    sed 's/^/  · /' <<<"$known"
fi

if [[ -n "$missing" ]]; then
    echo "::error::In supabase/migrations/ but NOT in the prod ledger — unapplied, or applied without being recorded:"
    sed 's/^/  ✗ /' <<<"$missing"
    echo "Verify against live schema before backfilling: an object that already exists means applied-but-untracked."
    status=1
fi

if [[ -n "$unexpected" ]]; then
    echo "::error::In the prod ledger but has no file in supabase/migrations/ — renamed or applied out-of-band:"
    sed 's/^/  ✗ /' <<<"$unexpected"
    status=1
fi

if [[ $status -eq 0 ]]; then
    echo "Ledger matches supabase/migrations/ ($(wc -l <<<"$files" | tr -d ' ') files)."
fi

exit $status
