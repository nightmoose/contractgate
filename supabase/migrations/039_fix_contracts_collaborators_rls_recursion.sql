-- Migration 039: break the RLS recursion between contracts and
-- contract_collaborators.
--
-- Migration 021 made each table's SELECT policy read the other:
--
--   contracts               "org members and collaborators can read contracts"
--                            -> subquery on contract_collaborators
--   contract_collaborators  "auth_read"
--                            -> subquery on contracts
--
-- Postgres evaluates the policy of every table a policy reads, so any
-- `authenticated` read of contracts fails with
-- `42P17 infinite recursion detected in policy for relation "contracts"`.
-- The dashboard's Contracts page hits this through the security_invoker view
-- active_contracts_public (PostgREST 500 on every load, found in the
-- 2026-10-13 E2E retest). The gateway uses the service role and is unaffected.
--
-- Fix, same pattern as 008/013: move the cross-table lookup into a
-- SECURITY DEFINER helper, which reads contracts without applying its RLS.
-- The collaborators policy keeps exactly the same meaning.

create or replace function public.get_my_org_contract_names()
returns setof text
language sql
security definer
stable
set search_path = public
as $$
    select c.name
    from   public.contracts c
    where  c.org_id in (select public.get_my_org_ids())
      and  c.deleted_at is null;
$$;

revoke execute on function public.get_my_org_contract_names() from public, anon;
grant  execute on function public.get_my_org_contract_names() to authenticated;

drop policy if exists "auth_read" on public.contract_collaborators;
create policy "auth_read" on public.contract_collaborators
    for select to authenticated
    using (
        -- My org is the collaborator being listed.
        org_id = any (select public.get_my_org_ids())
        -- OR my org owns the contract (owner can see all collaborators).
        or contract_name in (select public.get_my_org_contract_names())
    );
