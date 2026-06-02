-- Migration 010: ODCS import provenance tracking
--
-- Adds two columns to contract_versions so we can track where a version
-- came from and whether it needs human review before promotion to stable.
--
-- import_source values:
--   'native'        — created natively in ContractGate (default)
--   'odcs'          — imported from ODCS with x-contractgate-* extensions
--                     (lossless round-trip)
--   'odcs_stripped' — imported from ODCS without extensions
--                     (reduced fidelity; requires_review = TRUE)
--
-- requires_review: set TRUE on odcs_stripped imports; blocks promotion to
-- stable until a human explicitly clears it via the approve-import endpoint.

ALTER TABLE contract_versions
    ADD COLUMN IF NOT EXISTS import_source    TEXT    NOT NULL DEFAULT 'native',
    ADD COLUMN IF NOT EXISTS requires_review  BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE contract_versions
    ADD CONSTRAINT contract_versions_import_source_check
    CHECK (import_source IN ('native', 'odcs', 'odcs_stripped'));
