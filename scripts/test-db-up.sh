#!/usr/bin/env bash
# Spin up a throwaway Postgres for the #[ignore]d DB-backed tests and apply all
# migrations. Mirrors the CI `migrations-check` service (postgres:16, creds and
# db name matching the DATABASE_URL the tests expect).
#
# The compose stack's Postgres (docker-compose.yml) deliberately publishes NO
# host port and uses different creds (cg/cg/contractgate), so it can't serve
# host-run `cargo test`. This container does.
#
# Usage:
#   bash scripts/test-db-up.sh           # start + migrate
#   bash scripts/test-db-up.sh --down    # tear down
#
# Then:
#   DATABASE_URL=postgres://contractgate:contractgate@localhost:5432/contractgate_test \
#   cargo test --bin contractgate-server -- --ignored --exact \
#     tests::org_scoping::two_org_get_contract_isolation \
#     tests::org_scoping::two_org_get_version_isolation \
#     tests::rfc053_ready_tests::ready_returns_200_live_db
set -euo pipefail

NAME=cg-test-db
USER=contractgate
PASS=contractgate
DB=contractgate_test

if [[ "${1:-}" == "--down" ]]; then
  docker rm -f "$NAME" >/dev/null 2>&1 && echo "Removed $NAME." || echo "$NAME not running."
  exit 0
fi

# Recreate cleanly so migrations always apply to an empty DB.
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" -p 5432:5432 \
  -e POSTGRES_USER="$USER" \
  -e POSTGRES_PASSWORD="$PASS" \
  -e POSTGRES_DB="$DB" \
  postgres:16 >/dev/null

echo -n "Waiting for Postgres to accept connections"
until docker exec "$NAME" pg_isready -U "$USER" -d "$DB" >/dev/null 2>&1; do
  echo -n "."; sleep 1
done
echo " ready."

for f in $(ls -v supabase/migrations/*.sql); do
  echo "  → $f"
  docker exec -i "$NAME" psql -U "$USER" -d "$DB" -v ON_ERROR_STOP=1 < "$f" >/dev/null
done
echo "All migrations applied. DATABASE_URL=postgres://$USER:$PASS@localhost:5432/$DB"
echo "Tear down with: bash scripts/test-db-up.sh --down"
