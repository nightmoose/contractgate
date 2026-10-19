#!/bin/sh
# Refuse public placeholders when published ports are not loopback.
# Local bind (the default) is allowed. Keep the placeholder literals in
# sync with ops/selfhost/ensure-secrets.mjs.
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

bind="${SELFHOST_BIND:-127.0.0.1}"
case "$bind" in
  127.0.0.1|localhost|::1)
    exit 0
    ;;
esac

ph_jwt="selfhost-placeholder-jwt-secret-change-me"
ph_anon="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlLWRlbW8iLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MjAwMDAwMDAwMH0.FQCUyi7iBQeSGaeFYGpBRscSATPfYZlZUEi6YhcvRgo"
ph_service="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIiwiaXNzIjoic3VwYWJhc2UtZGVtbyIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoyMDAwMDAwMDAwfQ.fVtRW0gwj7WJkhMjWIaC40ZZOS9PaN4DMe_DNtFvIJM"

jwt="${SUPABASE_JWT_SECRET:-}"
anon="${SUPABASE_ANON_KEY:-}"
service="${SUPABASE_SERVICE_ROLE_KEY:-}"

if [ -z "$jwt" ] || [ -z "$anon" ] || [ -z "$service" ] \
  || [ "$jwt" = "$ph_jwt" ] || [ "$anon" = "$ph_anon" ] || [ "$service" = "$ph_service" ] \
  || [ "$anon" = "__CG_SELFHOST_ANON_KEY__" ] || [ "$service" = "__CG_SELFHOST_ANON_KEY__" ]; then
  echo "self-host: refusing to start. Ports bind to ${bind}, but the JWT secret or API keys are public placeholders or empty. Use the default bind 127.0.0.1, or generate unique keys (docs/self-host-reference.md) before exposing the stack." >&2
  exit 1
fi
