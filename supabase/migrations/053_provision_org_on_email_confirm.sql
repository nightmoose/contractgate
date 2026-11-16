-- Migration 053: Provision an org only after email confirmation
--
-- handle_new_user() used to insert an org + owner membership on every
-- auth.users insert, including unconfirmed email signups. When those
-- unconfirmed users were later purged, org_memberships cascaded away and
-- the orgs stayed, inflating the org count (354 leftover bot orgs cleaned
-- 2026-11-16, backed up in schema cleanup_20261116).
--
-- Changes:
--   1. handle_new_user() still writes user_profiles on insert, but only
--      provisions an org when email_confirmed_at is already set (GitHub
--      OAuth and any autoconfirm path).
--   2. A new AFTER UPDATE trigger provisions the org when the email is
--      confirmed, with a first-sign-in fallback if confirm already happened
--      and no org exists yet. Idempotent if the user already has a membership.
--   3. A BEFORE DELETE trigger on auth.users deletes any org for which that
--      user is the only member, so a later unconfirmed-user purge cannot
--      leave another empty org. Shared helper used by delete_user_account().
--
-- Existing confirmed users and their orgs are untouched. Trigger-only:
-- execute is revoked from public, anon, and authenticated.

-- Local/CI stub (migration 006) omits columns that exist on hosted
-- auth.users. Add any column this migration reads so compose and self-host
-- apply cleanly. No-op on hosted Supabase, where the column already exists.
alter table auth.users
    add column if not exists email_confirmed_at timestamptz;


create or replace function public.provision_org_for_confirmed_user(
    p_id uuid,
    p_email text,
    p_meta jsonb
)
returns uuid
language plpgsql
security definer
set search_path = public
as $$
declare
    v_org_id uuid;
    v_slug   text;
    v_name   text;
    v_plan   text;
begin
    if p_id is null then
        raise exception 'p_id is required';
    end if;

    -- Already provisioned (re-confirm, first sign-in fallback, retries).
    select m.org_id
      into v_org_id
      from public.org_memberships m
     where m.user_id = p_id
       and m.deleted_at is null
     order by m.joined_at
     limit 1;

    if v_org_id is not null then
        return v_org_id;
    end if;

    v_plan := case
        when p_meta->>'product' = 'aischemagate' then 'aisg_free'
        else 'free'
    end;

    v_name := coalesce(
        p_meta->>'display_name',
        split_part(coalesce(p_email, ''), '@', 1),
        'user'
    );
    if v_name = '' then
        v_name := 'user';
    end if;

    v_slug := lower(regexp_replace(v_name, '[^a-zA-Z0-9]+', '-', 'g'));
    v_slug := trim(both '-' from v_slug);
    if v_slug = '' then
        v_slug := 'org';
    end if;

    begin
        insert into public.orgs (name, slug, plan)
        values (v_name, v_slug, v_plan)
        returning id into v_org_id;
    exception when unique_violation then
        insert into public.orgs (name, slug, plan)
        values (
            v_name,
            v_slug || '-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 8),
            v_plan
        )
        returning id into v_org_id;
    end;

    insert into public.org_memberships (org_id, user_id, role)
    values (v_org_id, p_id, 'owner');

    return v_org_id;
end;
$$;

create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
    insert into public.user_profiles (id, display_name)
    values (
        new.id,
        coalesce(new.raw_user_meta_data->>'display_name', split_part(new.email, '@', 1))
    )
    on conflict (id) do nothing;

    -- Email/password signups arrive unconfirmed. Wait for the confirm
    -- trigger. GitHub OAuth and autoconfirm arrive with email_confirmed_at set.
    if new.email_confirmed_at is null then
        return new;
    end if;

    perform public.provision_org_for_confirmed_user(
        new.id, new.email, new.raw_user_meta_data
    );
    return new;
end;
$$;

create or replace function public.handle_user_email_confirmed()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
    if new.email_confirmed_at is null then
        return new;
    end if;

    -- Confirm just happened, or first sign-in after a confirm that somehow
    -- missed org provisioning.
    if (old.email_confirmed_at is null and new.email_confirmed_at is not null)
       or (old.last_sign_in_at is null and new.last_sign_in_at is not null)
    then
        perform public.provision_org_for_confirmed_user(
            new.id, new.email, new.raw_user_meta_data
        );
    end if;

    return new;
end;
$$;

-- Delete every org for which p_user is the only member, walking child
-- tables in passes because several org_id FKs are ON DELETE RESTRICT.
create or replace function public.delete_solo_orgs_for_user(p_user uuid)
returns void
language plpgsql
security definer
set search_path = public, pg_temp
as $$
declare
    v_org     uuid;
    r         record;
    v_pending int;
    v_pass    int;
