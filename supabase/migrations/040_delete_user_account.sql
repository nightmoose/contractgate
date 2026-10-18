-- 040: self-serve account deletion (tester polish round, 2026-10-14).
--
-- delete_user_account(p_user) clears everything that would block
-- auth.admin.deleteUser(p_user), which the dashboard route
-- (/api/account/delete) calls right after:
--   1. Refuses when the user is the last owner of an org that still has
--      other members (they must transfer ownership first).
--   2. Purges every org where the user is the only member: all rows in
--      public tables with an org_id column, then the org. Child-table FKs are
--      RESTRICT (migration 012), so deletes retry in passes until the
--      dependency order works out.
--   3. Clears non-cascading references to auth.users (invited_by, …):
--      nullable columns are set NULL, NOT NULL ones have their rows deleted.
-- Service role only. Needs the owner's approval before it is applied to prod.

create or replace function public.delete_user_account(p_user uuid)
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
        raise exception 'p_user is required';
    end if;

    if exists (
        select 1
        from org_memberships m
        where m.user_id = p_user
          and m.role = 'owner'
          and exists (select 1 from org_memberships x
                      where x.org_id = m.org_id and x.user_id <> p_user)
          and not exists (select 1 from org_memberships x
                          where x.org_id = m.org_id and x.user_id <> p_user
                            and x.role = 'owner')
    ) then
        raise exception 'last_owner: transfer ownership of your organization before deleting your account'
            using errcode = 'P0001';
    end if;

    for v_org in
        select m.org_id
        from org_memberships m
        where m.user_id = p_user
          and not exists (select 1 from org_memberships x
                          where x.org_id = m.org_id and x.user_id <> p_user)
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

revoke all on function public.delete_user_account(uuid) from public;
do $$
begin
    if exists (select 1 from pg_roles where rolname = 'anon') then
        execute 'revoke all on function public.delete_user_account(uuid) from anon';
    end if;
    if exists (select 1 from pg_roles where rolname = 'authenticated') then
        execute 'revoke all on function public.delete_user_account(uuid) from authenticated';
    end if;
end $$;
grant execute on function public.delete_user_account(uuid) to service_role;
