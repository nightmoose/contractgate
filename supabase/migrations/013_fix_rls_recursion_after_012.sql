-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 013: Restore non-recursive RLS broken by migration 012
--
-- ### What broke
--
-- Migration 008 fixed an infinite-recursion RLS error (PG 42P17) on
-- `org_memberships` by introducing a SECURITY DEFINER helper
-- `public.get_my_org_ids()` and rewriting every "are you a member of this
-- org?" policy to call the helper instead of subquerying `org_memberships`
-- directly.
--
-- Migration 012 added `deleted_at IS NULL` filters by DROP+CREATE on the
-- same policies — but rebuilt them with inline subqueries on
-- `org_memberships`, reintroducing the exact recursion 008 fixed. Symptoms:
-- contract creation in the dashboard returned "Database error" because the
-- INSERT WITH CHECK clause recursively re-applied the SELECT policy on
-- `org_memberships`.
--
-- ### Fix
--
-- 1. Update `get_my_org_ids()` so it returns ONLY live (non-soft-deleted)
--    memberships. This is the universal case post-RFC-001 sign-off — no
--    caller wants tombstoned memberships.
-- 2. Rebuild every RLS policy 012 touched, using the helper. Live-row
--    filtering on the *target* table (e.g. `contracts.deleted_at IS NULL`)
--    stays inline; only the membership lookup goes through the helper.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. Helper: live memberships only ─────────────────────────────────────────

create or replace function public.get_my_org_ids()
returns setof uuid
language sql
security definer
stable
set search_path = public
as $$
    select org_id
    from   public.org_memberships
    where  user_id = auth.uid()
      and  deleted_at is null;
$$;

-- ── 2. Rebuild policies broken by 012 ────────────────────────────────────────

-- contracts
drop policy if exists "org members can read own contracts" on public.contracts;
create policy "org members can read own contracts"
    on public.contracts for select
    using (
        deleted_at is null
        and org_id in (select public.get_my_org_ids())
    );

drop policy if exists "org members can insert contracts" on public.contracts;
create policy "org members can insert contracts"
    on public.contracts for insert
    with check (org_id in (select public.get_my_org_ids()));

drop policy if exists "org members can update contracts" on public.contracts;
create policy "org members can update contracts"
    on public.contracts for update
    using (
        deleted_at is null
        and org_id in (select public.get_my_org_ids())
    );

-- audit_log
drop policy if exists "org members can read own audit log" on public.audit_log;
create policy "org members can read own audit log"
    on public.audit_log for select
    using (org_id in (select public.get_my_org_ids()));

-- api_keys
drop policy if exists "users can create own api keys" on public.api_keys;
create policy "users can create own api keys"
    on public.api_keys for insert
    with check (
        user_id = auth.uid()
        and org_id in (select public.get_my_org_ids())
    );

-- org_memberships (SELECT — the recursive offender)
drop policy if exists "org members can read memberships" on public.org_memberships;
create policy "org members can read memberships"
    on public.org_memberships for select
    using (
        deleted_at is null
        and org_id in (select public.get_my_org_ids())
    );

-- orgs (SELECT)
drop policy if exists "org members can read their org" on public.orgs;
create policy "org members can read their org"
    on public.orgs for select
    using (
        deleted_at is null
        and id in (select public.get_my_org_ids())
    );
