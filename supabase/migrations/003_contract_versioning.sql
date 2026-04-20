-- ContractGate — Migration 003: Contract Versioning (RFC-002)
-- Run after 002_quarantine_and_p99.sql
--
-- This migration promotes `contracts.version` from a nominal text field into
-- a first-class versioning system with strict, forward-only state
-- transitions (draft → stable → deprecated) and full immutability once a
-- version leaves draft.  See docs/rfcs/002-versioning.md for rationale.
--
-- DESTRUCTIVE: truncates all existing dev/test data so the schema can be
-- reshaped cleanly.  Per Alex 2026-04-18 — this is a dev/test environment.

-- ---------------------------------------------------------------------------
-- 0a. Drop the v1 `v_ingestion_summary` view BEFORE reshaping any columns.
--
-- The view (from migration 001) references `contracts.version`, which
-- Section 1 drops.  Without this explicit DROP, the ALTER TABLE in Section 1
-- errors with `2BP01: cannot drop column version because other objects
-- depend on it` and the whole migration aborts.  The view is recreated
-- against the new schema in Section 6.
-- ---------------------------------------------------------------------------

DROP VIEW IF EXISTS v_ingestion_summary;

-- ---------------------------------------------------------------------------
-- 0b. Wipe dependent data so we can reshape the tables without compat pain.
-- ---------------------------------------------------------------------------

TRUNCATE audit_log, quarantine_events, forwarded_events, contracts CASCADE;

-- ---------------------------------------------------------------------------
-- 1. Reshape `contracts`: identity-only, with policy flag + description.
-- ---------------------------------------------------------------------------

-- Drop obsolete single-version fields.  yaml_content moves to contract_versions.
ALTER TABLE contracts DROP COLUMN IF EXISTS version;
ALTER TABLE contracts DROP COLUMN IF EXISTS active;
ALTER TABLE contracts DROP COLUMN IF EXISTS yaml_content;

-- Resolution policy for unpinned traffic.
--   'strict'   — latest stable only, fail-closed (default, matches pitch).
--   'fallback' — on failure, retry other stables in promoted_at DESC order,
--                first pass wins.  See RFC-002 §2b.
ALTER TABLE contracts
    ADD COLUMN multi_stable_resolution TEXT NOT NULL DEFAULT 'strict'
        CHECK (multi_stable_resolution IN ('strict', 'fallback'));

-- Optional human-readable description.  Mutable; rename history mirrors to
-- contract_name_history via a trigger (see §4 below).
ALTER TABLE contracts
    ADD COLUMN description TEXT;

-- ---------------------------------------------------------------------------
-- 2. New table: contract_versions — one row per (contract, version) pair.
-- ---------------------------------------------------------------------------

CREATE TABLE contract_versions (
    id             UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id    UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    version        TEXT        NOT NULL,        -- semver: "1.0.0", "2.1.3"
    state          TEXT        NOT NULL
                   CHECK (state IN ('draft', 'stable', 'deprecated')),
    yaml_content   TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    promoted_at    TIMESTAMPTZ,                 -- draft → stable timestamp
    deprecated_at  TIMESTAMPTZ,                 -- stable → deprecated timestamp
    UNIQUE (contract_id, version)
);

-- Generic lookup by (contract_id, state).
CREATE INDEX idx_cv_contract_state
    ON contract_versions (contract_id, state);

-- Hot path: "latest stable per contract" — used on every unpinned request.
CREATE INDEX idx_cv_latest_stable
    ON contract_versions (contract_id, promoted_at DESC)
    WHERE state = 'stable';

-- ---------------------------------------------------------------------------
-- 3. Immutability + deletion guards (belt-and-braces; app also enforces).
-- ---------------------------------------------------------------------------

