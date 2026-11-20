-- Migration 054: downstream lineage and impact reports.
--
-- MUST be applied to production before the gateway build that serves this
-- feature is promoted. The gateway still boots if this file has not been
-- applied: lineage tables are not in the startup schema check. Version
-- create and publish skip the report, and the impact routes return 503
-- for an entitled org until the tables exist. Free orgs are rejected
-- with 402 before those tables are read.
--
-- Adds the stored plan value `impact` (Pro + Impact). It is a superset of
-- Pro: the same event cap, rate limit, 90-day retention, and paid gates,
-- plus downstream impact analysis. Pricing pages are unchanged. Which
-- plans may use impact analysis, and the asset and edge caps, live in
-- src/plan.rs so they can be edited without another migration.
--
-- Graph direction: an edge points downstream (upstream asset feeds the
-- downstream asset). Impact is a recursive walk along those edges.
-- Service role / table owner bypasses RLS. anon and authenticated have
-- no policy, so the dashboard cannot read lineage around the plan gate.

-- Fail fast instead of queueing behind a long transaction on orgs. The
-- constraint swap below needs a brief ACCESS EXCLUSIVE lock on orgs (one row
-- per organization), the same swap migrations 026 and 036 made.
select set_config('lock_timeout', '5s', true);

alter table public.orgs drop constraint if exists orgs_plan_valid;
alter table public.orgs
    add constraint orgs_plan_valid
    check (plan in ('free', 'growth', 'enterprise', 'aisg_free', 'aisg_pro', 'impact'));

comment on column public.orgs.plan is
    'Billing plan: free | growth | enterprise | aisg_free | aisg_pro | impact. '
    'impact is Pro + Impact: every Pro gate, plus downstream impact analysis. '
    'Feature membership and caps are constants in the gateway, not in this check.';

create table if not exists public.lineage_assets (
    id           uuid        primary key default gen_random_uuid(),
    org_id       uuid        not null references public.orgs(id) on delete cascade,
    kind         text        not null
        check (kind in ('contract', 'dataset', 'table', 'model', 'dashboard', 'consumer', 'job')),
    external_key text        not null,
    name         text        not null,
    owner        text,
    contract_id  uuid        references public.contracts(id) on delete cascade,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),
    constraint lineage_assets_org_key_unique unique (org_id, external_key),
    constraint lineage_assets_name_not_empty check (char_length(trim(name)) > 0),
    constraint lineage_assets_key_not_empty check (char_length(trim(external_key)) > 0)
);

comment on table public.lineage_assets is
    'Downstream lineage nodes for one org: contracts, datasets, tables, models, dashboards, consumers, and jobs.';

create index if not exists lineage_assets_org_idx
    on public.lineage_assets (org_id);

create index if not exists lineage_assets_contract_idx
    on public.lineage_assets (org_id, contract_id)
    where contract_id is not null;

create table if not exists public.lineage_edges (
    id            uuid        primary key default gen_random_uuid(),
    org_id        uuid        not null references public.orgs(id) on delete cascade,
    from_asset_id uuid        not null references public.lineage_assets(id) on delete cascade,
    to_asset_id   uuid        not null references public.lineage_assets(id) on delete cascade,
    edge_kind     text        not null
        check (edge_kind in ('feeds', 'displays', 'refreshes', 'owned_by')),
    field_name    text,
    created_at    timestamptz not null default now(),
    constraint lineage_edges_no_self check (from_asset_id <> to_asset_id)
);

comment on table public.lineage_edges is
    'Directed lineage. from_asset_id is upstream and to_asset_id is downstream. field_name, when set, limits the edge to one contract field.';

create unique index if not exists lineage_edges_unique
    on public.lineage_edges (
        org_id,
        from_asset_id,
        to_asset_id,
        edge_kind,
        coalesce(field_name, '')
    );

create index if not exists lineage_edges_from_idx
    on public.lineage_edges (org_id, from_asset_id);

-- Serves the ON DELETE CASCADE from lineage_assets on the downstream end.
create index if not exists lineage_edges_to_idx
    on public.lineage_edges (to_asset_id);

create table if not exists public.contract_impact_reports (
    id              uuid        primary key default gen_random_uuid(),
    org_id          uuid        not null references public.orgs(id) on delete cascade,
    contract_id     uuid        not null references public.contracts(id) on delete cascade,
    version_id      uuid        not null references public.contract_versions(id) on delete cascade,
    version         text        not null,
    base_version    text,
    trigger         text        not null
        check (trigger in ('version_created', 'version_published', 'recompute')),
    breaking_count  integer     not null,
    safe_count      integer     not null,
    affected_count  integer     not null,
    summary         text        not null,
    report          jsonb       not null,
    created_at      timestamptz not null default now(),
    constraint contract_impact_reports_unique unique (contract_id, version, trigger)
);

comment on table public.contract_impact_reports is
    'Stored downstream impact for one contract version. Written when a version is proposed, when it is published, and when a caller recomputes.';

create index if not exists contract_impact_reports_lookup_idx
    on public.contract_impact_reports (contract_id, version, created_at desc);

-- Serve the ON DELETE CASCADE from contract_versions and orgs.
create index if not exists contract_impact_reports_version_idx
    on public.contract_impact_reports (version_id);

create index if not exists contract_impact_reports_org_idx
    on public.contract_impact_reports (org_id);

alter table public.lineage_assets enable row level security;
alter table public.lineage_edges enable row level security;
alter table public.contract_impact_reports enable row level security;

-- Pro + Impact uses the Pro retention window. Migration 037 left `impact`
-- on the Free default (7 days) because the plan value did not exist yet.
create or replace function public.log_retention_days(p_plan text)
returns integer
language sql
immutable
as $$
    select case
        when p_plan in ('growth', 'aisg_pro', 'impact') then 90
        when p_plan = 'enterprise' then 365
        else 7
    end
$$;

-- Leaving Pro + Impact for Free schedules the same 30-day body grace as
-- leaving Pro. Returning to Pro + Impact clears a pending redaction.
create or replace function public.orgs_schedule_payload_redaction()
returns trigger
language plpgsql
as $$
begin
    if new.plan is distinct from old.plan then
        if old.plan in ('growth', 'aisg_pro', 'enterprise', 'impact')
           and new.plan in ('free', 'aisg_free') then
            new.payload_redact_after := now() + interval '30 days';
        elsif new.plan in ('growth', 'aisg_pro', 'enterprise', 'impact') then
            new.payload_redact_after := null;
        end if;
    end if;
    return new;
end;
$$;
