-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 007: Org-scoped tenancy  (RFC-001)
--
-- Implements Option B tenancy: every resource belongs to an org, not a user.
-- Users are members of an org; sharing access = inviting to org.
--
-- What changes:
--   NEW  orgs             — tenant unit; one auto-provisioned per user on signup
--   NEW  org_memberships  — user ↔ org with a role (owner / admin / member)
--   NEW  org_invites      — 7-day invite tokens for the dashboard invite flow
--   MOD  contracts        — add org_id FK
--   MOD  api_keys         — add org_id FK
--   MOD  audit_log        — add org_id FK (denormalised for fast per-org queries)
--   MOD  handle_new_user  — extended to auto-provision org + membership
--   RLS  rewritten on contracts / audit_log / forwarded_events / api_keys
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. orgs ───────────────────────────────────────────────────────────────────

create table if not exists public.orgs (
    id         uuid        primary key default gen_random_uuid(),
    name       text        not null,
    slug       text        not null unique,           -- url-safe, derived from owner email
    plan       text        not null default 'free',   -- 'free' | 'pro' | 'enterprise'
    created_at timestamptz not null default now(),
    constraint orgs_name_not_empty  check (char_length(trim(name))  > 0),
    constraint orgs_slug_not_empty  check (char_length(trim(slug))  > 0),
    constraint orgs_plan_valid      check (plan in ('free', 'pro', 'enterprise'))
);

comment on table public.orgs is
    'Tenant unit. One org is auto-provisioned for every user on first sign-up. '
    'Enterprise customers get their own dedicated instance (RFC-001).';

alter table public.orgs enable row level security;

-- NOTE: The SELECT policy on orgs references org_memberships and is defined
-- below, after org_memberships is created, to avoid a forward-reference error.

-- Only service role writes orgs (the trigger below uses SECURITY DEFINER).
create policy "service role full access to orgs"
    on public.orgs for all
    to service_role
    using (true) with check (true);

-- ── 2. org_memberships ────────────────────────────────────────────────────────

create table if not exists public.org_memberships (
    id         uuid        primary key default gen_random_uuid(),
    org_id     uuid        not null references public.orgs(id) on delete cascade,
    user_id    uuid        not null references auth.users(id) on delete cascade,
    role       text        not null default 'member',  -- 'owner' | 'admin' | 'member'
    invited_by uuid        references auth.users(id),
    joined_at  timestamptz not null default now(),
    constraint org_memberships_unique_user unique (org_id, user_id),
    constraint org_memberships_role_valid  check (role in ('owner', 'admin', 'member'))
);

comment on table public.org_memberships is
    'Maps users to orgs with a role. A user may technically belong to multiple '
    'orgs (data model supports it); the UI exposes single-org in the MVP.';

create index if not exists org_memberships_user_id_idx  on public.org_memberships (user_id);
create index if not exists org_memberships_org_id_idx   on public.org_memberships (org_id);

alter table public.org_memberships enable row level security;

