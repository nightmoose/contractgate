-- 041: contract_publications.org_id referenced auth.users(id) (migration 020
-- predates org tenancy, when org_id == user_id). Since RFC-001 org_id is an
-- orgs.id, so every publish failed with 23503 contract_publications_org_id_fkey.
-- Also the "auth_own_org" RLS policy compared org_id to auth.uid().
--
-- Depends on: none of 040 (numbered after it only; apply 040 first or
-- renumber if 040 is dropped).
--
-- Orphan check before applying (rows whose org_id is not an org):
--   select count(*) from public.contract_publications p
--   where p.org_id is not null
--     and not exists (select 1 from public.orgs o where o.id = p.org_id);
--
-- Repair: orphan rows take the org of their contract. Anything still
-- orphaned keeps the FK NOT VALID (new rows are enforced) and is reported.

alter table public.contract_publications
    drop constraint if exists contract_publications_org_id_fkey;

update public.contract_publications p
set    org_id = c.org_id
from   public.contracts c
where  c.id = p.contract_id
  and  (p.org_id is null
        or not exists (select 1 from public.orgs o where o.id = p.org_id));

alter table public.contract_publications
    add constraint contract_publications_org_id_fkey
    foreign key (org_id) references public.orgs(id) on delete cascade not valid;

do $$
declare
    v_orphans bigint;
begin
    select count(*) into v_orphans
    from public.contract_publications p
    where p.org_id is not null
      and not exists (select 1 from public.orgs o where o.id = p.org_id);
    if v_orphans = 0 then
        alter table public.contract_publications
            validate constraint contract_publications_org_id_fkey;
    else
        raise warning '041: % contract_publications rows still reference a non-org id; FK left NOT VALID', v_orphans;
    end if;
end $$;

-- RLS: org members, not "org_id = my user id".
drop policy if exists auth_own_org on public.contract_publications;
create policy auth_own_org on public.contract_publications
    for all to authenticated
    using (org_id in (select public.get_my_org_ids()))
    with check (org_id in (select public.get_my_org_ids()));
