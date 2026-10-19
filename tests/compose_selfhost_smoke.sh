#!/usr/bin/env bash
# Self-host restart: generate keys, create a user, down, up.
# Keys and the user survive. Gateway and web come back healthy.
# Ports stay on 127.0.0.1. Migrations are not applied twice.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

unset SUPABASE_JWT_SECRET SUPABASE_ANON_KEY SUPABASE_SERVICE_ROLE_KEY SELFHOST_DB_PASSWORD SELFHOST_BIND || true

compose() {
  docker compose -p cgselfhost -f "$ROOT/docker-compose.yml" -f "$ROOT/docker-compose.selfhost.yml" "$@"
}

cleanup() {
  compose down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

read_secrets() {
  docker run --rm -v cgselfhost_selfhost_secrets:/s:ro alpine:3.20 cat /s/secrets.env
}

field() {
  awk -F= -v key="$1" '$1 == key { print substr($0, length(key) + 2) }' "$2"
}

wait_http() {
  local url="$1" expect="$2" i=0 code=""
  while true; do
    code="$(curl -s -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "$expect" ]]; then
      return 0
    fi
    i=$((i + 1))
    if [[ "$i" -ge 180 ]]; then
      echo "timeout $url (last status $code)" >&2
      compose logs --tail 80 gateway web selfhost-migrate auth rest api postgres || true
      return 1
    fi
    sleep 2
  done
}

assert_loopback() {
  local got
  got="$(compose port "$1" "$2")"
  case "$got" in
    127.0.0.1:*) ;;
    *)
      echo "expected 127.0.0.1 for $1:$2, got $got" >&2
      return 1
      ;;
  esac
}

compose down -v --remove-orphans >/dev/null 2>&1 || true

echo "=== selfhost smoke: first up ==="
if ! compose up -d --build; then
  compose logs --tail 120 || true
  exit 1
fi

sec="$(mktemp)"
read_secrets > "$sec"
anon="$(field SUPABASE_ANON_KEY "$sec")"
service="$(field SUPABASE_SERVICE_ROLE_KEY "$sec")"
dbpass="$(field SELFHOST_DB_PASSWORD "$sec")"
jwt="$(field SUPABASE_JWT_SECRET "$sec")"
test -n "$anon"
test -n "$service"
test -n "$dbpass"
test "${#jwt}" -ge 32
test "$jwt" != "selfhost-placeholder-jwt-secret-change-me"

assert_loopback gateway 8080
assert_loopback api 8000
assert_loopback web 3000

wait_http "http://127.0.0.1:8080/ready" 200
wait_http "http://127.0.0.1:3000/auth/login" 200

echo "=== selfhost smoke: create user ==="
curl -sf -X POST "http://127.0.0.1:8000/auth/v1/signup" \
  -H "apikey: $anon" \
  -H "Authorization: Bearer $anon" \
  -H "Content-Type: application/json" \
  -d '{"email":"restart-check@example.com","password":"restart-check-pass"}' \
  >/dev/null

users="$(compose exec -T -e "PGPASSWORD=$dbpass" postgres \
  psql -U cg -d contractgate -tAc "SELECT count(*) FROM auth.users WHERE email = 'restart-check@example.com'")"
users="$(echo "$users" | tr -d '[:space:]')"
test "$users" = "1"

echo "=== selfhost smoke: down, then up ==="
compose down --remove-orphans
if ! compose up -d; then
  compose logs --tail 120 || true
  exit 1
fi

sec2="$(mktemp)"
read_secrets > "$sec2"
cmp "$sec" "$sec2"

wait_http "http://127.0.0.1:8080/ready" 200
wait_http "http://127.0.0.1:3000/auth/login" 200
assert_loopback gateway 8080
assert_loopback api 8000
assert_loopback web 3000

echo "=== selfhost smoke: user and keys still work ==="
if ! curl -sf -X POST "http://127.0.0.1:8000/auth/v1/token?grant_type=password" \
  -H "apikey: $anon" \
  -H "Authorization: Bearer $anon" \
  -H "Content-Type: application/json" \
  -d '{"email":"restart-check@example.com","password":"restart-check-pass"}' \
  >/dev/null; then
  echo "login after restart failed" >&2
  exit 1
fi
if ! curl -sf "http://127.0.0.1:8000/auth/v1/admin/users" \
  -H "apikey: $service" \
  -H "Authorization: Bearer $service" \
  | grep -q 'restart-check@example.com'; then
  echo "service role could not list the user after restart" >&2
  exit 1
fi

# docker exec does not see variables the entrypoint exported into pid 1.
web_key="$(compose exec -T web sh -c 'tr "\0" "\n" < /proc/1/environ' \
  | awk -F= '$1 == "SUPABASE_SERVICE_ROLE_KEY" { print substr($0, index($0, "=") + 1) }' \
  | tr -d '\r')"
if [[ "$web_key" != "$service" ]]; then
  echo "web process is not using the generated service role key" >&2
  exit 1
fi

# The web image is Alpine, so grep is BusyBox and has no --binary-files.
if compose exec -T web grep -R -F -q '__CG_SELFHOST_ANON_KEY__' /app; then
  echo "dashboard still contains the anon key placeholder" >&2
  exit 1
fi

echo "=== selfhost smoke: migrations skipped ==="
# Do not pipe compose logs into grep -q. Compose exits 255 on SIGPIPE
# once grep closes the pipe, and pipefail then fails the script.
migrate_log="$(compose logs selfhost-migrate)"
if ! printf '%s\n' "$migrate_log" | grep -q 'skip 001_initial_schema.sql'; then
  echo "migrations ran again after restart" >&2
  printf '%s\n' "$migrate_log" >&2
  exit 1
fi

echo "selfhost smoke: ok"
rm -f "$sec" "$sec2"
