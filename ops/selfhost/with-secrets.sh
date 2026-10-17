#!/bin/sh
# Load the generated secrets file, then exec the real process.
set -eu

file="${SECRETS_FILE:-/selfhost/secrets/secrets.env}"
if [ ! -f "$file" ]; then
  echo "self-host: missing $file" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
. "$file"
set +a

if [ -z "${SUPABASE_JWT_SECRET:-}" ] || [ -z "${SELFHOST_DB_PASSWORD:-}" ]; then
  echo "self-host: secrets file is incomplete" >&2
  exit 1
fi

export SUPABASE_JWT_SECRET
export SUPABASE_ANON_KEY
export SUPABASE_SERVICE_ROLE_KEY
export SELFHOST_DB_PASSWORD
export GOTRUE_JWT_SECRET="$SUPABASE_JWT_SECRET"
export GOTRUE_DB_DATABASE_URL="postgres://supabase_auth_admin:${SELFHOST_DB_PASSWORD}@postgres:5432/contractgate"
export PGRST_JWT_SECRET="$SUPABASE_JWT_SECRET"
export PGRST_DB_URI="postgres://authenticator:${SELFHOST_DB_PASSWORD}@postgres:5432/contractgate"
export DATABASE_URL="postgres://cg:${SELFHOST_DB_PASSWORD}@postgres/contractgate"
export PGPASSWORD="$SELFHOST_DB_PASSWORD"

exec "$@"
