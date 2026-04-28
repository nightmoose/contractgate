-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 006: User accounts and API key management
--
-- Adds:
--   user_profiles   — public profile row created on first sign-in
--   api_keys        — per-user API keys (hashed, never stored in plain text)
--
-- Key design decisions:
--   • The full API key is generated client-side and NEVER stored. Only a
--     bcrypt hash (key_hash) and an 8-char prefix (key_prefix) are persisted.
--   • key_prefix is used by the Rust backend for fast O(1) cache-miss lookups
--     (filter by prefix, then verify hash) so we avoid full-table scans.
--   • Revocation is instant: set revoked_at = now(). The 60-second TTL cache
--     in the Rust backend means revocation propagates within one minute.
--   • last_used_at is updated on each validated request for audit trail /
--     key-usage dashboards.
-- ─────────────────────────────────────────────────────────────────────────────

-- Supabase auth compatibility for local/CI Postgres
-- (real Supabase projects create these automatically)

DO $$
BEGIN
    CREATE SCHEMA IF NOT EXISTS auth;
EXCEPTION WHEN duplicate_schema THEN null;
END $$;

DO $$
BEGIN
    CREATE TABLE IF NOT EXISTS auth.users (
        id uuid PRIMARY KEY DEFAULT gen_random_uuid()
    );
EXCEPTION WHEN duplicate_table THEN null;
END $$;

-- Stub for Supabase's auth.uid() helper function
CREATE OR REPLACE FUNCTION auth.uid()
RETURNS uuid
LANGUAGE sql
STABLE
AS $$
    SELECT NULL::uuid;
$$;

-- ── user_profiles ─────────────────────────────────────────────────────────────
create table if not exists public.user_profiles (
    id            uuid        primary key references auth.users(id) on delete cascade,
    display_name  text,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);

comment on table public.user_profiles is
    'One row per authenticated user. Created automatically on first sign-in via trigger.';

-- Auto-create profile on new Supabase Auth sign-up.
--
-- SECURITY DEFINER rationale: this trigger fires on `auth.users` INSERT and
-- needs to write into `public.user_profiles`, but Supabase Auth runs as the
-- `supabase_auth_admin` role which has no direct GRANT on the `public`
-- schema.  Running the function with the *definer's* privileges (the
-- migration role, which owns `public`) lets the insert succeed without
-- giving the auth role broad write access.
--
-- `set search_path = public` is the standard SECURITY DEFINER hardening:
-- it pins the schema lookup so a malicious caller cannot shadow
-- `user_profiles` with an object in a higher-priority schema.  Do not drop
-- this clause.  See migration 007 for the org-membership trigger that
-- follows the same pattern.
create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer set search_path = public
as $$
begin
    insert into public.user_profiles (id, display_name)
    values (
        new.id,
        coalesce(new.raw_user_meta_data->>'display_name', split_part(new.email, '@', 1))
    )
    on conflict (id) do nothing;
    return new;
end;
$$;

drop trigger if exists on_auth_user_created on auth.users;
create trigger on_auth_user_created
    after insert on auth.users
    for each row execute procedure public.handle_new_user();

-- updated_at trigger
create or replace function public.set_updated_at()
returns trigger language plpgsql as $$
begin new.updated_at = now(); return new; end;
$$;

drop trigger if exists set_user_profiles_updated_at on public.user_profiles;
create trigger set_user_profiles_updated_at
    before update on public.user_profiles
    for each row execute procedure public.set_updated_at();

-- ── api_keys ──────────────────────────────────────────────────────────────────
create table if not exists public.api_keys (
    id          uuid        primary key default gen_random_uuid(),
    user_id     uuid        not null references auth.users(id) on delete cascade,

    -- Human-readable label chosen by the user, e.g. "Production S3 connector"
    name        text        not null,

    -- First 8 chars of the raw key (e.g. "cg_live_"), used as a fast-lookup
    -- discriminator. Stored in plain text — not secret on its own.
    key_prefix  varchar(12) not null,

    -- bcrypt hash of the full raw key. The raw key is never persisted anywhere.
    key_hash    text        not null,

    -- Optional: restrict this key to specific contract UUIDs.
    -- NULL = unrestricted (key can access all contracts owned by user).
    allowed_contract_ids  uuid[]  default null,

    created_at  timestamptz not null default now(),
    last_used_at timestamptz,
    revoked_at  timestamptz,

    constraint api_keys_name_not_empty check (char_length(trim(name)) > 0)
);

comment on table public.api_keys is
    'Per-user API keys. Only the bcrypt hash and 8-char prefix are stored; '
    'the full key is shown to the user exactly once at creation time.';

comment on column public.api_keys.key_prefix is
    'First 8–12 chars of the raw key (not secret). Used by the Rust backend '
    'to narrow the hash-verify lookup to a single candidate row.';

comment on column public.api_keys.key_hash is
    'bcrypt hash (cost 10) of the full raw key. Verified server-side on each '
    'cache-miss. The raw key is never stored.';

comment on column public.api_keys.allowed_contract_ids is
    'NULL = key can access all contracts. Non-null = restricted to listed UUIDs.';

-- Fast lookup: Rust backend queries WHERE key_prefix = $1 AND revoked_at IS NULL
create index if not exists api_keys_prefix_active_idx
    on public.api_keys (key_prefix)
    where revoked_at is null;

-- Secondary index for user's key list page
create index if not exists api_keys_user_id_idx
    on public.api_keys (user_id, created_at desc);

-- Computed helper: is this key currently active?
-- (Used in RLS and UI — not a stored column, just a generated expression)
alter table public.api_keys
    add column if not exists is_active boolean
    generated always as (revoked_at is null) stored;

-- ── Row-level security ────────────────────────────────────────────────────────

alter table public.user_profiles enable row level security;
alter table public.api_keys      enable row level security;

-- user_profiles: users see and update only their own profile
create policy "users can read own profile"
    on public.user_profiles for select
    using (auth.uid() = id);

create policy "users can update own profile"
    on public.user_profiles for update
    using (auth.uid() = id);

-- api_keys: users can fully manage only their own keys
create policy "users can read own api keys"
    on public.api_keys for select
    using (auth.uid() = user_id);

create policy "users can create own api keys"
    on public.api_keys for insert
    with check (auth.uid() = user_id);

create policy "users can revoke own api keys"
    on public.api_keys for update
    using (auth.uid() = user_id);

-- Service role (used by Rust backend via Supabase REST) can read all keys
-- for validation purposes. This does NOT expose the raw key — only the hash.
create policy "service role can read all keys for validation"
    on public.api_keys for select
    to service_role
    using (true);

create policy "service role can update last_used_at"
    on public.api_keys for update
    to service_role
    using (true);

-- ── Immutability guard: key_hash must not change after creation ───────────────
create or replace function public.guard_api_key_hash_immutable()
returns trigger language plpgsql as $$
begin
    if new.key_hash <> old.key_hash then
        raise exception 'api_keys.key_hash is immutable after creation';
    end if;
    if new.key_prefix <> old.key_prefix then
        raise exception 'api_keys.key_prefix is immutable after creation';
    end if;
    return new;
end;
$$;

drop trigger if exists api_keys_hash_immutable on public.api_keys;
create trigger api_keys_hash_immutable
    before update on public.api_keys
    for each row execute procedure public.guard_api_key_hash_immutable();
