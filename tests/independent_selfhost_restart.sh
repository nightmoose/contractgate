#!/usr/bin/env bash
# Independent check of the documented self-host quickstart.
# Clean volumes, no repo-root .env. Create a user and a contract, then
# restart and down/up without -v. Data must remain and the stack must be
# healthy. Placeholder keys must not publish the API on a public bind.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -e .env ]]; then
  echo "refusing to run: repo-root .env exists" >&2
  exit 1
fi

unset SUPABASE_JWT_SECRET SUPABASE_ANON_KEY SUPABASE_SERVICE_ROLE_KEY SELFHOST_DB_PASSWORD SELFHOST_BIND || true

FILES=(-f "$ROOT/docker-compose.yml" -f "$ROOT/docker-compose.selfhost.yml")
PH_PROJECT="cg-placeholder-expose"
EMAIL="indep-restart@example.com"
PASSWORD="indep-restart-pass"
MARKER="indep-restart-marker"
RAW_KEY="cg_live_indeprestart01"
KEY_PREFIX="${RAW_KEY:0:12}"

# Public placeholders that used to ship in the example env. Not credentials.
PH_JWT="selfhost-placeholder-jwt-secret-change-me"
PH_ANON="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlLWRlbW8iLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MjAwMDAwMDAwMH0.FQCUyi7iBQeSGaeFYGpBRscSATPfYZlZUEi6YhcvRgo"
PH_SERVICE="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIiwiaXNzIjoic3VwYWJhc2UtZGVtbyIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoyMDAwMDAwMDAwfQ.fVtRW0gwj7WJkhMjWIaC40ZZOS9PaN4DMe_DNtFvIJM"

compose() {
  docker compose "${FILES[@]}" "$@"
}

ph_compose() {
  SUPABASE_JWT_SECRET="$PH_JWT" \
  SUPABASE_ANON_KEY="$PH_ANON" \
  SUPABASE_SERVICE_ROLE_KEY="$PH_SERVICE" \
  SELFHOST_DB_PASSWORD="0123456789abcdef0123456789abcdef" \
  SELFHOST_BIND="0.0.0.0" \
  docker compose -p "$PH_PROJECT" "${FILES[@]}" "$@"
}

cleanup() {
  local code=$?
  if [[ "$code" -ne 0 ]]; then
    echo "independent selfhost test failed ($code)" >&2
    compose ps -a >&2 || true
    compose logs --tail 100 gateway web selfhost-migrate auth postgres selfhost-guard selfhost-secrets api >&2 || true
    ph_compose logs --tail 40 selfhost-secrets selfhost-guard >&2 || true
  fi
  ph_compose down -v --remove-orphans >/dev/null 2>&1 || true
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  rm -f /tmp/cg-flag-off.yml /tmp/cg-login.json /tmp/cg-kinesis.json /tmp/cg-me.json /tmp/cg-secrets-a /tmp/cg-secrets-b
  exit "$code"
}
trap cleanup EXIT

echo "=== docs: loopback default and how to expose on purpose ==="
grep -q 'SELFHOST_BIND' docs/self-host-reference.md
grep -q '127.0.0.1' docs/self-host-reference.md
grep -q '0.0.0.0' docs/self-host-reference.md
grep -q 'placeholder' docs/self-host-reference.md
grep -q '127.0.0.1' README.md
grep -q 'SELFHOST_BIND=127.0.0.1' .env.selfhost.example
if grep -q 'selfhost-placeholder-jwt-secret-change-me' .env.selfhost.example; then
  echo "example env still contains the public JWT placeholder" >&2
  exit 1
fi
if grep -Eq 'eyJ[A-Za-z0-9_-]{20,}' .env.selfhost.example; then
  echo "example env still contains a JWT" >&2
  exit 1
fi

compose down -v --remove-orphans >/dev/null 2>&1 || true
ph_compose down -v --remove-orphans >/dev/null 2>&1 || true

echo "=== quickstart: docker compose up --build (no .env) ==="
# Same files and command as docs/self-host-reference.md. Detached so the
# script can continue. Parallel builds are capped so a small machine can finish.
COMPOSE_PARALLEL_LIMIT=1 compose up -d --build

read_secrets() {
  compose run --rm --no-deps --entrypoint cat selfhost-secrets /selfhost/secrets/secrets.env
}

field() {
  awk -F= -v key="$1" '$1 == key { print substr($0, length(key) + 2) }' "$2"
}

