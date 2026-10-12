-- Migration 037: enforce audit/quarantine log retention by plan
--
-- Pricing has promised "7 days audit log retention" on Free and 90 on paid
-- plans, but nothing deleted anything: every validation writes a ~1 KB
-- audit_log row (plus a quarantine row on failure) forever. This makes the
-- promise true and keeps Supabase storage bounded (decided 2026-10-12).
--
--   Free (free, aisg_free)      7 days
--   Pro  (growth, aisg_pro)    90 days
--   Enterprise                365 days (raise per contract if agreed)
--
-- Rows without an org (self-hosted / legacy) are never touched. Usage
-- metering reads org_monthly_usage, not audit_log, so purging never changes
-- what a customer is billed or capped at.
--
-- Changes:
--   NEW  log_retention_days(plan)   — the table above, in one place
--   NEW  purge_expired_logs(batch)  — deletes expired rows in batches; first
--                                      clears the NO ACTION cross-references
--                                      between audit_log and quarantine_events
--   NEW  pg_cron job "purge-expired-logs", hourly — only where pg_cron exists
--        (Supabase); skipped on plain Postgres (CI, self-host).

create or replace function public.log_retention_days(p_plan text)
returns integer
language sql
immutable
as $$
    select case
        when p_plan in ('growth', 'aisg_pro') then 90
        when p_plan = 'enterprise'           then 365
        else 7
    end
$$;

create or replace function public.purge_expired_logs(p_batch integer default 20000)
returns table (audit_deleted integer, quarantine_deleted integer)
language plpgsql
security definer set search_path = public
as $$
declare
    v_days      integer;
    v_orgs      uuid[];
    v_cutoff    timestamptz;
    v_audit     uuid[];
    v_quar      uuid[];
    v_a_total   integer := 0;
    v_q_total   integer := 0;
    v_n         integer;
begin
    -- One pass per retention bucket so each query is a plain range scan on
    -- (org_id, created_at) / (created_at) indexes.
    for v_days in select distinct public.log_retention_days(plan) from public.orgs loop
        select array_agg(id) into v_orgs
        from public.orgs
        where public.log_retention_days(plan) = v_days;
        continue when v_orgs is null;

        v_cutoff := now() - make_interval(days => v_days);

        select array_agg(a.id) into v_audit
        from (
            select id from public.audit_log
            where org_id = any (v_orgs) and created_at < v_cutoff
            limit p_batch
        ) a;

        select array_agg(q.id) into v_quar
        from (
            select qe.id
            from public.quarantine_events qe
            join public.contracts c on c.id = qe.contract_id
            where c.org_id = any (v_orgs) and qe.created_at < v_cutoff
            limit p_batch
        ) q;

        -- Break references first: the FKs between the two tables are NO ACTION.
        if v_audit is not null then
            update public.quarantine_events
               set replayed_into_audit_id = null
             where replayed_into_audit_id = any (v_audit);
        end if;
        if v_quar is not null then
            update public.audit_log
               set replay_of_quarantine_id = null
             where replay_of_quarantine_id = any (v_quar);
            update public.quarantine_events
               set replay_of_quarantine_id = null
             where replay_of_quarantine_id = any (v_quar);

            delete from public.quarantine_events where id = any (v_quar);
            get diagnostics v_n = row_count;
            v_q_total := v_q_total + v_n;
        end if;
        if v_audit is not null then
            delete from public.audit_log where id = any (v_audit);
            get diagnostics v_n = row_count;
            v_a_total := v_a_total + v_n;
        end if;
    end loop;

    return query select v_a_total, v_q_total;
end;
$$;

-- Maintenance only: never callable through PostgREST.
revoke execute on function public.purge_expired_logs(integer) from public, anon, authenticated;

-- Hourly schedule where pg_cron is available (Supabase). Dynamic SQL so this
-- file still applies on plain Postgres, where the cron schema doesn't exist.
do $$
begin
    if exists (select 1 from pg_available_extensions where name = 'pg_cron') then
        execute 'create extension if not exists pg_cron';
        execute $cron$select cron.schedule('purge-expired-logs', '17 * * * *', 'select public.purge_expired_logs()')$cron$;
    else
        raise notice 'pg_cron not available: purge_expired_logs() is not scheduled here';
    end if;
end
$$;
