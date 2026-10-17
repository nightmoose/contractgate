-- Local self-host roles. Applied only on first boot of the self-host
-- compose file, before GoTrue creates the auth schema. The authenticator
-- password is a local placeholder. Change it before exposing this database.

DO $$
BEGIN
    -- GoTrue's later migrations GRANT to the role `postgres`, which exists on
    -- the hosted image and on a default Postgres cluster. This compose
    -- database's superuser is `cg`.
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        CREATE ROLE postgres NOLOGIN SUPERUSER;
    END IF;
    -- GoTrue creates unqualified enum types. Its search_path must list auth
    -- first or a later migration looks up auth.factor_type and fails.
    -- Password is a local placeholder.
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'supabase_auth_admin') THEN
        CREATE ROLE supabase_auth_admin LOGIN PASSWORD 'supabase_auth_admin' SUPERUSER;
    END IF;
    ALTER ROLE supabase_auth_admin SET search_path TO auth, public;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'anon') THEN
        CREATE ROLE anon NOLOGIN NOINHERIT;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
        CREATE ROLE authenticated NOLOGIN NOINHERIT;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'service_role') THEN
        CREATE ROLE service_role NOLOGIN NOINHERIT BYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticator') THEN
        CREATE ROLE authenticator LOGIN PASSWORD 'authenticator' NOINHERIT;
    END IF;
END $$;

GRANT anon, authenticated, service_role TO authenticator;
GRANT anon, authenticated, service_role TO cg;

-- GoTrue's first migration creates auth.users. It does not create the schema.
-- Hosted Supabase already has it. This database does not.
CREATE SCHEMA IF NOT EXISTS auth;
GRANT USAGE ON SCHEMA auth TO anon, authenticated, service_role;
