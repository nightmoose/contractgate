-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 021: Provider-Consumer Collaboration (RFC-033)
--
-- Adds:
--   contract_collaborators      — scoped cross-org role grants on a single contract.
--   contract_comments           — threaded field-anchored notes.
--   contract_change_proposals   — editor proposes YAML; reviewer/owner approves/rejects;
--                                  owner applies.
--
-- Design notes:
--   • Every RLS policy that tests org membership MUST route through
--     public.get_my_org_ids() (SECURITY DEFINER helper from migration 008/013).
--     Inline subqueries on org_memberships cause PG 42P17 infinite-recursion.
--   • contract_collaborators PRIMARY KEY is (contract_name, org_id).
--     contract_name is the stable text name on public.contracts.name.
--   • The contracts SELECT policy is extended here to include collaborator orgs.
--     We DROP the existing policy written in migration 013 and recreate it with
--     the additional OR branch.  This is the only safe way to add a condition to
--     an existing Supabase RLS policy.
--   • "owner" role is implicit (the contract's org_id matches the caller's org);
--     it is never stored in contract_collaborators.
--   • org-visibility publications (RFC-032) resolve to a viewer row here:
--     import_published_contract (storage.rs) inserts a viewer collaborator when
--     visibility = 'org'.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. contract_collaborators ─────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.contract_collaborators (
    contract_name   text        NOT NULL,
    org_id          uuid        NOT NULL REFERENCES public.orgs(id) ON DELETE CASCADE,
    role            text        NOT NULL
                                CHECK (role IN ('editor', 'reviewer', 'viewer')),
    granted_by      uuid        NOT NULL,   -- org_id of the org that made the grant
    granted_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (contract_name, org_id)
);

COMMENT ON TABLE public.contract_collaborators IS
    'Scoped cross-org role grants on a single contract (RFC-033). '
    'An org not in this table and not the owner org has no access. '
    'Roles: editor (propose changes), reviewer (approve/reject), viewer (read-only). '
    'Owner role is implicit — it is the org_id on public.contracts, never stored here.';

CREATE INDEX IF NOT EXISTS idx_collabs_contract_name
    ON public.contract_collaborators (contract_name);

-- ── 2. contract_comments ──────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.contract_comments (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_name   text        NOT NULL,
    field           text,                       -- NULL = whole-contract comment
    org_id          uuid        NOT NULL,        -- commenter's org
    author          text        NOT NULL,        -- display name / email
    body            text        NOT NULL,
    resolved        boolean     NOT NULL DEFAULT false,
    created_at      timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.contract_comments IS
    'Flat threaded notes on a contract, optionally anchored to a specific field. '
    'Resolvable by any party with owner or collaborator access (RFC-033).';

CREATE INDEX IF NOT EXISTS idx_comments_contract_name
    ON public.contract_comments (contract_name, created_at DESC);

-- ── 3. contract_change_proposals ─────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.contract_change_proposals (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_name   text        NOT NULL,
    proposed_by     uuid        NOT NULL,   -- proposing org_id
    proposed_yaml   text        NOT NULL,
    status          text        NOT NULL DEFAULT 'open'
                                CHECK (status IN ('open', 'approved', 'rejected', 'applied')),
    decided_by      uuid,                   -- org_id that approved/rejected
    created_at      timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.contract_change_proposals IS
    'An editor proposes a YAML change; a reviewer or owner approves or rejects; '
    'the owner applies. Collaborator edits never land on a stable version directly '
    '— they always flow through proposal → approve → apply (RFC-033).';

CREATE INDEX IF NOT EXISTS idx_proposals_contract_status
    ON public.contract_change_proposals (contract_name, status, created_at DESC);

-- ── 4. RLS — contract_collaborators ──────────────────────────────────────────

ALTER TABLE public.contract_collaborators ENABLE ROW LEVEL SECURITY;

-- Service role has unrestricted access (Rust backend).
CREATE POLICY "service_all" ON public.contract_collaborators
    FOR ALL TO service_role
    USING (TRUE) WITH CHECK (TRUE);

-- Authenticated users can see collaborator rows where their org is the
-- collaborator OR their org owns the contract.
-- Both membership checks go through get_my_org_ids() — no inline subquery.
CREATE POLICY "auth_read" ON public.contract_collaborators
    FOR SELECT TO authenticated
    USING (
        -- My org is the collaborator being listed.
        org_id = ANY (SELECT public.get_my_org_ids())
        -- OR my org owns the contract (owner can see all collaborators).
        OR contract_name IN (
            SELECT c.name
            FROM   public.contracts c
            WHERE  c.org_id = ANY (SELECT public.get_my_org_ids())
              AND  c.deleted_at IS NULL
        )
    );

-- ── 5. RLS — contract_comments ────────────────────────────────────────────────

ALTER TABLE public.contract_comments ENABLE ROW LEVEL SECURITY;

CREATE POLICY "service_all" ON public.contract_comments
    FOR ALL TO service_role
    USING (TRUE) WITH CHECK (TRUE);

-- Visible to orgs that own or collaborate on the contract.
CREATE POLICY "auth_read" ON public.contract_comments
    FOR SELECT TO authenticated
    USING (
        contract_name IN (
            SELECT c.name
            FROM   public.contracts c
            WHERE  c.org_id = ANY (SELECT public.get_my_org_ids())
              AND  c.deleted_at IS NULL
        )
        OR contract_name IN (
            SELECT cc.contract_name
            FROM   public.contract_collaborators cc
            WHERE  cc.org_id = ANY (SELECT public.get_my_org_ids())
        )
    );

-- ── 6. RLS — contract_change_proposals ───────────────────────────────────────

ALTER TABLE public.contract_change_proposals ENABLE ROW LEVEL SECURITY;

CREATE POLICY "service_all" ON public.contract_change_proposals
    FOR ALL TO service_role
    USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "auth_read" ON public.contract_change_proposals
    FOR SELECT TO authenticated
    USING (
        contract_name IN (
            SELECT c.name
            FROM   public.contracts c
            WHERE  c.org_id = ANY (SELECT public.get_my_org_ids())
              AND  c.deleted_at IS NULL
        )
        OR contract_name IN (
            SELECT cc.contract_name
            FROM   public.contract_collaborators cc
            WHERE  cc.org_id = ANY (SELECT public.get_my_org_ids())
        )
    );

-- ── 7. Extend contracts SELECT policy to include collaborator orgs ────────────
--
-- Migration 013 wrote the policy "org members can read own contracts" which
-- checks: deleted_at IS NULL AND org_id IN (get_my_org_ids()).
--
-- We widen it to: owner org OR any collaborator org.
-- Only the SELECT (read) policy needs widening — insert/update/delete stay
-- owner-org-only (a collaborator can never mutate a contract row directly).

DROP POLICY IF EXISTS "org members can read own contracts" ON public.contracts;

CREATE POLICY "org members and collaborators can read contracts"
    ON public.contracts FOR SELECT TO authenticated
    USING (
        deleted_at IS NULL
        AND (
            -- My org owns this contract.
            org_id = ANY (SELECT public.get_my_org_ids())
            -- OR my org is an explicit collaborator on this contract.
            OR name IN (
                SELECT cc.contract_name
                FROM   public.contract_collaborators cc
                WHERE  cc.org_id = ANY (SELECT public.get_my_org_ids())
            )
        )
    );
