-- ContractGate — Migration 002: Quarantine Events + P99 Latency Support
-- Run after 001_initial_schema.sql

-- ---------------------------------------------------------------------------
-- quarantine_events: holds events that failed contract validation
--
-- Failed events are written here so they can be inspected, replayed, or
-- purged after human review.  This separates "bad data" from the clean
-- forwarded_events table without losing the original payload.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS quarantine_events (
    id                UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id       UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    payload           JSONB       NOT NULL,
    violation_count   INTEGER     NOT NULL DEFAULT 0,
    violation_details JSONB       NOT NULL DEFAULT '[]'::jsonb,
    validation_us     BIGINT      NOT NULL DEFAULT 0,
    source_ip         TEXT,
    -- Lifecycle: 'pending' → 'reviewed' → 'purged' | 'replayed'
    status            TEXT        NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending', 'reviewed', 'replayed', 'purged')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at       TIMESTAMPTZ
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_quarantine_contract_id  ON quarantine_events (contract_id);
CREATE INDEX IF NOT EXISTS idx_quarantine_created_at   ON quarantine_events (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_quarantine_status       ON quarantine_events (status);
-- Fast lookup of pending items per contract
CREATE INDEX IF NOT EXISTS idx_quarantine_pending      ON quarantine_events (contract_id, created_at DESC)
    WHERE status = 'pending';

-- RLS
ALTER TABLE quarantine_events ENABLE ROW LEVEL SECURITY;

CREATE POLICY "auth_all" ON quarantine_events
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "service_all" ON quarantine_events
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);

-- ---------------------------------------------------------------------------
-- p99 latency support — percentile helper function
--
-- Adds a lightweight percentile_disc aggregate so the stats endpoint can
-- return p99 validation latency without pulling all rows into Rust.
-- ---------------------------------------------------------------------------

-- percentile_disc is built-in as an ordered-set aggregate in PostgreSQL 9.4+;
-- no extension needed.  The view below wraps it for easy querying.

CREATE OR REPLACE VIEW v_latency_percentiles AS
SELECT
    contract_id,
    COUNT(*)                                                    AS total_events,
    AVG(validation_us)::BIGINT                                  AS avg_us,
    percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50_us,
    percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95_us,
    percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99_us
FROM audit_log
GROUP BY contract_id;

-- Global (all contracts combined)
CREATE OR REPLACE VIEW v_latency_percentiles_global AS
SELECT
    COUNT(*)                                                    AS total_events,
    AVG(validation_us)::BIGINT                                  AS avg_us,
    percentile_disc(0.50) WITHIN GROUP (ORDER BY validation_us) AS p50_us,
    percentile_disc(0.95) WITHIN GROUP (ORDER BY validation_us) AS p95_us,
    percentile_disc(0.99) WITHIN GROUP (ORDER BY validation_us) AS p99_us
FROM audit_log;
