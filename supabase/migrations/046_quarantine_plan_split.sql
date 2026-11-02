-- Migration 046: quarantine plan split
--
-- Free and AISchemaGate Free share one pending-row cap (placeholders in
-- src/plan.rs). This migration does not enforce the cap. It:
--   * redacts leftover Free record bodies in place and keeps the rows
--   * records org_id on quarantine_events and indexes (org_id, status, created_at)
--   * starts a 30-day grace before bodies are removed on a later downgrade
--   * stores Pro bulk-replay jobs
--
-- No notice is sent. Rows are never deleted by the redaction statements.

-- ---------------------------------------------------------------------------
-- org_id on quarantine rows, for the shared org cap
-- ---------------------------------------------------------------------------

alter table public.quarantine_events
    add column if not exists org_id uuid;

update public.quarantine_events qe
set org_id = c.org_id
from public.contracts c
where qe.contract_id = c.id
  and qe.org_id is null;

create index if not exists idx_quarantine_org_status_created
    on public.quarantine_events (org_id, status, created_at);

create or replace function public.quarantine_events_fill_org_id()
returns trigger
language plpgsql
as $$
begin
    if new.org_id is null then
        select c.org_id into new.org_id
        from public.contracts c
        where c.id = new.contract_id;
    end if;
    return new;
end;
$$;

drop trigger if exists quarantine_events_fill_org_id on public.quarantine_events;
create trigger quarantine_events_fill_org_id
    before insert on public.quarantine_events
    for each row execute procedure public.quarantine_events_fill_org_id();

-- ---------------------------------------------------------------------------
-- BEGIN free body redaction
-- Quietly remove leftover Free record bodies. The org never turned storage
-- on (the column default). Self-hosted orgs that enabled storage are left
-- for the downgrade grace. Rows stay.
-- ---------------------------------------------------------------------------

update public.quarantine_events qe
set payload = null,
    payload_redacted = true
from public.contracts c
join public.orgs o on o.id = c.org_id
where qe.contract_id = c.id
  and o.plan in ('free', 'aisg_free')
  and o.store_event_payloads = false
  and qe.payload is not null
  and qe.payload_redacted = false;

update public.audit_log a
set raw_event = '{}'::jsonb,
    raw_event_redacted = true
from public.orgs o
where a.org_id = o.id
  and o.plan in ('free', 'aisg_free')
  and o.store_event_payloads = false
  and a.raw_event_redacted = false
  and a.raw_event is distinct from '{}'::jsonb;

update public.forwarded_events fe
set payload = 'null'::jsonb
from public.contracts c
join public.orgs o on o.id = c.org_id
where fe.contract_id = c.id
  and o.plan in ('free', 'aisg_free')
  and o.store_event_payloads = false
  and fe.payload is distinct from 'null'::jsonb;

-- END free body redaction

-- ---------------------------------------------------------------------------
-- Downgrade grace. The trigger schedules it. The hourly purge redacts.
-- ---------------------------------------------------------------------------

alter table public.orgs
    add column if not exists payload_redact_after timestamptz;

comment on column public.orgs.payload_redact_after is
    'After a paid-to-free downgrade, stored bodies are redacted once this time has passed. Cleared if the org is paid again before then. Rows are kept.';

-- BEGIN grace redaction
create or replace function public.orgs_schedule_payload_redaction()
returns trigger
language plpgsql
as $$
begin
    if new.plan is distinct from old.plan then
        if old.plan in ('growth', 'aisg_pro', 'enterprise')
           and new.plan in ('free', 'aisg_free') then
            new.payload_redact_after := now() + interval '30 days';
        elsif new.plan in ('growth', 'aisg_pro', 'enterprise') then
            new.payload_redact_after := null;
        end if;
    end if;
    return new;
end;
$$;

drop trigger if exists orgs_schedule_payload_redaction on public.orgs;
create trigger orgs_schedule_payload_redaction
    before update of plan on public.orgs
    for each row execute procedure public.orgs_schedule_payload_redaction();

create or replace function public.redact_elapsed_payload_grace(p_batch integer default 20000)
returns integer
language plpgsql
security definer set search_path = public
as $$
declare
    v_n integer := 0;
    v_step integer;
