-- ContractGate — Migration 004: Manual Replay Quarantine (RFC-003)
-- Run after 003_contract_versioning.sql
--
-- Additive only.  Adds link columns so a replay attempt can be traced back
-- to its source quarantine row, and the source can find the audit_log row
-- its payload ultimately landed in on success.  See docs/rfcs/003-auto-retry.md
-- for the full model.
--
-- The existing `quarantine_events.status` enum ('pending', 'reviewed',
-- 'replayed', 'purged') is unchanged; 'replayed' was reserved in
-- migration 002 and is now the terminal state reached by a successful
-- replay attempt against the source row.

-- ---------------------------------------------------------------------------
-- 1. quarantine_events: replay lifecycle + parent link
-- ---------------------------------------------------------------------------
--
-- replayed_at               : first successful replay timestamp (immutable).
-- replayed_into_audit_id    : FK to the audit_log row a passing replay
--                             produced (first-wins; immutable once set).
-- replay_of_quarantine_id   : for rows created BY a failed replay attempt,
--                             points back at the source quarantine row.
--                             NULL for fresh ingest-time quarantine rows.

ALTER TABLE quarantine_events
    ADD COLUMN IF NOT EXISTS replayed_at             TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS replayed_into_audit_id  UUID REFERENCES audit_log(id),
    ADD COLUMN IF NOT EXISTS replay_of_quarantine_id UUID REFERENCES quarantine_events(id);

-- ---------------------------------------------------------------------------
-- 2. audit_log: link passing replays back to their source quarantine row
-- ---------------------------------------------------------------------------
--
-- Fresh ingest audit rows leave this NULL.  Replay-pass audit rows set it
-- to the source quarantine row they originated from.

ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS replay_of_quarantine_id UUID REFERENCES quarantine_events(id);

-- ---------------------------------------------------------------------------
-- 3. Indexes for the replay-history lookup path
-- ---------------------------------------------------------------------------
--
-- "Show me all replay attempts for quarantine row X."  Partial indexes —
-- the vast majority of rows have NULL replay_of.

CREATE INDEX IF NOT EXISTS idx_quarantine_replay_of
    ON quarantine_events (replay_of_quarantine_id)
    WHERE replay_of_quarantine_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_audit_replay_of
    ON audit_log (replay_of_quarantine_id)
    WHERE replay_of_quarantine_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 4. Trigger: guard the immutability of successful-replay bookkeeping
-- ---------------------------------------------------------------------------
--
-- Once replayed_at / replayed_into_audit_id are populated (first successful
-- replay), they cannot be overwritten.  The app layer's conditional UPDATE
-- is authoritative, but this is belt-and-braces against accidental
-- double-marks if two concurrent replay requests race.
--
-- Allowed transitions on these columns:
--   (NULL, NULL) -> (ts, uuid)          -- first successful replay
--   (ts,  uuid)  -> (ts,  uuid)         -- no change (normal UPDATE pass)
--
-- Disallowed:
--   (ts,  uuid)  -> (ts',  uuid')       -- second replay overwriting first
--   (ts,  uuid)  -> (NULL, NULL)        -- clearing the stamp

CREATE OR REPLACE FUNCTION quarantine_replay_stamp_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.replayed_at IS NOT NULL
       AND (NEW.replayed_at IS DISTINCT FROM OLD.replayed_at
            OR NEW.replayed_into_audit_id IS DISTINCT FROM OLD.replayed_into_audit_id)
    THEN
        RAISE EXCEPTION
            'quarantine_events replay stamp is immutable once set '
            '(id=%, replayed_at=%, replayed_into_audit_id=%)',
            OLD.id, OLD.replayed_at, OLD.replayed_into_audit_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS quarantine_replay_stamp_guard_trg ON quarantine_events;
CREATE TRIGGER quarantine_replay_stamp_guard_trg
    BEFORE UPDATE ON quarantine_events
    FOR EACH ROW
    EXECUTE FUNCTION quarantine_replay_stamp_guard();
