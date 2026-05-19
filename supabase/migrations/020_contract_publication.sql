-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 020: Contract Sharing & Publication (RFC-032)
--
-- Adds:
--   contract_publications  — one row per published contract version.
--   contracts (provenance) — imported_from_ref, import_mode, imported_at columns.
--
-- Design notes:
--   • `ref` is a 24-hex-char opaque stable id (12 random bytes encoded as hex).
--   • `link_token` is 32 hex chars (16 random bytes) — required when
--     visibility = 'link', NULL otherwise.
--   • revoked_at is NULL while active; set to NOW() on soft-delete.
--   • `org` visibility is defined in the schema now but enforced/used by RFC-033.
--   • The (contract_id, version) FK references the contract_versions table
--     using the UUID row id (not the (contract_id, version) text pair) to keep
--     the reference stable even if version strings change.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. contract_publications table ───────────────────────────────────────────

CREATE TABLE IF NOT EXISTS contract_publications (
    ref             text        PRIMARY KEY
                                DEFAULT encode(gen_random_bytes(12), 'hex'),
    contract_id     uuid        NOT NULL
                                REFERENCES contracts(id) ON DELETE CASCADE,
    version_id      uuid        NOT NULL
                                REFERENCES contract_versions(id) ON DELETE CASCADE,
    -- Denormalised for fast public-facing fetch without a JOIN.
    contract_name   text        NOT NULL,
    contract_version text       NOT NULL,
    yaml_content    text        NOT NULL,
    visibility      text        NOT NULL DEFAULT 'link'
                                CHECK (visibility IN ('public', 'link', 'org')),
    -- Non-null iff visibility = 'link'.
    link_token      text,
    -- Auth: which org published this.
    org_id          uuid        REFERENCES auth.users(id),
    published_by    text,                          -- human-readable label / email
    published_at    timestamptz NOT NULL DEFAULT now(),
    revoked_at      timestamptz,                   -- NULL = active
    CONSTRAINT link_token_required
        CHECK (visibility != 'link' OR link_token IS NOT NULL)
);

-- Fast lookup by (contract_id, visibility) for the "get current publication"
-- path used by import-status.
CREATE INDEX IF NOT EXISTS idx_cp_contract_id
    ON contract_publications (contract_id, visibility);

-- ── 2. RLS on contract_publications ──────────────────────────────────────────

ALTER TABLE contract_publications ENABLE ROW LEVEL SECURITY;

-- Authenticated users can see their own org's publications.
DROP POLICY IF EXISTS "auth_own_org" ON contract_publications;
CREATE POLICY "auth_own_org" ON contract_publications
    FOR ALL TO authenticated
    USING (org_id = auth.uid())
    WITH CHECK (org_id = auth.uid());

-- Service role has full access (used by the Rust backend).
DROP POLICY IF EXISTS "service_all" ON contract_publications;
CREATE POLICY "service_all" ON contract_publications
    FOR ALL TO service_role
    USING (TRUE) WITH CHECK (TRUE);

-- ── 3. Provenance columns on contracts ───────────────────────────────────────
--
-- Tracks whether a contract was imported from a publication and in what mode.
-- All three columns are NULL on natively-created contracts.

ALTER TABLE contracts
    ADD COLUMN IF NOT EXISTS imported_from_ref  text,
    ADD COLUMN IF NOT EXISTS import_mode        text
        CHECK (import_mode IN ('snapshot', 'subscribe')),
    ADD COLUMN IF NOT EXISTS imported_at        timestamptz;

-- FK back to the publication (informational; not enforced to allow revocation
-- without cascading deletes on the importing contract side).
CREATE INDEX IF NOT EXISTS idx_contracts_imported_from_ref
    ON contracts (imported_from_ref)
    WHERE imported_from_ref IS NOT NULL;

-- ── 4. Extend import_source CHECK constraint on contract_versions ─────────────
--
-- Migration 010 created: CHECK (import_source IN ('native', 'odcs', 'odcs_stripped'))
-- Publication imports write import_source = 'publication', so we must widen it.

ALTER TABLE contract_versions
    DROP CONSTRAINT IF EXISTS contract_versions_import_source_check;

ALTER TABLE contract_versions
    ADD CONSTRAINT contract_versions_import_source_check
        CHECK (import_source IN ('native', 'odcs', 'odcs_stripped', 'publication'));
