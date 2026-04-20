-- ContractGate — Migration 005: PII Transforms (RFC-004)
-- Run after 004_quarantine_replay.sql
--
-- Additive only.  Adds the two columns RFC-004 needs:
--   1. `contracts.pii_salt`    — per-contract secret salt used by kind=hash
--                                (HMAC-SHA256 key) and by kind=mask
--                                style=format_preserving (PRNG seed).
--                                Populated on all existing rows via DEFAULT
--                                gen_random_bytes(32); generated server-side
--                                for new contracts.  Never exposed in any
--                                API response, audit row, or dashboard
--                                surface (the Rust layer owns that invariant
--                                — see contract.rs serializer).
--
--   2. `contract_versions.compliance_mode` — per-version boolean.  When true
--                                the validator raises UNDECLARED_FIELD on
--                                any inbound field not present in
--                                ontology.entities (normal per-event
--                                violation, not a batch-wide reject).
--                                Default false for backwards compat; flips
--                                on only when an author opts in.
--
-- Neither column is touched by any trigger.  Salt is intentionally immutable
-- after insert (rotation would invalidate every prior hash — see RFC-004
-- non-goals); compliance_mode follows the version's normal immutability
-- rules (mutable while draft, frozen once promoted).

-- ---------------------------------------------------------------------------
-- 1. contracts.pii_salt
-- ---------------------------------------------------------------------------
--
-- pgcrypto provides gen_random_bytes().  Prior migrations already depend on
-- uuid-ossp; pgcrypto is effectively always available on Supabase but we
-- ensure it explicitly here so this migration is self-contained.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

ALTER TABLE contracts
    ADD COLUMN IF NOT EXISTS pii_salt BYTEA NOT NULL
        DEFAULT gen_random_bytes(32);

-- The salt is a secret — defense in depth beyond the Rust serializer strip.
-- Nothing in the app or dashboard should ever SELECT it by column name in a
-- user-facing path; all access goes through contract.rs::CompiledContract.
COMMENT ON COLUMN contracts.pii_salt IS
    'Per-contract HMAC/PRNG salt used by PII transforms (RFC-004). Never surface in API responses, audit rows, or UI state.';

-- ---------------------------------------------------------------------------
-- 2. contract_versions.compliance_mode
-- ---------------------------------------------------------------------------

ALTER TABLE contract_versions
    ADD COLUMN IF NOT EXISTS compliance_mode BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN contract_versions.compliance_mode IS
    'RFC-004: when true, undeclared inbound fields raise UNDECLARED_FIELD as a per-event violation. Default false.';

-- ---------------------------------------------------------------------------
-- 3. Immutability-guard extension
-- ---------------------------------------------------------------------------
--
-- contract_versions already has an immutability trigger from migration 003
-- that forbids mutating yaml_content / version / contract_id / created_at
-- once state leaves draft.  compliance_mode needs the same protection — it
-- is part of the semantic contract and must be frozen post-promotion.
--
-- Rather than re-declare the whole function, we add a companion trigger
-- whose sole job is to guard compliance_mode.  Keeping it as its own
-- trigger avoids a destructive rewrite of the migration-003 function.

CREATE OR REPLACE FUNCTION contract_versions_compliance_mode_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.state IN ('stable', 'deprecated')
       AND NEW.compliance_mode IS DISTINCT FROM OLD.compliance_mode
    THEN
        RAISE EXCEPTION
            'contract_versions.compliance_mode is immutable once state leaves draft '
            '(id=%, state=%, old=%, new=%)',
            OLD.id, OLD.state, OLD.compliance_mode, NEW.compliance_mode;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS contract_versions_compliance_mode_guard_trg ON contract_versions;
CREATE TRIGGER contract_versions_compliance_mode_guard_trg
    BEFORE UPDATE ON contract_versions
    FOR EACH ROW
    EXECUTE FUNCTION contract_versions_compliance_mode_guard();
