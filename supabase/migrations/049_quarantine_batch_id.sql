-- Migration 049: per-run id on quarantine rows
--
-- Callers may pass an optional batch id (also called a run id) on ingest and
-- egress. It is stored here so an orchestrator can count one pipeline run's
-- quarantined rows. NULL when the caller omits it, and on rows written before
-- this column existed. Replay-created rows do not copy the id.

alter table public.quarantine_events
    add column if not exists batch_id text;

do $$
begin
    if not exists (
        select 1
        from pg_constraint
        where conname = 'quarantine_events_batch_id_len'
    ) then
        alter table public.quarantine_events
            add constraint quarantine_events_batch_id_len
            check (
                batch_id is null
                or char_length(batch_id) between 1 and 255
            );
    end if;
end $$;

comment on column public.quarantine_events.batch_id is
    'Caller-supplied run id from ingest or egress (X-Batch-Id, alias run_id). NULL when omitted.';

-- One contract's rows for one run. The summary groups status and direction
-- inside that slice, so those columns ride along in the index.
create index if not exists idx_quarantine_events_contract_batch
    on public.quarantine_events (contract_id, batch_id, status, direction)
    where batch_id is not null;
