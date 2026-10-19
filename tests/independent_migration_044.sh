#!/usr/bin/env bash
# Independent check of migration 044.
# A fresh database with the 006 NULL stub gets a JWT-aware auth.uid().
# Running the file again is a no-op. A non-stub function is left unchanged.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SQL="$ROOT/supabase/migrations/044_auth_uid_jwt_when_stub.sql"
NAME="cg044indep"

docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" \
  -e POSTGRES_HOST_AUTH_METHOD=trust \
  -e POSTGRES_USER=cg \
  -e POSTGRES_DB=contractgate \
  -v "$SQL:/044.sql:ro" \
  postgres:16-alpine >/dev/null

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for _ in $(seq 1 60); do
  if docker exec "$NAME" pg_isready -U cg -d contractgate >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$NAME" pg_isready -U cg -d contractgate >/dev/null

psql() {
  docker exec -i "$NAME" psql -v ON_ERROR_STOP=1 -U cg -d contractgate "$@"
}

def() {
  psql -tA -c "SELECT pg_get_functiondef('auth.uid()'::regprocedure)"
}

echo "=== fresh database: 006 stub, then 044 ==="
psql <<'SQL'
CREATE SCHEMA IF NOT EXISTS auth;
CREATE OR REPLACE FUNCTION auth.uid()
RETURNS uuid
LANGUAGE sql
STABLE
AS $$
    SELECT NULL::uuid;
$$;
SQL
psql -f /044.sql >/dev/null
first="$(def)"
printf '%s\n' "$first" | grep -q 'current_setting'
printf '%s\n' "$first" | grep -q 'request.jwt.claim.sub'
printf '%s\n' "$first" | grep -q 'request.jwt.claims'
if printf '%s\n' "$first" | grep -q 'SELECT NULL::uuid'; then
  echo "044 left the NULL stub in place" >&2
  exit 1
fi

echo "=== already applied: second run does not change the function ==="
psql -f /044.sql >/dev/null
second="$(def)"
if [[ "$first" != "$second" ]]; then
  echo "reapplying 044 changed auth.uid()" >&2
  exit 1
fi

echo "=== hosted-style function is a no-op ==="
psql <<'SQL'
CREATE OR REPLACE FUNCTION auth.uid()
RETURNS uuid
LANGUAGE sql
STABLE
AS $$
    SELECT NULLIF(current_setting('request.jwt.claim.sub', true), '')::uuid;
$$;
SQL
hosted="$(def)"
psql -f /044.sql >/dev/null
hosted_after="$(def)"
if [[ "$hosted" != "$hosted_after" ]]; then
  echo "044 rewrote a non-stub auth.uid()" >&2
  printf '%s\n' "$hosted_after" >&2
  exit 1
fi
if printf '%s\n' "$hosted_after" | grep -q 'request.jwt.claims'; then
  echo "044 replaced the hosted function with the self-host body" >&2
  exit 1
fi

echo "=== missing auth.uid() does not fail the file ==="
psql -c "DROP FUNCTION auth.uid();" >/dev/null
psql -f /044.sql >/dev/null

echo "independent migration 044: ok"
