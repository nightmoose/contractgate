-- Migration 045: drop mode for failing records.
--
-- Quarantine stays the default. A contract may instead count a failed record
-- and store nothing (no quarantine row, no audit row). A safety cap can
-- record that quarantine volume crossed a caller-set threshold. It does not
-- switch modes until enforcement is enabled. Drop counts are minute buckets
-- by failure reason and are kept for 30 days.

alter table public.contract_versions
    add column if not exists on_fail text not null default 'quarantine'
        check (on_fail in ('quarantine', 'drop')),
    add column if not exists on_fail_locked boolean not null default false;

comment on column public.contract_versions.on_fail is
    'Denormalized from the version document. quarantine (default) or drop. '
    'The document is authoritative; a live override may sit on contracts.fail_policy.';

comment on column public.contract_versions.on_fail_locked is
    'When true, a request cannot override this version''s on_fail.';

alter table public.contracts
    add column if not exists fail_policy jsonb;

comment on column public.contracts.fail_policy is
    'Live on-fail override. Null means the resolved version document applies. '
    'Shape: {"on_fail":"quarantine"|"drop","on_fail_locked":bool,"quarantine_guard":{...}}.';

alter table public.orgs
    add column if not exists quarantine_guard jsonb;

comment on column public.orgs.quarantine_guard is
    'Account safety cap. When it has a threshold it replaces the contract cap. '
    'enforce defaults to false (watch only). Thresholds are unset until configured.';

-- Minute buckets of dropped failures. reason_key is a violation kind and field
-- path, never a record value. org_id is null for traffic with no org.
create table if not exists public.dropped_event_counts (
    org_id            uuid,
    contract_id       uuid not null references public.contracts(id) on delete cascade,
    contract_version  text not null,
    direction         text not null check (direction in ('ingress', 'egress')),
    bucket_start      timestamptz not null,
    source            text not null check (source in ('request', 'contract', 'default', 'auto_trip')),
    reason_key        text not null,
    dropped           bigint not null default 0 check (dropped >= 0),
    constraint dropped_event_counts_bucket unique nulls not distinct (
        org_id, contract_id, contract_version, direction, bucket_start, source, reason_key
    )
);

create index if not exists dropped_event_counts_bucket_idx
    on public.dropped_event_counts (bucket_start);

create index if not exists dropped_event_counts_contract_idx
    on public.dropped_event_counts (contract_id, bucket_start desc);

comment on table public.dropped_event_counts is
    'Count-only ledger of dropped failures. One row per minute, reason, and source. Kept 30 days.';

-- Opt-in, at most 5 metadata samples per contract version, direction, and minute.
-- metadata is violation kind and field path only.
create table if not exists public.dropped_event_samples (
    org_id            uuid,
    contract_id       uuid not null references public.contracts(id) on delete cascade,
    contract_version  text not null,
    direction         text not null check (direction in ('ingress', 'egress')),
    bucket_start      timestamptz not null,
    sample_idx        smallint not null check (sample_idx >= 0 and sample_idx < 5),
    reason_key        text not null,
    metadata          jsonb not null,
    primary key (contract_id, contract_version, direction, bucket_start, sample_idx)
);

comment on table public.dropped_event_samples is
    'Optional samples of dropped failures. Failure metadata only; never record contents. Kept 30 days.';

create index if not exists dropped_event_samples_bucket_idx
    on public.dropped_event_samples (bucket_start);

-- Quarantine inserts per minute, so the safety cap does not scan quarantine_events.
create table if not exists public.quarantine_volume_minute (
    org_id        uuid,
    contract_id   uuid not null references public.contracts(id) on delete cascade,
    bucket_start  timestamptz not null,
    quarantined   bigint not null default 0 check (quarantined >= 0),
    primary key (contract_id, bucket_start)
);

create index if not exists quarantine_volume_minute_org_idx
    on public.quarantine_volume_minute (org_id, bucket_start);

comment on table public.quarantine_volume_minute is
    'Per-minute quarantine insert counts used by the safety cap. Older than 26 hours are purged.';

-- contract_id null is the account-wide cap. It overrides a contract cap.
create table if not exists public.quarantine_guard_state (
    org_id      uuid not null references public.orgs(id) on delete cascade,
    contract_id uuid references public.contracts(id) on delete cascade,
    tripped_at  timestamptz,
    trip_until  timestamptz,
    reason      text,
    trip_count  integer not null default 0,
    constraint quarantine_guard_state_scope unique nulls not distinct (org_id, contract_id)
);

comment on table public.quarantine_guard_state is
    'Enforcing safety-cap trip. Cleared when trip_until passes or an owner resets it. '
    'Watch-only caps do not write a trip here.';

create table if not exists public.quarantine_guard_events (
    id           uuid primary key default gen_random_uuid(),
    org_id       uuid not null,
    contract_id  uuid,
    observed_at  timestamptz not null default now(),
    kind         text not null check (kind in ('would_trip', 'tripped', 'reset')),
    scope        text not null check (scope in ('account', 'contract')),
    window_name  text,
    observed     bigint,
    threshold    bigint
);

create index if not exists quarantine_guard_events_observed_idx
    on public.quarantine_guard_events (observed_at);

create index if not exists quarantine_guard_events_scope_idx
    on public.quarantine_guard_events (org_id, contract_id, observed_at desc);

comment on table public.quarantine_guard_events is
    'Would-trip, trip, and reset events for the safety cap. Kept 30 days.';

-- RLS mirrors quarantine_events: org members can read, the gateway role writes.
alter table public.dropped_event_counts enable row level security;
alter table public.dropped_event_samples enable row level security;
alter table public.quarantine_volume_minute enable row level security;
alter table public.quarantine_guard_state enable row level security;
alter table public.quarantine_guard_events enable row level security;

create policy "org members can read dropped event counts"
    on public.dropped_event_counts for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can read dropped event samples"
    on public.dropped_event_samples for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can read quarantine volume"
    on public.quarantine_volume_minute for select
    to authenticated
    using (
        contract_id in (
            select id from public.contracts
            where org_id in (select public.get_my_org_ids())
              and deleted_at is null
        )
    );

create policy "org members can read quarantine guard state"
    on public.quarantine_guard_state for select
    to authenticated
    using (org_id in (select public.get_my_org_ids()));

create policy "org members can read quarantine guard events"
    on public.quarantine_guard_events for select
    to authenticated
    using (org_id in (select public.get_my_org_ids()));

-- Extend log purge. Drop buckets are 30 days for every plan, not the audit window.
-- Return type gains drops_deleted, so the previous function has to be replaced.
drop function if exists public.purge_expired_logs(integer);

create function public.purge_expired_logs(p_batch integer default 20000)
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

-- Rollback (apply in this order, in one transaction):
--   1. Restore purge_expired_logs(integer) from migration 037. It returns
--      (audit_deleted, quarantine_deleted) and does not read the tables below.
--      pg_cron calls the function by name, so replace it before dropping tables.
--   2. Drop the new tables: dropped_event_counts, dropped_event_samples,
--      quarantine_volume_minute, quarantine_guard_state, quarantine_guard_events.
--   3. Alter contract_versions drop column on_fail, drop column on_fail_locked.
--   4. Alter contracts drop column fail_policy.
--   5. Alter orgs drop column quarantine_guard.
-- Existing audit_log and quarantine_events rows are not rewritten or deleted
-- by this migration. New columns are nullable or constant-default, so applying
-- it does not rewrite those tables.
