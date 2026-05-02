#!/usr/bin/env bash
# ContractGate — Postgres initdb wrapper (Compose only).
#
# Runs once when the `postgres` service first boots an empty data dir.
# Applies every file in /migrations/*.sql (numeric order via `ls -v`)
# and then every file in /seed/*.sql (compose-only demo data).
#
# Why a wrapper instead of bind-mounting the migrations dir directly into
# /docker-entrypoint-initdb.d:
#   - We need to apply demo seed *after* migrations.  A flat directory
#     mount would require us to either pollute supabase/migrations with
#     compose-only seed files (bad — they'd run in real Supabase) or
#     bind-mount each file individually (verbose + brittle when migrations
#     are added).
#   - This wrapper keeps `supabase/migrations/` pure schema and lets us
#     drop demo data into `ops/postgres/seed/` without affecting prod.
set -euo pipefail

run_dir() {
    local dir="$1"
    local label="$2"
    if [[ ! -d "$dir" ]]; then
        echo "  (no $label dir at $dir; skipping)"
        return 0
    fi
    shopt -s nullglob
    local files=("$dir"/*.sql)
    shopt -u nullglob
    if [[ ${#files[@]} -eq 0 ]]; then
        echo "  (no $label files in $dir; skipping)"
        return 0
    fi
    # Sort numerically by leading digits via ls -v.
    while IFS= read -r f; do
        echo "  → $f"
        psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" -f "$f"
    done < <(ls -v "$dir"/*.sql)
}

echo "ContractGate initdb-wrapper: applying migrations"
run_dir /migrations "migration"

echo "ContractGate initdb-wrapper: applying compose-only seed data"
run_dir /seed "seed"

echo "ContractGate initdb-wrapper: complete"
