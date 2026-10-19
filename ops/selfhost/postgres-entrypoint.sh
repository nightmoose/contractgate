#!/bin/sh
# Apply the generated database password before the official entrypoint
# initializes an empty data directory.
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

if [ -z "${SELFHOST_DB_PASSWORD:-}" ]; then
  echo "self-host: SELFHOST_DB_PASSWORD is empty" >&2
  exit 1
fi

template=/selfhost/roles.sql
if ! grep -q '__SELFHOST_DB_PASSWORD__' "$template"; then
  echo "self-host: roles template is missing the password token" >&2
  exit 1
fi

export POSTGRES_PASSWORD="$SELFHOST_DB_PASSWORD"
sed "s/__SELFHOST_DB_PASSWORD__/${SELFHOST_DB_PASSWORD}/g" "$template" > /tmp/selfhost-roles.sql
if grep -q '__SELFHOST_DB_PASSWORD__' /tmp/selfhost-roles.sql; then
  echo "self-host: password substitution failed" >&2
  exit 1
fi

exec docker-entrypoint.sh "$@"
