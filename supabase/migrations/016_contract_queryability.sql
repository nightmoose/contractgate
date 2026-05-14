-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 016: Contract Queryability (RFC-028)
--
-- Adds queryability metadata to contract_versions so contracts are first-class
-- SQL objects alongside the audit log.  No breaking changes to existing rows or
-- APIs — all new columns are nullable.
--
-- Adds:
--   contract_versions.source       — PMS vendor or logical feed name
--   contract_versions.deployed_by  — CI job or user that ran deploy-contract
--   contract_versions.deployed_at  — wall-clock time of the deploy operation
--   contract_versions.parsed_json  — parsed ontology as JSONB for SQL querying
--
-- Creates:
--   active_contracts_public — read-only view for auditors / external stakeholders
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. New columns on contract_versions ──────────────────────────────────────

ALTER TABLE contract_versions
    ADD COLUMN IF NOT EXISTS source      text,
    ADD COLUMN IF NOT EXISTS deployed_by text,
    ADD COLUMN IF NOT EXISTS deployed_at timestamptz,
    ADD COLUMN IF NOT EXISTS parsed_json jsonb;

-- Fast lookup: "which stable versions came from this source?"
CREATE INDEX IF NOT EXISTS idx_cv_source
    ON contract_versions (source)
    WHERE state = 'stable' AND source IS NOT NULL;

-- Fast lookup: "all stable versions for a contract, newest deploy first"
CREATE INDEX IF NOT EXISTS idx_cv_deployed_at
    ON contract_versions (contract_id, deployed_at DESC NULLS LAST)
    WHERE state = 'stable';

-- ── 2. Auditor view — no full-DB access required ──────────────────────────────
--
-- Grants SELECT on this view to authenticated users.  The underlying
-- contract_versions and contracts tables remain protected by RLS; the view
-- is intentionally limited to fields safe for external review.

CREATE OR REPLACE VIEW active_contracts_public AS
SELECT
    c.id            AS contract_id,
    c.name,
    cv.version,
    cv.source,
    cv.deployed_at,
    cv.deployed_by,
    cv.parsed_json
FROM contract_versions cv
JOIN contracts c ON c.id = cv.contract_id
WHERE cv.state = 'stable';

GRANT SELECT ON active_contracts_public TO authenticated;
