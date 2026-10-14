#!/usr/bin/env bash
# Throwaway-DB test for migration 042. Usage: DATABASE_URL=postgres://... (empty DB)
# Applies 001..041, seeds name-collision fixtures, applies 042, checks the
# backfill and RLS (as role "authenticated"). Never point this at prod.
set -euo pipefail
export PGOPTIONS='-c client_min_messages=warning'
cd "$(dirname "$0")/.."
: "${DATABASE_URL:?set DATABASE_URL to an empty throwaway database}"
P=(psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q)
for f in $(ls -v supabase/migrations/*.sql); do
  [[ "$(basename "$f")" == 042_* ]] && break
  "${P[@]}" -f "$f" >/dev/null
done
"${P[@]}" <<'SQL'
-- test-only: Supabase grants table access to authenticated by default
grant usage on schema public, auth to authenticated;
grant select on all tables in schema public to authenticated;
grant execute on function public.get_my_org_ids(), auth.uid() to authenticated;
-- test-only: let auth.uid() read a session setting
create or replace function auth.uid() returns uuid language sql stable
  as $$ select nullif(current_setting('test.uid', true), '')::uuid $$;
insert into auth.users (id, email) values
  ('00000000-0000-0000-0000-0000000000a1', 'a@test.invalid'),
  ('00000000-0000-0000-0000-0000000000b1', 'b@test.invalid'),
  ('00000000-0000-0000-0000-0000000000c1', 'c@test.invalid');
insert into orgs (id, name, slug) values
  ('00000000-0000-0000-0000-00000000000a', 'A', 't42-a'),
  ('00000000-0000-0000-0000-00000000000b', 'B', 't42-b'),
  ('00000000-0000-0000-0000-00000000000c', 'C', 't42-c');
insert into org_memberships (org_id, user_id) values
  ('00000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-0000000000a1'),
  ('00000000-0000-0000-0000-00000000000b', '00000000-0000-0000-0000-0000000000b1'),
  ('00000000-0000-0000-0000-00000000000c', '00000000-0000-0000-0000-0000000000c1');
-- A and B both own "orders"; C owns two live "dup" contracts (ambiguous).
insert into contracts (id, name, org_id) values
  ('10000000-0000-0000-0000-00000000000a', 'orders', '00000000-0000-0000-0000-00000000000a'),
  ('10000000-0000-0000-0000-00000000000b', 'orders', '00000000-0000-0000-0000-00000000000b'),
  ('10000000-0000-0000-0000-0000000000c1', 'dup',    '00000000-0000-0000-0000-00000000000c'),
  ('10000000-0000-0000-0000-0000000000c2', 'dup',    '00000000-0000-0000-0000-00000000000c');
-- A grants C viewer on A's orders; C grants B viewer on ambiguous dup.
insert into contract_collaborators (contract_name, org_id, role, granted_by) values
  ('orders', '00000000-0000-0000-0000-00000000000c', 'viewer', '00000000-0000-0000-0000-00000000000a'),
  ('dup',    '00000000-0000-0000-0000-00000000000b', 'viewer', '00000000-0000-0000-0000-00000000000c');
insert into contract_comments (contract_name, org_id, author, body) values
  ('orders', '00000000-0000-0000-0000-00000000000c', 'c', 'C on A orders'),
  ('orders', '00000000-0000-0000-0000-00000000000b', 'b', 'B on own orders'),
  ('dup',    '00000000-0000-0000-0000-00000000000c', 'c', 'ambiguous');
insert into contract_change_proposals (contract_name, proposed_by, proposed_yaml) values
  ('orders', '00000000-0000-0000-0000-00000000000a', 'x: 1'),
  ('nope',   '00000000-0000-0000-0000-00000000000a', 'x: 2');
SQL
"${P[@]}" -f supabase/migrations/042_collaboration_contract_id.sql >/dev/null
check() { local got; got=$(psql "$DATABASE_URL" -Atq -c "$2"); [[ "$got" == "$3" ]] || { echo "FAIL $1: got '$got' want '$3'"; exit 1; }; echo "ok   $1"; }
check "grant points to A's orders"   "select contract_id from contract_collaborators" "10000000-0000-0000-0000-00000000000a"
check "comments backfilled per org"  "select string_agg(contract_id::text, ',' order by body) from contract_comments" \
      "10000000-0000-0000-0000-00000000000b,10000000-0000-0000-0000-00000000000a"
check "ambiguous/orphan proposal dropped" "select count(*) from contract_change_proposals" "1"
check "contract_id not null"  "select count(*) from information_schema.columns where column_name='contract_id' and is_nullable='NO' and table_name in ('contract_collaborators','contract_comments','contract_change_proposals')" "3"
check "pk is (contract_id, org_id)" "select pg_get_constraintdef(oid) from pg_constraint where conname='contract_collaborators_pkey'" "PRIMARY KEY (contract_id, org_id)"
as() { psql "$DATABASE_URL" -Atq -c "begin; set local role authenticated; set local test.uid = '$1'; $2; commit;" | tail -n +1; }
rls() { local got; got=$(as "$2" "$3"); [[ "$got" == "$4" ]] || { echo "FAIL $1: got '$got' want '$4'"; exit 1; }; echo "ok   $1"; }
rls "C (viewer) reads A's contract, no recursion" 00000000-0000-0000-0000-0000000000c1 "select string_agg(org_id::text, ',') from contracts where name='orders'" "00000000-0000-0000-0000-00000000000a"
rls "C sees its comment on A orders" 00000000-0000-0000-0000-0000000000c1 "select count(*) from contract_comments" "1"
rls "A sees C's grant"               00000000-0000-0000-0000-0000000000a1 "select count(*) from contract_collaborators" "1"
rls "A reads proposals"              00000000-0000-0000-0000-0000000000a1 "select count(*) from contract_change_proposals" "1"
check "same name, other org: separate grant allowed" "insert into contract_collaborators (contract_id, org_id, role, granted_by) values ('10000000-0000-0000-0000-00000000000b','00000000-0000-0000-0000-00000000000c','viewer','00000000-0000-0000-0000-00000000000b') returning 1" "1"
check "cascade on contract delete" "delete from contracts where id='10000000-0000-0000-0000-00000000000b'; select count(*) from contract_comments where contract_id='10000000-0000-0000-0000-00000000000b'" "0"
echo "migration 042: all checks passed"