wait_http() {
  local url="$1" expect="$2" i=0 code=""
  while true; do
    code="$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "$expect" ]]; then
      return 0
    fi
    i=$((i + 1))
    if [[ "$i" -ge 180 ]]; then
      echo "timeout $url (last status $code)" >&2
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
      echo "expected 127.0.0.1 for $1:$2, got ${got:-<empty>}" >&2
      return 1
      ;;
  esac
  local listeners
  listeners="$(ss -ltn | awk -v port=":$2" '$4 ~ port"$" { print $4 }')"
  if ! printf '%s\n' "$listeners" | grep -qx "127.0.0.1:$2"; then
    echo "port $2 is not bound only to 127.0.0.1: ${listeners:-<none>}" >&2
    return 1
  fi
  if printf '%s\n' "$listeners" | grep -Eq '^(0\.0\.0\.0:|\[::\]:|\*:)'; then
    echo "port $2 is also public: $listeners" >&2
    return 1
  fi
}

refuse_nonloopback() {
  local port="$1" ip
  ip="$(ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -1)"
  if [[ -z "$ip" ]]; then
    echo "no global IPv4 address; skipped non-loopback probe for $port" >&2
    return 0
  fi
  if curl -sf --connect-timeout 2 "http://${ip}:${port}/" >/dev/null; then
    echo "http://${ip}:${port}/ accepted a connection" >&2
    return 1
  fi
}

psql_q() {
  local dbpass="$1" sql="$2"
  compose exec -T -e "PGPASSWORD=${dbpass}" postgres \
    psql -v ON_ERROR_STOP=1 -U cg -d contractgate -tA -c "$sql"
}

echo "=== first boot health and loopback binds ==="
wait_http "http://127.0.0.1:8080/ready" 200
wait_http "http://127.0.0.1:3000/auth/login" 200
assert_loopback gateway 8080
assert_loopback api 8000
assert_loopback web 3000
refuse_nonloopback 8080
refuse_nonloopback 8000
refuse_nonloopback 3000

read_secrets > /tmp/cg-secrets-a
anon="$(field SUPABASE_ANON_KEY /tmp/cg-secrets-a)"
service="$(field SUPABASE_SERVICE_ROLE_KEY /tmp/cg-secrets-a)"
dbpass="$(field SELFHOST_DB_PASSWORD /tmp/cg-secrets-a)"
jwt="$(field SUPABASE_JWT_SECRET /tmp/cg-secrets-a)"
test -n "$anon"
test -n "$service"
test -n "$dbpass"
test "${#jwt}" -ge 32
test "$jwt" != "$PH_JWT"
test "$anon" != "$PH_ANON"

echo "=== create user and contract ==="
signup_code="$(curl -s --max-time 20 -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:8000/auth/v1/signup" \
  -H "apikey: ${anon}" \
  -H "Authorization: Bearer ${anon}" \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}")"
case "$signup_code" in
  200|201) ;;
  *)
    echo "signup returned $signup_code" >&2
    exit 1
    ;;
esac

users="$(psql_q "$dbpass" "SELECT count(*) FROM auth.users WHERE email = '${EMAIL}'")"
profiles="$(psql_q "$dbpass" "SELECT count(*) FROM public.user_profiles p JOIN auth.users u ON u.id = p.id WHERE u.email = '${EMAIL}'")"
owners="$(psql_q "$dbpass" "SELECT count(*) FROM public.org_memberships m JOIN auth.users u ON u.id = m.user_id WHERE u.email = '${EMAIL}' AND m.role = 'owner'")"
test "$users" = "1"
test "$profiles" = "1"
test "$owners" = "1"

ids="$(psql_q "$dbpass" "SELECT u.id || ' ' || m.org_id FROM auth.users u JOIN public.org_memberships m ON m.user_id = u.id WHERE u.email = '${EMAIL}' AND m.role = 'owner'")"
user_id="${ids%% *}"
org_id="${ids##* }"
test -n "$user_id"
test -n "$org_id"

psql_q "$dbpass" "INSERT INTO public.contracts (name, org_id, description) VALUES ('${MARKER}', '${org_id}', 'independent restart marker') RETURNING id" >/dev/null
key_hash="$(printf '%s' "$RAW_KEY" | openssl dgst -sha256 -binary | openssl base64 -A)"
test "${#key_hash}" -eq 44
psql_q "$dbpass" "INSERT INTO public.api_keys (user_id, org_id, name, key_prefix, key_hash) VALUES ('${user_id}', '${org_id}', 'indep', '${KEY_PREFIX}', '${key_hash}')" >/dev/null
contract_id="$(psql_q "$dbpass" "SELECT id FROM public.contracts WHERE name = '${MARKER}' AND org_id = '${org_id}'")"
test -n "$contract_id"

