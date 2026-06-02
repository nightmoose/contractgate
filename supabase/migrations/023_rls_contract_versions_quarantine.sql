-- Migration 023: org-scoped RLS on contract_versions, contract_name_history,
--                and quarantine_events (RFC-040 / P0-2)
--
-- Migrations 002 and 003 created world-readable `auth_all` policies on these
-- three tables.  Migration 007 fixed `contracts`, `audit_log`, and
-- `forwarded_events` but never touched these.  Any authenticated Supabase user
-- could SELECT/INSERT/UPDATE/DELETE every other tenant's YAML, version history,
-- and quarantined payloads via the anon REST API.
--
-- Fix: drop `auth_all`, recreate with org-scoped policies using
-- `public.get_my_org_ids()` — the SECURITY DEFINER helper from migration 008
-- that avoids PG-42P17 infinite recursion on org_memberships.
--
-- The `service_all` policies are left untouched (they are no-ops because the
-- service role bypasses RLS unconditionally in Supabase, but removing them is
-- a separate cleanup task per P2-8).

-- ── contract_versions ─────────────────────────────────────────────────────────
--
-- No direct org_id column — scope through the parent contracts row.

drop policy if exists "auth_all" on public.contract_versions;

create policy "org members can read contract versions"
    on public.contract_versions for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can insert contract versions"
    on public.contract_versions for insert
    to authenticated
    with check (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can update contract versions"
    on public.contract_versions for update
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    )
    with check (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can delete contract versions"
    on public.contract_versions for delete
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

-- ── contract_name_history ─────────────────────────────────────────────────────
--
-- Append-only rename log.  Scope through parent contracts row.
-- No UPDATE or DELETE policy — rows are immutable by design.

drop policy if exists "auth_all" on public.contract_name_history;

create policy "org members can read contract name history"
    on public.contract_name_history for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can insert contract name history"
    on public.contract_name_history for insert
    to authenticated
    with check (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

-- ── quarantine_events ─────────────────────────────────────────────────────────
--
-- Has a direct contract_id FK — same join pattern.

drop policy if exists "auth_all" on public.quarantine_events;

create policy "org members can read quarantine events"
    on public.quarantine_events for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can insert quarantine events"
    on public.quarantine_events for insert
    to authenticated
    with check (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can update quarantine events"
    on public.quarantine_events for update
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    )
    with check (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can delete quarantine events"
    on public.quarantine_events for delete
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );
