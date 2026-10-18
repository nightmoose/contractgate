-- Migration 042: key collaboration tables by contract id, not contract name.
--
-- contract_collaborators, contract_comments and contract_change_proposals were
-- keyed by contracts.name (021). Names are only unique per org (and not even
-- that, today), so two orgs with the same contract name shared grants,
-- comments and proposals. This adds contract_id (FK contracts(id), ON DELETE
-- CASCADE), backfills it, makes it NOT NULL, re-keys the grant table on
-- (contract_id, org_id) and moves the RLS policies to ids.
--
-- Backfill (only live, non-deleted contracts are candidates):
--   grants     -> the contract named contract_name owned by granted_by
--   comments   -> the contract named contract_name that the commenter's org
--                 owns or holds a (backfilled) grant on
--   proposals  -> same, using proposed_by
-- Rows with zero or more than one candidate are DELETED (ambiguous or
-- orphaned). There are no real customers yet; run the counts below on prod
-- before applying to see what would go.
--
--   select count(*) from contract_collaborators cc where (select count(*)
--     from contracts c where c.name = cc.contract_name
--      and c.org_id = cc.granted_by and c.deleted_at is null) <> 1;
--
-- contract_name stays as a denormalised label (nullable now); nothing keys
-- on it any more.
--
-- RLS: cross-table lookups go through SECURITY DEFINER helpers so no policy
-- reads a table whose policy reads it back (AGENTS.md §8, migration 039).

begin;

-- ── 1. columns ───────────────────────────────────────────────────────────────
alter table public.contract_collaborators
    add column if not exists contract_id uuid references public.contracts(id) on delete cascade;
alter table public.contract_comments
    add column if not exists contract_id uuid references public.contracts(id) on delete cascade;
alter table public.contract_change_proposals
    add column if not exists contract_id uuid references public.contracts(id) on delete cascade;

-- ── 2. backfill ──────────────────────────────────────────────────────────────
update public.contract_collaborators cc
set    contract_id = (
         select c.id from public.contracts c
         where  c.name = cc.contract_name and c.org_id = cc.granted_by
           and  c.deleted_at is null)
where  cc.contract_id is null
  and  (select count(*) from public.contracts c
        where  c.name = cc.contract_name and c.org_id = cc.granted_by
          and  c.deleted_at is null) = 1;

delete from public.contract_collaborators where contract_id is null;

update public.contract_comments x
set    contract_id = (
         select c.id from public.contracts c
         where  c.name = x.contract_name and c.deleted_at is null
           and  (c.org_id = x.org_id or exists (
                   select 1 from public.contract_collaborators cc
                   where cc.contract_id = c.id and cc.org_id = x.org_id)))
where  x.contract_id is null
  and  (select count(*) from public.contracts c
        where  c.name = x.contract_name and c.deleted_at is null
          and  (c.org_id = x.org_id or exists (
                  select 1 from public.contract_collaborators cc
                  where cc.contract_id = c.id and cc.org_id = x.org_id))) = 1;

delete from public.contract_comments where contract_id is null;

update public.contract_change_proposals x
set    contract_id = (
         select c.id from public.contracts c
         where  c.name = x.contract_name and c.deleted_at is null
           and  (c.org_id = x.proposed_by or exists (
                   select 1 from public.contract_collaborators cc
                   where cc.contract_id = c.id and cc.org_id = x.proposed_by)))
where  x.contract_id is null
  and  (select count(*) from public.contracts c
        where  c.name = x.contract_name and c.deleted_at is null
          and  (c.org_id = x.proposed_by or exists (
                  select 1 from public.contract_collaborators cc
                  where cc.contract_id = c.id and cc.org_id = x.proposed_by))) = 1;

delete from public.contract_change_proposals where contract_id is null;

-- ── 3. NOT NULL, keys, indexes ───────────────────────────────────────────────
alter table public.contract_collaborators    alter column contract_id set not null;
alter table public.contract_comments         alter column contract_id set not null;
alter table public.contract_change_proposals alter column contract_id set not null;

alter table public.contract_collaborators drop constraint if exists contract_collaborators_pkey;
alter table public.contract_collaborators add  constraint contract_collaborators_pkey
    primary key (contract_id, org_id);

alter table public.contract_collaborators    alter column contract_name drop not null;
alter table public.contract_comments         alter column contract_name drop not null;
alter table public.contract_change_proposals alter column contract_name drop not null;

drop index if exists public.idx_collabs_contract_name;
drop index if exists public.idx_comments_contract_name;
drop index if exists public.idx_proposals_contract_status;

create index if not exists idx_collabs_org_id
    on public.contract_collaborators (org_id);
create index if not exists idx_comments_contract_id
    on public.contract_comments (contract_id, created_at desc);
create index if not exists idx_proposals_contract_id_status
    on public.contract_change_proposals (contract_id, status, created_at desc);

-- ── 4. RLS helpers (SECURITY DEFINER: read without applying RLS) ────────────
create or replace function public.get_my_org_contract_ids()
returns setof uuid
language sql security definer stable
set search_path = public
as $$
    select c.id from public.contracts c
    where  c.org_id in (select public.get_my_org_ids())
      and  c.deleted_at is null;
$$;

create or replace function public.get_my_collab_contract_ids()
returns setof uuid
language sql security definer stable
set search_path = public
as $$
    select cc.contract_id from public.contract_collaborators cc
    where  cc.org_id in (select public.get_my_org_ids());
$$;

revoke execute on function public.get_my_org_contract_ids()    from public, anon;
revoke execute on function public.get_my_collab_contract_ids() from public, anon;
grant  execute on function public.get_my_org_contract_ids()    to authenticated;
grant  execute on function public.get_my_collab_contract_ids() to authenticated;

-- ── 5. policies by id ────────────────────────────────────────────────────────
drop policy if exists "auth_read" on public.contract_collaborators;
create policy "auth_read" on public.contract_collaborators
    for select to authenticated
    using (
        org_id = any (select public.get_my_org_ids())
        or contract_id in (select public.get_my_org_contract_ids())
    );

drop policy if exists "auth_read" on public.contract_comments;
create policy "auth_read" on public.contract_comments
    for select to authenticated
    using (
        contract_id in (select public.get_my_org_contract_ids())
        or contract_id in (select public.get_my_collab_contract_ids())
    );

drop policy if exists "auth_read" on public.contract_change_proposals;
create policy "auth_read" on public.contract_change_proposals
    for select to authenticated
    using (
        contract_id in (select public.get_my_org_contract_ids())
        or contract_id in (select public.get_my_collab_contract_ids())
    );

drop policy if exists "org members and collaborators can read contracts" on public.contracts;
create policy "org members and collaborators can read contracts"
    on public.contracts for select to authenticated
    using (
        deleted_at is null
        and (
            org_id = any (select public.get_my_org_ids())
            or id in (select public.get_my_collab_contract_ids())
        )
    );

commit;
