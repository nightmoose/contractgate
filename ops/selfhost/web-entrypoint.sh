#!/bin/sh
# Substitute the generated anon key into the built dashboard, then start it.
# The image is built with a non-credential placeholder so the key does not
# have to exist at image build time.
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

export SUPABASE_ANON_KEY
export SUPABASE_SERVICE_ROLE_KEY

token="__CG_SELFHOST_ANON_KEY__"
key="${SUPABASE_ANON_KEY:-}"
if [ -z "$key" ]; then
  echo "self-host: anon key is empty" >&2
  exit 1
fi
case "$key" in
  *[!A-Za-z0-9._=-]*)
    echo "self-host: anon key has characters the dashboard bundle cannot embed" >&2
    exit 1
    ;;
esac

if [ ! -f /tmp/selfhost-anon-substituted ]; then
  found=$(grep -rl "$token" /app 2>/dev/null || true)
  if [ -z "$found" ]; then
    echo "self-host: dashboard image is missing the anon key placeholder; rebuild the web image" >&2
    exit 1
  fi
  printf '%s\n' "$found" | while IFS= read -r path; do
    [ -n "$path" ] || continue
    sed -i "s|$token|$key|g" "$path"
  done
  if grep -rq "$token" /app; then
    echo "self-host: anon key placeholder remains after substitution" >&2
    exit 1
  fi
  touch /tmp/selfhost-anon-substituted
fi

exec "$@"
