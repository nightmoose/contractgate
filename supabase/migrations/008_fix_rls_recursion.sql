-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 008: Fix infinite-recursion RLS on org_memberships (error 42P17)
--
-- The policies created in 007 check `org_memberships` from within
-- `org_memberships` policies, causing PostgreSQL to recurse forever.
--
-- Fix: introduce a SECURITY DEFINER helper function that reads
-- org_memberships bypassing RLS.  All policies that need "what orgs does
-- this user belong to?" call the function instead of querying the table
-- directly.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. Helper: returns the set of org_ids the current user is a member of ────
--
-- SECURITY DEFINER + explicit search_path = runs as the function owner
-- (postgres / service role) so RLS on org_memberships is not re-evaluated,
-- breaking the recursion.  STABLE so the planner can cache the result within
-- a single statement.

create or replace function public.get_my_org_ids()
returns setof uuid
language sql
security definer
stable
set search_path = public
as $$
    select org_id
    from   public.org_memberships
    where  user_id = auth.uid();
$$;

-- ── 2. Rebuild org_memberships policies using the helper ──────────────────────

drop policy if exists "org members can read memberships"          on public.org_memberships;
drop policy if exists "org owners and admins can manage memberships" on public.org_memberships;
drop policy if exists "org owners and admins can delete memberships" on public.org_memberships;

-- SELECT: any member of the org can see the membership list.
create policy "org members can read memberships"
    on public.org_memberships for select
    using (org_id in (select public.get_my_org_ids()));

-- INSERT: only owners/admins may add members.
create policy "org owners and admins can manage memberships"
    on public.org_memberships for insert
    with check (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );

-- DELETE: only owners/admins may remove members.
create policy "org owners and admins can delete memberships"
    on public.org_memberships for delete
    using (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );

-- ── 3. Rebuild orgs SELECT policy using the helper ───────────────────────────
--
-- Not recursive (orgs ≠ org_memberships) but worth switching to the helper
-- for consistency and to avoid any planner re-evaluation cost.

drop policy if exists "org members can read their org" on public.orgs;

create policy "org members can read their org"
    on public.orgs for select
    using (id in (select public.get_my_org_ids()));

-- ── 4. Rebuild downstream table policies using the helper ────────────────────
--
-- contracts / api_keys / org_invites all had correct (non-recursive) policies
-- in 007, but rewriting them to use get_my_org_ids() makes them immune to
-- any future recursion if the table structure changes.

-- contracts
drop policy if exists "org members can read own contracts"   on public.contracts;
drop policy if exists "org members can insert contracts"     on public.contracts;
drop policy if exists "org members can update contracts"     on public.contracts;
drop policy if exists "org members can delete contracts"     on public.contracts;

create policy "org members can read own contracts"
    on public.contracts for select
    using (org_id in (select public.get_my_org_ids()));

create policy "org members can insert contracts"
    on public.contracts for insert
    with check (org_id in (select public.get_my_org_ids()));

create policy "org members can update contracts"
    on public.contracts for update
    using (org_id in (select public.get_my_org_ids()));

create policy "org members can delete contracts"
    on public.contracts for delete
    using (org_id in (select public.get_my_org_ids()));

-- api_keys
drop policy if exists "users can create own api keys" on public.api_keys;

create policy "users can create own api keys"
    on public.api_keys for insert
    with check (
        user_id = auth.uid()
        and org_id in (select public.get_my_org_ids())
    );

-- org_invites
drop policy if exists "org managers can read invites"   on public.org_invites;
drop policy if exists "org managers can create invites" on public.org_invites;
drop policy if exists "org managers can delete invites" on public.org_invites;
drop policy if exists "org managers can revoke invites" on public.org_invites;

create policy "org managers can read invites"
    on public.org_invites for select
    using (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );

create policy "org managers can create invites"
    on public.org_invites for insert
    with check (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );

create policy "org managers can delete invites"
    on public.org_invites for delete
    using (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );

create policy "org managers can revoke invites"
    on public.org_invites for update
    using (
        org_id in (
            select org_id
            from   public.org_memberships
            where  user_id = auth.uid()
              and  role in ('owner', 'admin')
        )
    );