-- Once a version leaves draft, its content is frozen forever.  The only
-- legal UPDATE on a non-draft row is a state transition stable → deprecated
-- (with the deprecated_at timestamp being set).
CREATE OR REPLACE FUNCTION contract_versions_immutability_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.state IN ('stable', 'deprecated') THEN
        IF NEW.yaml_content IS DISTINCT FROM OLD.yaml_content
        OR NEW.version      IS DISTINCT FROM OLD.version
        OR NEW.contract_id  IS DISTINCT FROM OLD.contract_id
        OR NEW.created_at   IS DISTINCT FROM OLD.created_at
        OR NEW.promoted_at  IS DISTINCT FROM OLD.promoted_at
        OR (OLD.state = 'stable'     AND NEW.state NOT IN ('stable', 'deprecated'))
        OR (OLD.state = 'deprecated' AND NEW.state <> 'deprecated')
        THEN
            RAISE EXCEPTION
                'contract_versions row is frozen once state leaves draft '
                '(id=%, state=%)', OLD.id, OLD.state;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contract_versions_frozen
    BEFORE UPDATE ON contract_versions
    FOR EACH ROW EXECUTE FUNCTION contract_versions_immutability_guard();

-- Only draft versions can be deleted.  Stable and deprecated live forever.
CREATE OR REPLACE FUNCTION contract_versions_delete_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.state <> 'draft' THEN
        RAISE EXCEPTION
            'only draft versions may be deleted (id=%, state=%)',
            OLD.id, OLD.state;
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contract_versions_deletable_draft_only
    BEFORE DELETE ON contract_versions
    FOR EACH ROW EXECUTE FUNCTION contract_versions_delete_guard();

-- ---------------------------------------------------------------------------
-- 4. Name history — append-only mirror of every rename.
-- ---------------------------------------------------------------------------

CREATE TABLE contract_name_history (
    id           UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id  UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    old_name     TEXT        NOT NULL,
    new_name     TEXT        NOT NULL,
    changed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cnh_contract_time
    ON contract_name_history (contract_id, changed_at DESC);

CREATE OR REPLACE FUNCTION contracts_name_history_trigger()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.name IS DISTINCT FROM OLD.name THEN
        INSERT INTO contract_name_history (contract_id, old_name, new_name)
        VALUES (OLD.id, OLD.name, NEW.name);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contracts_record_rename
    AFTER UPDATE OF name ON contracts
    FOR EACH ROW EXECUTE FUNCTION contracts_name_history_trigger();

-- ---------------------------------------------------------------------------
-- 5. Audit trail — every row now identifies the exact version used.
--
-- Under fallback mode, contract_version records the version that ACCEPTED
-- the event, not the default latest-stable.  "validated under" always
-- reflects the actual contract used (see memory: audit honesty).
-- ---------------------------------------------------------------------------

ALTER TABLE audit_log         ADD COLUMN contract_version TEXT;
ALTER TABLE quarantine_events ADD COLUMN contract_version TEXT;
ALTER TABLE forwarded_events  ADD COLUMN contract_version TEXT;

CREATE INDEX idx_audit_contract_version
    ON audit_log (contract_id, contract_version);

-- ---------------------------------------------------------------------------
-- 6. Rebuild v_ingestion_summary against the new schema (groups by
--    contract_version too).  The old view was dropped in Section 0a before
--    the column reshape so we only need the CREATE here.
--    v_latency_percentiles is unchanged (contract_id-level is still useful).
-- ---------------------------------------------------------------------------

CREATE VIEW v_ingestion_summary AS
SELECT
    c.id                          AS contract_id,
    c.name                        AS contract_name,
    c.multi_stable_resolution,
    a.contract_version,
    COUNT(a.id)                   AS total_events,
    SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)     AS passed_events,
    SUM(CASE WHEN NOT a.passed THEN 1 ELSE 0 END) AS failed_events,
    ROUND(
        SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)::NUMERIC
        / NULLIF(COUNT(a.id), 0) * 100, 2
    )                             AS pass_rate_pct,
    AVG(a.validation_us)::BIGINT  AS avg_validation_us,
    MAX(a.created_at)             AS last_event_at
FROM contracts c
LEFT JOIN audit_log a ON a.contract_id = c.id
GROUP BY c.id, c.name, c.multi_stable_resolution, a.contract_version;

-- ---------------------------------------------------------------------------
-- 7. RLS on the new tables — same "authenticated + service_role have all" policy
--    as the existing tables (lock down in production later).
-- ---------------------------------------------------------------------------

ALTER TABLE contract_versions     ENABLE ROW LEVEL SECURITY;
ALTER TABLE contract_name_history ENABLE ROW LEVEL SECURITY;

CREATE POLICY "auth_all" ON contract_versions
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);
CREATE POLICY "service_all" ON contract_versions
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "auth_all" ON contract_name_history
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);
CREATE POLICY "service_all" ON contract_name_history
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);
