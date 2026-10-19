#!/bin/sh
# Apply schema after GoTrue has created the real auth.users table.
# 006 then installs the NULL auth.uid() stub; 044 replaces only that stub.
#
# Applied filenames are recorded in selfhost.schema_migrations so a
# later `up` skips them. Existing migration SQL is not rewritten. A database
# that already finished 043 (the index exists) and has an empty ledger is
# baselined through 043; 044 stays eligible because it is safe to run again.
set -eu

file="${SECRETS_FILE:-/selfhost/secrets/secrets.env}"
if [ -f "$file" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$file"
  set +a
  export PGPASSWORD="${SELFHOST_DB_PASSWORD}"
fi

echo "selfhost-apply: waiting for auth.users.encrypted_password"
i=0
ready=0
while [ "$i" -lt 60 ]; do
    if psql -v ON_ERROR_STOP=1 -tAc \
        "SELECT 1 FROM information_schema.columns WHERE table_schema = 'auth' AND table_name = 'users' AND column_name = 'encrypted_password'" \
        | grep -q 1; then
        ready=1
        break
    fi
    i=$((i + 1))
    sleep 2
done

if [ "$ready" -ne 1 ]; then
    echo "selfhost-apply: auth schema was not created" >&2
    exit 1
fi

psql -v ON_ERROR_STOP=1 <<'SQL'
CREATE SCHEMA IF NOT EXISTS selfhost;
CREATE TABLE IF NOT EXISTS selfhost.schema_migrations (
    filename text PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT now()
);
SQL

index_exists=$(psql -v ON_ERROR_STOP=1 -tAc \
  "SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relname = 'idx_forwarded_events_contract_created_id'")
index_exists=$(printf '%s' "$index_exists" | tr -d '[:space:]')
ledger=$(psql -v ON_ERROR_STOP=1 -tAc "SELECT count(*) FROM selfhost.schema_migrations")
ledger=$(printf '%s' "$ledger" | tr -d '[:space:]')

safe_name() {
  case "$1" in
    *[!A-Za-z0-9._-]*)
      echo "selfhost-apply: unsafe migration name $1" >&2
      exit 1
      ;;
  esac
}

if [ "$ledger" = "0" ] && [ "$index_exists" = "1" ]; then
  echo "selfhost-apply: recording migrations already present through 043"
  for f in $(ls /migrations/*.sql | sort); do
    base=$(basename "$f")
    safe_name "$base"
    case "$base" in
      044_*|04[5-9]_*|[1-9][0-9][0-9]_*)
        ;;
      *)
        psql -v ON_ERROR_STOP=1 -c \
          "INSERT INTO selfhost.schema_migrations (filename) VALUES ('$base') ON CONFLICT DO NOTHING"
        ;;
    esac
  done
fi

echo "selfhost-apply: applying migrations"
for f in $(ls /migrations/*.sql | sort); do
  base=$(basename "$f")
  safe_name "$base"
  seen=$(psql -v ON_ERROR_STOP=1 -tAc \
    "SELECT 1 FROM selfhost.schema_migrations WHERE filename = '$base'")
  seen=$(printf '%s' "$seen" | tr -d '[:space:]')
  if [ "$seen" = "1" ]; then
    echo "  skip $base"
    continue
  fi
  echo "  → $f"
  psql -v ON_ERROR_STOP=1 -f "$f"
  psql -v ON_ERROR_STOP=1 -c \
    "INSERT INTO selfhost.schema_migrations (filename) VALUES ('$base')"
done

echo "selfhost-apply: applying grants"
psql -v ON_ERROR_STOP=1 -f /selfhost/grants.sql
echo "selfhost-apply: complete"