begin
    if p_user is null then
        return;
    end if;

    for v_org in
        select m.org_id
        from public.org_memberships m
        where m.user_id = p_user
          and (m.deleted_at is null)
          and not exists (
              select 1
              from public.org_memberships x
              where x.org_id = m.org_id
                and x.user_id <> p_user
                and x.deleted_at is null
          )
    loop
        for v_pass in 1..10 loop
            v_pending := 0;
            for r in
                select c.table_schema, c.table_name
                from information_schema.columns c
                join information_schema.tables t
                  on t.table_schema = c.table_schema and t.table_name = c.table_name
                where c.table_schema = 'public'
                  and c.column_name = 'org_id'
                  and c.data_type = 'uuid'
                  and t.table_type = 'BASE TABLE'
                  and c.table_name <> 'orgs'
            loop
                begin
                    execute format('delete from %I.%I where org_id = $1',
                                   r.table_schema, r.table_name) using v_org;
                exception when foreign_key_violation then
                    v_pending := v_pending + 1;
                end;
            end loop;
            exit when v_pending = 0;
        end loop;
        delete from public.orgs where id = v_org;
    end loop;
end;
$$;

create or replace function public.cleanup_orgs_on_user_delete()
returns trigger
language plpgsql
security definer
set search_path = public, pg_temp
as $$
begin
    perform public.delete_solo_orgs_for_user(old.id);
    return old;
end;
$$;

-- delete_user_account keeps the last-owner guard and the non-cascading
-- auth.users FK cleanup, but delegates solo-org deletion to the helper
-- so self-serve delete and unconfirmed-user purge share one path.
create or replace function public.delete_user_account(p_user uuid)
returns void
language plpgsql
security definer
set search_path = public, pg_temp
as $$
declare
    r record;
begin
    if p_user is null then
        raise exception 'p_user is required';
    end if;

    if exists (
        select 1
        from org_memberships m
        where m.user_id = p_user
          and m.role = 'owner'
          and exists (select 1 from org_memberships x
                      where x.org_id = m.org_id and x.user_id <> p_user
                        and x.deleted_at is null)
          and not exists (select 1 from org_memberships x
                          where x.org_id = m.org_id and x.user_id <> p_user
                            and x.role = 'owner'
                            and x.deleted_at is null)
    ) then
        raise exception 'last_owner: transfer ownership of your organization before deleting your account'
            using errcode = 'P0001';
    end if;

    perform public.delete_solo_orgs_for_user(p_user);

    for r in
        select cl.relnamespace::regnamespace::text as sch,
               cl.relname                           as tbl,
               a.attname                            as col,
               a.attnotnull                         as notnull
        from pg_constraint k
        join pg_class cl     on cl.oid = k.conrelid
        join pg_attribute a  on a.attrelid = k.conrelid and a.attnum = k.conkey[1]
        where k.contype = 'f'
          and k.confrelid = 'auth.users'::regclass
          and k.confdeltype <> 'c'
          and array_length(k.conkey, 1) = 1
    loop
        if r.notnull then
            execute format('delete from %I.%I where %I = $1', r.sch, r.tbl, r.col) using p_user;
        else
            execute format('update %I.%I set %I = null where %I = $1', r.sch, r.tbl, r.col, r.col)
                using p_user;
        end if;
    end loop;
end;
$$;

drop trigger if exists on_auth_user_created on auth.users;
create trigger on_auth_user_created
    after insert on auth.users
    for each row execute function public.handle_new_user();

drop trigger if exists on_auth_user_email_confirmed on auth.users;
create trigger on_auth_user_email_confirmed
    after update of email_confirmed_at, last_sign_in_at on auth.users
    for each row execute function public.handle_user_email_confirmed();

drop trigger if exists on_auth_user_deleted_cleanup_orgs on auth.users;
create trigger on_auth_user_deleted_cleanup_orgs
    before delete on auth.users
    for each row execute function public.cleanup_orgs_on_user_delete();

revoke all on function public.provision_org_for_confirmed_user(uuid, text, jsonb) from public;
revoke all on function public.handle_new_user() from public;
revoke all on function public.handle_user_email_confirmed() from public;
revoke all on function public.delete_solo_orgs_for_user(uuid) from public;
revoke all on function public.cleanup_orgs_on_user_delete() from public;
revoke all on function public.delete_user_account(uuid) from public;

do $$
begin
    if exists (select 1 from pg_roles where rolname = 'anon') then
        execute 'revoke all on function public.provision_org_for_confirmed_user(uuid, text, jsonb) from anon';
        execute 'revoke all on function public.handle_new_user() from anon';
        execute 'revoke all on function public.handle_user_email_confirmed() from anon';
        execute 'revoke all on function public.delete_solo_orgs_for_user(uuid) from anon';
        execute 'revoke all on function public.cleanup_orgs_on_user_delete() from anon';
        execute 'revoke all on function public.delete_user_account(uuid) from anon';
    end if;
    if exists (select 1 from pg_roles where rolname = 'authenticated') then
        execute 'revoke all on function public.provision_org_for_confirmed_user(uuid, text, jsonb) from authenticated';
        execute 'revoke all on function public.handle_new_user() from authenticated';
        execute 'revoke all on function public.handle_user_email_confirmed() from authenticated';
        execute 'revoke all on function public.delete_solo_orgs_for_user(uuid) from authenticated';
        execute 'revoke all on function public.cleanup_orgs_on_user_delete() from authenticated';
        execute 'revoke all on function public.delete_user_account(uuid) from authenticated';
    end if;
end $$;

grant execute on function public.delete_user_account(uuid) to service_role;
grant execute on function public.delete_solo_orgs_for_user(uuid) to service_role;
-- The other functions are trigger-only; no grant to client roles or service_role.