uid_def="$(psql_q "$dbpass" "SELECT pg_get_functiondef('auth.uid()'::regprocedure)")"
printf '%s\n' "$uid_def" | grep -q 'current_setting'
printf '%s\n' "$uid_def" | grep -q 'request.jwt.claims'
ledger="$(psql_q "$dbpass" "SELECT count(*) FROM selfhost.schema_migrations")"
files="$(find "$ROOT/supabase/migrations" -name '*.sql' | wc -l | tr -d ' ')"
test "$ledger" = "$files"
applied_044="$(psql_q "$dbpass" "SELECT count(*) FROM selfhost.schema_migrations WHERE filename = '044_auth_uid_jwt_when_stub.sql'")"
test "$applied_044" = "1"

echo "=== kinesis enable without AWS credentials is 400, not 500 ==="
kinesis_code="$(curl -s --max-time 20 -o /tmp/cg-kinesis.json -w '%{http_code}' -X POST \
  "http://127.0.0.1:8080/contracts/${contract_id}/kinesis-ingress/enable" \
  -H "x-api-key: ${RAW_KEY}" \
  -H "Content-Type: application/json" \
  -d '{}')"
if [[ "$kinesis_code" != "400" ]]; then
  echo "kinesis enable returned $kinesis_code" >&2
  cat /tmp/cg-kinesis.json >&2 || true
  exit 1
fi
python3 - <<'PY'
import json
body = json.load(open("/tmp/cg-kinesis.json"))
assert body.get("status") == 400, body
text = body.get("error", "")
assert "Kinesis ingress is currently unavailable" in text, text
assert "AWS credentials are not configured" in text, text
PY

echo "=== self-hosted /v1/me ==="
me_code="$(curl -s --max-time 15 -o /tmp/cg-me.json -w '%{http_code}' "http://127.0.0.1:8080/v1/me" -H "x-api-key: ${RAW_KEY}")"
test "$me_code" = "200"
python3 - <<'PY'
import json
body = json.load(open("/tmp/cg-me.json"))
assert body.get("self_hosted") is True, body
assert body.get("paid_features") is True, body
assert body.get("plan") == "free", body
PY

echo "=== docker compose restart keeps data ==="
compose restart
wait_http "http://127.0.0.1:8080/ready" 200
wait_http "http://127.0.0.1:3000/auth/login" 200
assert_loopback gateway 8080
read_secrets > /tmp/cg-secrets-b
cmp /tmp/cg-secrets-a /tmp/cg-secrets-b
marker="$(psql_q "$dbpass" "SELECT count(*) FROM public.contracts WHERE name = '${MARKER}'")"
test "$marker" = "1"
login_code="$(curl -s --max-time 20 -o /tmp/cg-login.json -w '%{http_code}' -X POST \
  "http://127.0.0.1:8000/auth/v1/token?grant_type=password" \
  -H "apikey: ${anon}" \
  -H "Authorization: Bearer ${anon}" \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}")"
test "$login_code" = "200"
python3 - <<'PY'
import json
body = json.load(open("/tmp/cg-login.json"))
assert body.get("access_token"), "login response missing access_token"
PY
rm -f /tmp/cg-login.json

echo "=== down/up without -v keeps data and skips applied migrations ==="
compose down --remove-orphans
compose up -d
wait_http "http://127.0.0.1:8080/ready" 200
wait_http "http://127.0.0.1:3000/auth/login" 200
assert_loopback gateway 8080
assert_loopback api 8000
assert_loopback web 3000
read_secrets > /tmp/cg-secrets-b
cmp /tmp/cg-secrets-a /tmp/cg-secrets-b
marker="$(psql_q "$dbpass" "SELECT count(*) FROM public.contracts WHERE name = '${MARKER}'")"
users="$(psql_q "$dbpass" "SELECT count(*) FROM auth.users WHERE email = '${EMAIL}'")"
test "$marker" = "1"
test "$users" = "1"
login_code="$(curl -s --max-time 20 -o /tmp/cg-login.json -w '%{http_code}' -X POST \
  "http://127.0.0.1:8000/auth/v1/token?grant_type=password" \
  -H "apikey: ${anon}" \
  -H "Authorization: Bearer ${anon}" \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}")"
test "$login_code" = "200"
rm -f /tmp/cg-login.json
uid_def="$(psql_q "$dbpass" "SELECT pg_get_functiondef('auth.uid()'::regprocedure)")"
printf '%s\n' "$uid_def" | grep -q 'request.jwt.claims'
ledger="$(psql_q "$dbpass" "SELECT count(*) FROM selfhost.schema_migrations WHERE filename = '044_auth_uid_jwt_when_stub.sql'")"
test "$ledger" = "1"
migrate_log="$(compose logs selfhost-migrate)"
printf '%s\n' "$migrate_log" | grep -q 'skip 001_initial_schema.sql'
printf '%s\n' "$migrate_log" | grep -q 'skip 044_auth_uid_jwt_when_stub.sql'