begin
    update public.quarantine_events qe
    set payload = null,
        payload_redacted = true
    where qe.id in (
        select qe2.id
        from public.quarantine_events qe2
        join public.contracts c on c.id = qe2.contract_id
        join public.orgs o on o.id = c.org_id
        where o.payload_redact_after is not null
          and o.payload_redact_after <= now()
          and qe2.payload is not null
          and qe2.payload_redacted = false
        limit p_batch
    );
    get diagnostics v_step = row_count;
    v_n := v_n + v_step;

    update public.audit_log a
    set raw_event = '{}'::jsonb,
        raw_event_redacted = true
    where a.id in (
        select a2.id
        from public.audit_log a2
        join public.orgs o on o.id = a2.org_id
        where o.payload_redact_after is not null
          and o.payload_redact_after <= now()
          and a2.raw_event_redacted = false
          and a2.raw_event is distinct from '{}'::jsonb
        limit p_batch
    );
    get diagnostics v_step = row_count;
    v_n := v_n + v_step;

    update public.forwarded_events fe
    set payload = 'null'::jsonb
    where fe.id in (
        select fe2.id
        from public.forwarded_events fe2
        join public.contracts c on c.id = fe2.contract_id
        join public.orgs o on o.id = c.org_id
        where o.payload_redact_after is not null
          and o.payload_redact_after <= now()
          and fe2.payload is distinct from 'null'::jsonb
        limit p_batch
    );
    get diagnostics v_step = row_count;
    v_n := v_n + v_step;

    update public.orgs o
    set payload_redact_after = null
    where o.payload_redact_after is not null
      and o.payload_redact_after <= now()
      and not exists (
          select 1
          from public.quarantine_events qe
          join public.contracts c on c.id = qe.contract_id
          where c.org_id = o.id
            and qe.payload is not null
            and qe.payload_redacted = false
      )
      and not exists (
          select 1
          from public.audit_log a
          where a.org_id = o.id
            and a.raw_event_redacted = false
            and a.raw_event is distinct from '{}'::jsonb
      )
      and not exists (
          select 1
          from public.forwarded_events fe
          join public.contracts c on c.id = fe.contract_id
          where c.org_id = o.id
            and fe.payload is distinct from 'null'::jsonb
      );

    return v_n;
end;
$$;

revoke execute on function public.redact_elapsed_payload_grace(integer) from public, anon, authenticated;
-- END grace redaction

create or replace function public.purge_expired_logs(p_batch integer default 20000)
returns table (audit_deleted integer, quarantine_deleted integer, drops_deleted integer)
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
    v_d_total   integer := 0;
    v_n         integer;
begin
    perform public.redact_elapsed_payload_grace(p_batch);

    delete from public.dropped_event_counts
    where ctid in (
        select ctid from public.dropped_event_counts
        where bucket_start < now() - interval '30 days'
        limit p_batch
    );
    get diagnostics v_n = row_count;
    v_d_total := v_d_total + v_n;

    delete from public.dropped_event_samples
    where ctid in (
        select ctid from public.dropped_event_samples
        where bucket_start < now() - interval '30 days'
        limit p_batch
    );
    get diagnostics v_n = row_count;
    v_d_total := v_d_total + v_n;

    delete from public.quarantine_guard_events
    where ctid in (
        select ctid from public.quarantine_guard_events
        where observed_at < now() - interval '30 days'
        limit p_batch
    );
    get diagnostics v_n = row_count;
    v_d_total := v_d_total + v_n;

    -- The cap only reads the last hour. Keep a day of buckets so a late
    -- purge cannot strand the current hour.
    delete from public.quarantine_volume_minute
    where bucket_start < now() - interval '26 hours';

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

    return query select v_a_total, v_q_total, v_d_total;
end;
$$;

revoke execute on function public.purge_expired_logs(integer) from public, anon, authenticated;


-- ---------------------------------------------------------------------------
-- Pro bulk-replay jobs. The gateway writes them; members can read status.
-- ---------------------------------------------------------------------------

create table if not exists public.quarantine_replay_jobs (
    id uuid primary key default gen_random_uuid(),
    org_id uuid not null references public.orgs(id) on delete cascade,
    contract_id uuid not null references public.contracts(id) on delete cascade,
    target_version text,
    filter_version text,
    filter_reason text,
    status text not null default 'queued'
        check (status in ('queued', 'running', 'completed', 'failed')),
    record_limit integer not null,
    selected integer not null default 0,
    replayed integer not null default 0,
    failed integer not null default 0,
    skipped integer not null default 0,
    error text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists quarantine_replay_jobs_contract_idx
    on public.quarantine_replay_jobs (contract_id, created_at desc);

comment on table public.quarantine_replay_jobs is
    'Background replay of matching quarantined records. Pro, AISchemaGate Pro, Enterprise, and self-host. record_limit is the per-job ceiling.';

alter table public.quarantine_replay_jobs enable row level security;

drop policy if exists "org members can read replay jobs" on public.quarantine_replay_jobs;
create policy "org members can read replay jobs"
    on public.quarantine_replay_jobs for select
    to authenticated
    using (org_id in (select public.get_my_org_ids()));