-- Now that org_memberships exists, we can create the orgs SELECT policy that
-- references it (deferred from the orgs block above to avoid a forward reference).
create policy "org members can read their org"
    on public.orgs for select
    using (
        id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

-- Members can see the full membership list for their org (needed for the
-- Members tab in /account).
create policy "org members can read memberships"
    on public.org_memberships for select
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

-- Owners / admins can add and remove members (enforced in app layer too).
create policy "org owners and admins can manage memberships"
    on public.org_memberships for insert
    with check (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

create policy "org owners and admins can delete memberships"
    on public.org_memberships for delete
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

create policy "service role full access to org_memberships"
    on public.org_memberships for all
    to service_role
    using (true) with check (true);

-- ── 3. org_invites ────────────────────────────────────────────────────────────

create table if not exists public.org_invites (
    id          uuid        primary key default gen_random_uuid(),
    org_id      uuid        not null references public.orgs(id) on delete cascade,
    email       text        not null,
    role        text        not null default 'member',
    invited_by  uuid        not null references auth.users(id),
    token       uuid        not null unique default gen_random_uuid(),
    expires_at  timestamptz not null default now() + interval '7 days',
    accepted_at timestamptz,
    revoked_at  timestamptz,
    created_at  timestamptz not null default now(),
    constraint org_invites_role_valid check (role in ('owner', 'admin', 'member'))
);

comment on table public.org_invites is
    '7-day invite tokens. Redeemed at /auth/accept-invite?token=<uuid>. '
    'Accepted invites set accepted_at; expired+unaccepted rows are safe to prune.';

create index if not exists org_invites_token_idx  on public.org_invites (token);
create index if not exists org_invites_org_id_idx on public.org_invites (org_id);

alter table public.org_invites enable row level security;

-- Org owners/admins can see invites for their org.
create policy "org managers can read invites"
    on public.org_invites for select
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

create policy "org managers can create invites"
    on public.org_invites for insert
    with check (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

create policy "org managers can delete invites"
    on public.org_invites for delete
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

-- Revocation is done via UPDATE (setting revoked_at), not DELETE.
create policy "org managers can revoke invites"
    on public.org_invites for update
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

-- Service role needs full access for invite redemption flow.
create policy "service role full access to org_invites"
    on public.org_invites for all
    to service_role
    using (true) with check (true);

-- ── 4. Add org_id to existing tables ─────────────────────────────────────────

-- contracts
alter table public.contracts
    add column if not exists org_id uuid references public.orgs(id) on delete cascade;

create index if not exists contracts_org_id_idx on public.contracts (org_id);

-- api_keys
alter table public.api_keys
    add column if not exists org_id uuid references public.orgs(id) on delete cascade;

create index if not exists api_keys_org_id_idx on public.api_keys (org_id);

-- audit_log
alter table public.audit_log
    add column if not exists org_id uuid references public.orgs(id) on delete cascade;

create index if not exists audit_log_org_id_idx         on public.audit_log (org_id);
create index if not exists audit_log_org_id_created_idx on public.audit_log (org_id, created_at desc);

-- ── 5. RLS: replace the "auth = anyone authenticated" policies ────────────────
--
-- Old policies allowed any authenticated user to read/write any row. We now
-- scope to org membership.

-- contracts
drop policy if exists "auth_all"    on public.contracts;
drop policy if exists "service_all" on public.contracts;

create policy "org members can read own contracts"
    on public.contracts for select
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

create policy "org members can insert contracts"
    on public.contracts for insert
    with check (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

create policy "org members can update contracts"
    on public.contracts for update
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

create policy "org members can delete contracts"
    on public.contracts for delete
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

create policy "service role full access to contracts"
    on public.contracts for all
    to service_role
    using (true) with check (true);

-- audit_log
drop policy if exists "auth_all"    on public.audit_log;
drop policy if exists "service_all" on public.audit_log;

create policy "org members can read own audit log"
    on public.audit_log for select
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

-- Audit rows are written by the Rust backend (service_role), not by the user.
create policy "service role full access to audit_log"
    on public.audit_log for all
    to service_role
    using (true) with check (true);

-- forwarded_events
drop policy if exists "auth_all"    on public.forwarded_events;
drop policy if exists "service_all" on public.forwarded_events;

create policy "service role full access to forwarded_events"
    on public.forwarded_events for all
    to service_role
    using (true) with check (true);

-- api_keys: tighten the existing per-user policy to also check org membership
drop policy if exists "users can read own api keys"         on public.api_keys;
drop policy if exists "users can create own api keys"       on public.api_keys;
drop policy if exists "users can revoke own api keys"       on public.api_keys;
drop policy if exists "service role can read all keys for validation" on public.api_keys;
drop policy if exists "service role can update last_used_at"          on public.api_keys;

create policy "users can read own api keys"
    on public.api_keys for select
    using (user_id = auth.uid());

create policy "users can create own api keys"
    on public.api_keys for insert
    with check (
        user_id = auth.uid()
        and org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

create policy "users can revoke own api keys"
    on public.api_keys for update
    using (user_id = auth.uid());

create policy "service role can read all keys for validation"
    on public.api_keys for select
    to service_role
    using (true);

create policy "service role can update last_used_at"
    on public.api_keys for update
    to service_role
    using (true);

-- ── 6. Auto-provision org + membership on signup ──────────────────────────────
--
-- Replaces / extends the existing handle_new_user() trigger from migration 006.
-- We SECURITY DEFINER so the function can write orgs/memberships without the
-- invoking user's RLS policies blocking it.

create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer set search_path = public
as $$
declare
    v_org_id   uuid;
    v_slug     text;
    v_name     text;
    v_counter  int := 0;
begin
    -- 1. Create user_profile row (unchanged from migration 006).
    insert into public.user_profiles (id, display_name)
    values (
        new.id,
        coalesce(new.raw_user_meta_data->>'display_name', split_part(new.email, '@', 1))
    )
    on conflict (id) do nothing;

    -- 2. Derive a slug from the email prefix, ensuring uniqueness.
    v_name := coalesce(
        new.raw_user_meta_data->>'display_name',
        split_part(new.email, '@', 1)
    );
    -- Sanitise: lowercase, replace non-alphanumeric runs with '-', trim dashes.
    v_slug := lower(regexp_replace(v_name, '[^a-zA-Z0-9]+', '-', 'g'));
    v_slug := trim(both '-' from v_slug);

    -- Collision loop: append -2, -3, ... until the slug is unique.
    loop
        begin
            if v_counter = 0 then
                insert into public.orgs (name, slug)
                values (v_name, v_slug)
                returning id into v_org_id;
            else
                insert into public.orgs (name, slug)
                values (v_name, v_slug || '-' || v_counter)
                returning id into v_org_id;
            end if;
            exit;  -- success
        exception when unique_violation then
            v_counter := v_counter + 1;
        end;
    end loop;

    -- 3. Make the new user the owner of their org.
    insert into public.org_memberships (org_id, user_id, role)
    values (v_org_id, new.id, 'owner');

    return new;
end;
$$;

-- Re-attach trigger (drop + recreate is idempotent).
drop trigger if exists on_auth_user_created on auth.users;
create trigger on_auth_user_created
    after insert on auth.users
    for each row execute procedure public.handle_new_user();

-- ── 7. Backfill existing users → orgs ────────────────────────────────────────
--
-- For every auth.users row that doesn't already have an org, we create one
-- and insert an owner membership. Existing contracts and api_keys are then
-- assigned to the first (only) org owned by each user.
--
-- This is a one-time idempotent script: it no-ops for users who already went
-- through the trigger above.

do $$
declare
    rec        record;
    v_org_id   uuid;
    v_slug     text;
    v_name     text;
    v_counter  int;
begin
    for rec in
        select u.id, u.email, u.raw_user_meta_data
        from auth.users u
        where not exists (
            select 1 from public.org_memberships m where m.user_id = u.id
        )
    loop
        v_name := coalesce(
            rec.raw_user_meta_data->>'display_name',
            split_part(rec.email, '@', 1)
        );
        v_slug    := lower(regexp_replace(v_name, '[^a-zA-Z0-9]+', '-', 'g'));
        v_slug    := trim(both '-' from v_slug);
        v_counter := 0;

        loop
            begin
                if v_counter = 0 then
                    insert into public.orgs (name, slug)
                    values (v_name, v_slug)
                    returning id into v_org_id;
                else
                    insert into public.orgs (name, slug)
                    values (v_name, v_slug || '-' || v_counter)
                    returning id into v_org_id;
                end if;
                exit;
            exception when unique_violation then
                v_counter := v_counter + 1;
            end;
        end loop;

        insert into public.org_memberships (org_id, user_id, role)
        values (v_org_id, rec.id, 'owner');
    end loop;
end;
$$;

-- Assign orphaned contracts (org_id IS NULL) to the owning user's org.
-- "Owning user" here is ambiguous since the old schema had no owner — we
-- pick the earliest membership (owner) for each existing user. If multiple
-- users exist and contracts are truly shared, they all land in the first
-- user's org. This is acceptable for dev/test environments.
update public.contracts c
set    org_id = (
    select m.org_id
    from   public.org_memberships m
    where  m.role = 'owner'
    order  by m.joined_at
    limit  1
)
where  c.org_id is null;

-- Same for api_keys.
update public.api_keys k
set    org_id = (
    select m.org_id
    from   public.org_memberships m
    where  m.user_id = k.user_id
    limit  1
)
where  k.org_id is null;

-- Same for audit_log: derive from the contract's org_id.
update public.audit_log a
set    org_id = (
    select c.org_id
    from   public.contracts c
    where  c.id = a.contract_id
)
where  a.org_id is null;

-- ── 8. Make org_id NOT NULL on contracts and api_keys after backfill ──────────
--
-- We defer this until after the backfill so the ALTER doesn't fail on
-- pre-existing rows. audit_log can remain nullable (historic rows without a
-- contract might arrive from edge cases in the wild).

alter table public.contracts  alter column org_id set not null;
alter table public.api_keys   alter column org_id set not null;
