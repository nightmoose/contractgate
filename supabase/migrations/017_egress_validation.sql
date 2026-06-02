-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 017: Egress Validation (RFC-029)
--
-- Adds a `direction` discriminator column to `audit_log` and
-- `quarantine_events` so inbound (ingress) and outbound (egress) validation
-- events can be distinguished and queried together in one table.
--
-- Existing rows default to `'ingress'` — no behavior change for any current
-- query.  New egress validation writes set `direction = 'egress'`.
--
-- Adds:
--   audit_log.direction         — 'ingress' | 'egress', NOT NULL DEFAULT 'ingress'
--   quarantine_events.direction — 'ingress' | 'egress', NOT NULL DEFAULT 'ingress'
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. audit_log ─────────────────────────────────────────────────────────────

ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS direction text NOT NULL DEFAULT 'ingress'
    CHECK (direction IN ('ingress', 'egress'));

-- Index for "show me all egress audit rows for this contract" queries
-- (RFC-031 scorecard will need this).
CREATE INDEX IF NOT EXISTS idx_audit_log_direction
    ON audit_log (contract_id, direction, created_at DESC);

-- ── 2. quarantine_events ─────────────────────────────────────────────────────

ALTER TABLE quarantine_events
    ADD COLUMN IF NOT EXISTS direction text NOT NULL DEFAULT 'ingress'
    CHECK (direction IN ('ingress', 'egress'));

-- Index for "show me all egress quarantine rows for this contract" queries.
CREATE INDEX IF NOT EXISTS idx_quarantine_events_direction
    ON quarantine_events (contract_id, direction, created_at DESC);

-- ── 3. RLS — no changes needed ───────────────────────────────────────────────
--
-- Existing RLS policies on audit_log and quarantine_events are based on
-- org_id and contract_id, not on direction.  Both ingress and egress rows
-- belong to the same org that owns the contract, so the existing policies
-- correctly scope all rows.  No new policy is required.