echo "=== hosted flag off keeps free-plan gates ==="
cat > /tmp/cg-flag-off.yml <<'YAML'
services:
  gateway:
    environment:
      CONTRACTGATE_SELF_HOSTED: "0"
YAML
docker compose "${FILES[@]}" -f /tmp/cg-flag-off.yml up -d --no-deps gateway
wait_http "http://127.0.0.1:8080/ready" 200
me_code="$(curl -s --max-time 15 -o /tmp/cg-me.json -w '%{http_code}' "http://127.0.0.1:8080/v1/me" -H "x-api-key: ${RAW_KEY}")"
test "$me_code" = "200"
python3 - <<'PY'
import json
body = json.load(open("/tmp/cg-me.json"))
assert body.get("self_hosted") is False, body
assert body.get("paid_features") is False, body
assert body.get("plan") == "free", body
PY
hosted_free="$(curl -s --max-time 20 -o /tmp/cg-kinesis.json -w '%{http_code}' -X POST \
  "http://127.0.0.1:8080/contracts/${contract_id}/kinesis-ingress/enable" \
  -H "x-api-key: ${RAW_KEY}" \
  -H "Content-Type: application/json" \
  -d '{}')"
if [[ "$hosted_free" != "403" ]]; then
  echo "hosted free kinesis enable returned $hosted_free" >&2
  cat /tmp/cg-kinesis.json >&2 || true
  exit 1
fi

echo "=== hosted enterprise still does not take the self-host 400 ==="
psql_q "$dbpass" "UPDATE public.orgs SET plan = 'enterprise' WHERE id = '${org_id}'" >/dev/null
hosted_ent="$(curl -s -o /tmp/cg-kinesis.json -w '%{http_code}' --max-time 45 -X POST \
  "http://127.0.0.1:8080/contracts/${contract_id}/kinesis-ingress/enable" \
  -H "x-api-key: ${RAW_KEY}" \
  -H "Content-Type: application/json" \
  -d '{}' || true)"
psql_q "$dbpass" "UPDATE public.orgs SET plan = 'free' WHERE id = '${org_id}'" >/dev/null
if [[ "$hosted_ent" == "400" ]] && grep -q 'AWS credentials are not configured' /tmp/cg-kinesis.json; then
  echo "hosted gateway returned the self-host kinesis 400" >&2
  exit 1
fi
case "$hosted_ent" in
  500|000|"") ;;
  *)
    echo "hosted enterprise kinesis enable returned unexpected $hosted_ent" >&2
    cat /tmp/cg-kinesis.json >&2 || true
    exit 1
    ;;
esac

compose up -d --no-deps gateway
wait_http "http://127.0.0.1:8080/ready" 200

echo "=== placeholder keys refuse a public bind ==="
compose down --remove-orphans
# The dashboard image name is project-scoped. Retag it so the public-bind
# project does not rebuild the dashboard.
src_web="$(compose images web --format '{{.Repository}}:{{.Tag}}' | head -1 || true)"
if [[ -z "$src_web" || "$src_web" == ":" ]]; then
  src_web="$(docker images --format '{{.Repository}}:{{.Tag}}' | awk -F: '$1 ~ /-web$/ { print; exit }')"
fi
docker tag "$src_web" "${PH_PROJECT}-web:latest"
set +e
ph_out="$(ph_compose up -d --no-build 2>&1)"
ph_code=$?
set -e
ph_logs="$(ph_compose logs selfhost-secrets selfhost-guard 2>&1 || true)"
if ! printf '%s\n%s\n' "$ph_out" "$ph_logs" | grep -q 'refusing to start'; then
  echo "placeholder public bind did not refuse" >&2
  printf '%s\n' "$ph_out" "$ph_logs" >&2
  exit 1
fi
if [[ "$ph_code" -eq 0 ]]; then
  echo "placeholder public bind exited 0" >&2
  exit 1
fi
if ss -ltn | awk '$4 ~ /:(8080|8000|3000)$/ { print $4 }' | grep -Eq '^(0\.0\.0\.0:|\[::\]:|\*:)'; then
  echo "placeholder public bind published a data port" >&2
  ss -ltn | awk '$4 ~ /:(8080|8000|3000)$/' >&2 || true
  exit 1
fi
ph_compose down -v --remove-orphans >/dev/null

echo "=== original volumes still have the user after the refused boot ==="
compose up -d
wait_http "http://127.0.0.1:8080/ready" 200
users="$(psql_q "$dbpass" "SELECT count(*) FROM auth.users WHERE email = '${EMAIL}'")"
marker="$(psql_q "$dbpass" "SELECT count(*) FROM public.contracts WHERE name = '${MARKER}'")"
test "$users" = "1"
test "$marker" = "1"
assert_loopback gateway 8080

echo "independent selfhost restart: ok"
