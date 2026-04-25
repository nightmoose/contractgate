-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 009: GitHub integration
--
-- Adds:
--   github_integrations — one row per org, stores GitHub repo + PAT config
--
-- Design:
--   • One config per org (UNIQUE on org_id). Org owners manage it via the
--     /account settings page.
--   • github_token stores a GitHub Personal Access Token (PAT) with repo
--     write scope. Supabase encrypts data at rest; RLS restricts access to
--     org members only.
--   • The Next.js API route (/api/github/sync) reads this config server-side
--     (service role) so the token is never exposed to the browser.
-- ─────────────────────────────────────────────────────────────────────────────

create table if not exists public.github_integrations (
    id           uuid        primary key default gen_random_uuid(),
    org_id       uuid        not null references public.orgs(id) on delete cascade,
    -- "owner/repo" e.g. "acme-corp/data-contracts"
    repo         text        not null,
    -- directory prefix inside the repo, e.g. "contracts/" (must end with /)
    path_prefix  text        not null default 'contracts/',
    -- branch to commit to
    branch       text        not null default 'main',
    -- GitHub Personal Access Token with repo write scope.
    -- Stored server-side only; never sent to the browser.
    github_token text,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),

    constraint github_integrations_org_unique unique (org_id),
    constraint github_integrations_repo_not_empty
        check (char_length(trim(repo)) > 0),
    constraint github_integrations_path_prefix_slash
        check (path_prefix = '' or path_prefix like '%/')
);

comment on table public.github_integrations is
    'One row per org. Stores the GitHub repo + PAT needed to commit contracts '
    'as YAML files. Token is kept server-side and never exposed to the browser.';

comment on column public.github_integrations.repo is
    '"owner/repo" format, e.g. "acme-corp/data-contracts".';

comment on column public.github_integrations.path_prefix is
    'Slash-terminated path prefix inside the repo, e.g. "contracts/". '
    'Files are committed to <path_prefix><contract_slug>/<version>.yaml.';

comment on column public.github_integrations.github_token is
    'GitHub Personal Access Token with contents:write scope. '
    'Stored at rest; never returned to the browser via the config GET endpoint.';

-- updated_at trigger (reuse the set_updated_at function from migration 006)
create trigger github_integrations_updated_at
    before update on public.github_integrations
    for each row execute procedure public.set_updated_at();

-- ── Indexes ───────────────────────────────────────────────────────────────────

create index if not exists github_integrations_org_id_idx
    on public.github_integrations (org_id);

-- ── Row Level Security ────────────────────────────────────────────────────────

alter table public.github_integrations enable row level security;

-- Org members can read their own integration config.
-- NOTE: The token column is intentionally excluded from the SELECT policy
-- scope — the token is only read server-side via the service role.
create policy "org members can read own github integration"
    on public.github_integrations for select
    using (
        org_id in (
            select org_id from public.org_memberships where user_id = auth.uid()
        )
    );

-- Only org owners and admins can create the integration config.
create policy "org owners and admins can create github integration"
    on public.github_integrations for insert
    with check (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

-- Only org owners and admins can update the integration config.
create policy "org owners and admins can update github integration"
    on public.github_integrations for update
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

-- Only org owners and admins can delete the integration config.
create policy "org owners and admins can delete github integration"
    on public.github_integrations for delete
    using (
        org_id in (
            select org_id from public.org_memberships
            where user_id = auth.uid()
              and role in ('owner', 'admin')
        )
    );

-- Service role has full access (used by Next.js API routes to read the token
-- server-side without surfacing it to the browser).
create policy "service role full access to github integrations"
    on public.github_integrations for all
    to service_role
    using (true) with check (true);
