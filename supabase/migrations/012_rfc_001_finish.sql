-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 012: RFC-001 finish — soft-delete + uuid slug suffix
--
-- Sign-off (2026-05-03) decisions on RFC-001 that postdate migration 007/008:
--   #4  Slug collision strategy   → uuid-8 suffix (was: -2, -3, ...)
--   #6  Deletion semantics        → soft delete; flip CASCADE → RESTRICT
--
-- What changes:
--   MOD  handle_new_user()        — uuid suffix on slug collision
--   ADD  deleted_at               — orgs, org_memberships, contracts, api_keys
--   MOD  FK cascades              → ON DELETE RESTRICT on org_id parents
--   MOD  RLS                      — filter deleted_at IS NULL on selects
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. handle_new_user(): uuid-suffix on slug collision ──────────────────────

create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer set search_path = public
as $$
declare
    v_org_id uuid;
    v_slug   text;
    v_name   text;
begin
    -- 1. Create user_profile row (unchanged from migration 006).
    insert into public.user_profiles (id, display_name)
    values (
        new.id,
        coalesce(new.raw_user_meta_data->>'display_name', split_part(new.email, '@', 1))
    )
    on conflict (id) do nothing;

    -- 2. Derive slug. On the first try, use the bare slug; on uniqueness
    --    conflict, append an 8-char hex suffix from gen_random_uuid().
    --    Two attempts are statistically sufficient (collision odds ~1 in 4B).
    v_name := coalesce(
        new.raw_user_meta_data->>'display_name',
        split_part(new.email, '@', 1)
    );
    v_slug := lower(regexp_replace(v_name, '[^a-zA-Z0-9]+', '-', 'g'));
    v_slug := trim(both '-' from v_slug);

    begin
        insert into public.orgs (name, slug)
        values (v_name, v_slug)
        returning id into v_org_id;
    exception when unique_violation then
        -- One retry with a uuid-8 suffix is enough; if THAT collides too,
        -- something is badly wrong and we want the error to bubble up.
        insert into public.orgs (name, slug)
        values (
            v_name,
            v_slug || '-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 8)
        )
        returning id into v_org_id;
    end;

    -- 3. Make the new user the owner of their org.
    insert into public.org_memberships (org_id, user_id, role)
    values (v_org_id, new.id, 'owner');

    return new;
end;
$$;

-- Trigger reattachment is a no-op if migration 007 already wired it; recreate
-- defensively in case a downstream env applied 012 standalone.
drop trigger if exists on_auth_user_created on auth.users;
create trigger on_auth_user_created
    after insert on auth.users
    for each row execute procedure public.handle_new_user();

-- ── 2. Soft-delete columns ───────────────────────────────────────────────────
--
-- "Never lose data" (sign-off #6). Every row that the app could plausibly
-- want to remove gets a deleted_at timestamp instead of a DELETE.
-- audit_log / forwarded_events / org_invites already model their lifecycle
-- (append-only or have a revoked_at), so they don't need a new column.

alter table public.orgs
    add column if not exists deleted_at timestamptz;

alter table public.org_memberships
    add column if not exists deleted_at timestamptz;

alter table public.contracts
    add column if not exists deleted_at timestamptz;

alter table public.api_keys
    add column if not exists deleted_at timestamptz;

-- Partial indexes keep the hot-path "live rows only" queries fast without
-- bloating the index with tombstones.
create index if not exists orgs_live_idx
    on public.orgs (id) where deleted_at is null;

create index if not exists org_memberships_live_idx
    on public.org_memberships (org_id, user_id) where deleted_at is null;

create index if not exists contracts_live_org_idx
    on public.contracts (org_id) where deleted_at is null;

create index if not exists api_keys_live_org_idx
    on public.api_keys (org_id) where deleted_at is null;

-- ── 3. Flip CASCADE → RESTRICT on org_id parents ─────────────────────────────
--
-- Hard-deleting an org would silently take its memberships, contracts, keys,
-- and audit log with it. We make the hard delete fail fast so the only path
-- that succeeds is the soft delete (set deleted_at).
--
-- We don't touch the user_id → auth.users cascades. When Supabase deletes a
-- user account, cleaning up the join rows is the right behaviour — the audit
-- trail still lives in audit_log (which has no user_id FK).

do $$
declare
    rec record;
begin
    for rec in
        select conname, conrelid::regclass::text as tbl
        from   pg_constraint
        where  contype = 'f'
          and  confrelid = 'public.orgs'::regclass
          and  confdeltype = 'c'  -- 'c' = CASCADE
    loop
        execute format('alter table %s drop constraint %I', rec.tbl, rec.conname);
        execute format(
            'alter table %s add constraint %I foreign key (org_id) '
            'references public.orgs(id) on delete restrict',
            rec.tbl, rec.conname
        );
    end loop;
end;
$$;

-- ── 4. RLS: filter deleted_at IS NULL on member-facing selects ───────────────
--
-- Service-role policies are unchanged — the Rust backend needs to see
-- soft-deleted rows for restore and audit purposes. Only the user-facing
-- (`authenticated`) policies hide tombstones.

-- contracts
drop policy if exists "org members can read own contracts" on public.contracts;
create policy "org members can read own contracts"
    on public.contracts for select
    using (
        deleted_at is null
        and org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );

drop policy if exists "org members can insert contracts" on public.contracts;
create policy "org members can insert contracts"
    on public.contracts for insert
    with check (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );

drop policy if exists "org members can update contracts" on public.contracts;
create policy "org members can update contracts"
    on public.contracts for update
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );

-- DELETE policy stays in place for the soft-delete UPDATE path; we don't
-- expose a hard delete to the dashboard. Drop the prior DELETE policy so a
-- direct DELETE from the dashboard fails closed.
drop policy if exists "org members can delete contracts" on public.contracts;

-- audit_log: hide rows whose owning org has been soft-deleted.
drop policy if exists "org members can read own audit log" on public.audit_log;
create policy "org members can read own audit log"
    on public.audit_log for select
    using (
        org_id in (
            select om.org_id
            from   public.org_memberships om
            join   public.orgs            o on o.id = om.org_id
            where  om.user_id    = auth.uid()
              and  om.deleted_at is null
              and  o.deleted_at  is null
        )
    );

-- api_keys: own keys, live only.
drop policy if exists "users can read own api keys"   on public.api_keys;
drop policy if exists "users can create own api keys" on public.api_keys;
drop policy if exists "users can revoke own api keys" on public.api_keys;

create policy "users can read own api keys"
    on public.api_keys for select
    using (user_id = auth.uid() and deleted_at is null);

create policy "users can create own api keys"
    on public.api_keys for insert
    with check (
        user_id = auth.uid()
        and org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );

-- "Revoke" remains an UPDATE (sets revoked_at and/or deleted_at).
create policy "users can revoke own api keys"
    on public.api_keys for update
    using (user_id = auth.uid());

-- org_memberships: hide soft-deleted rows from the members tab.
drop policy if exists "org members can read memberships" on public.org_memberships;
create policy "org members can read memberships"
    on public.org_memberships for select
    using (
        deleted_at is null
        and org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );

-- orgs: hide soft-deleted orgs from all user-facing reads.
drop policy if exists "org members can read their org" on public.orgs;
create policy "org members can read their org"
    on public.orgs for select
    using (
        deleted_at is null
        and id in (
            select org_id from public.org_memberships
            where user_id = auth.uid() and deleted_at is null
        )
    );
