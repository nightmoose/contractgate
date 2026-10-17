#!/usr/bin/env bash
# Generator and bind guard. No Docker.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

gen() {
  env -u SUPABASE_JWT_SECRET -u SUPABASE_ANON_KEY -u SUPABASE_SERVICE_ROLE_KEY \
    -u SELFHOST_DB_PASSWORD \
    SECRETS_FILE="$1" SELFHOST_BIND="${2:-127.0.0.1}" \
    node "$ROOT/ops/selfhost/ensure-secrets.mjs"
}

echo "=== generate, then reuse ==="
gen "$TMP/a/secrets.env"
cp "$TMP/a/secrets.env" "$TMP/a/first"
gen "$TMP/a/secrets.env"
cmp "$TMP/a/first" "$TMP/a/secrets.env"
test -s "$TMP/a/secrets.env"
grep -q '^SUPABASE_JWT_SECRET=.' "$TMP/a/secrets.env"
grep -q '^SUPABASE_ANON_KEY=eyJ' "$TMP/a/secrets.env"
grep -q '^SUPABASE_SERVICE_ROLE_KEY=eyJ' "$TMP/a/secrets.env"
grep -q '^SELFHOST_DB_PASSWORD=.' "$TMP/a/secrets.env"
if grep -q 'selfhost-placeholder-jwt-secret-change-me' "$TMP/a/secrets.env"; then
  echo "generated file contains the public placeholder" >&2
  exit 1
fi

echo "=== second directory differs ==="
gen "$TMP/b/secrets.env"
if cmp -s "$TMP/a/secrets.env" "$TMP/b/secrets.env"; then
  echo "two first boots produced the same secrets" >&2
  exit 1
fi

echo "=== public bind rejects placeholders ==="
set +e
out=$(SUPABASE_JWT_SECRET=selfhost-placeholder-jwt-secret-change-me \
  SUPABASE_ANON_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlLWRlbW8iLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MjAwMDAwMDAwMH0.FQCUyi7iBQeSGaeFYGpBRscSATPfYZlZUEi6YhcvRgo \
  SUPABASE_SERVICE_ROLE_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIiwiaXNzIjoic3VwYWJhc2UtZGVtbyIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoyMDAwMDAwMDAwfQ.fVtRW0gwj7WJkhMjWIaC40ZZOS9PaN4DMe_DNtFvIJM \
  SELFHOST_DB_PASSWORD=dbpassdbpassdbpassdbpassdbpass \
  SELFHOST_BIND=0.0.0.0 \
  SECRETS_FILE="$TMP/c/secrets.env" \
  node "$ROOT/ops/selfhost/ensure-secrets.mjs" 2>&1)
code=$?
set -e
test "$code" -ne 0
echo "$out" | grep -q 'refusing to start'
test ! -f "$TMP/c/secrets.env"

echo "=== loopback allows an explicit placeholder set ==="
SUPABASE_JWT_SECRET=selfhost-placeholder-jwt-secret-change-me \
  SUPABASE_ANON_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlLWRlbW8iLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MjAwMDAwMDAwMH0.FQCUyi7iBQeSGaeFYGpBRscSATPfYZlZUEi6YhcvRgo \
  SUPABASE_SERVICE_ROLE_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIiwiaXNzIjoic3VwYWJhc2UtZGVtbyIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoyMDAwMDAwMDAwfQ.fVtRW0gwj7WJkhMjWIaC40ZZOS9PaN4DMe_DNtFvIJM \
  SELFHOST_DB_PASSWORD=dbpassdbpassdbpassdbpassdbpass \
  SELFHOST_BIND=127.0.0.1 \
  SECRETS_FILE="$TMP/d/secrets.env" \
  node "$ROOT/ops/selfhost/ensure-secrets.mjs"
test -f "$TMP/d/secrets.env"

echo "=== guard ==="
SELFHOST_BIND=127.0.0.1 SECRETS_FILE="$TMP/d/secrets.env" /bin/sh "$ROOT/ops/selfhost/guard.sh"
set +e
out=$(SELFHOST_BIND=0.0.0.0 SECRETS_FILE="$TMP/d/secrets.env" /bin/sh "$ROOT/ops/selfhost/guard.sh" 2>&1)
code=$?
set -e
test "$code" -ne 0
echo "$out" | grep -q 'refusing to start'
SELFHOST_BIND=0.0.0.0 SECRETS_FILE="$TMP/a/secrets.env" /bin/sh "$ROOT/ops/selfhost/guard.sh"

echo "=== print does not write ==="
node "$ROOT/ops/selfhost/ensure-secrets.mjs" --print | grep -q '^SUPABASE_JWT_SECRET='

echo "selfhost_secrets_test: ok"
