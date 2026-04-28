-- ContractGate — Initial Schema
-- Migration: 001_initial_schema
-- Run this against your Supabase project via the SQL editor or psql.

-- Supabase compatibility roles for local/CI Postgres
-- (these roles exist automatically in real Supabase projects)
DO $$
BEGIN
    CREATE ROLE "anon";
EXCEPTION WHEN duplicate_object THEN null;
END $$;

DO $$
BEGIN
    CREATE ROLE "authenticated";
EXCEPTION WHEN duplicate_object THEN null;
END $$;

DO $$
BEGIN
    CREATE ROLE "service_role";
EXCEPTION WHEN duplicate_object THEN null;
END $$;

-- Enable UUID extension (usually already enabled in Supabase)
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ---------------------------------------------------------------------------
-- contracts: versioned semantic contracts
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS contracts (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name         TEXT        NOT NULL,
    version      TEXT        NOT NULL DEFAULT '1.0',
    active       BOOLEAN     NOT NULL DEFAULT TRUE,
    yaml_content TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_contracts_active ON contracts (active);
CREATE INDEX IF NOT EXISTS idx_contracts_name   ON contracts (name);

-- Trigger to auto-update updated_at
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contracts_updated_at
    BEFORE UPDATE ON contracts
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- ---------------------------------------------------------------------------
-- audit_log: immutable record of every ingestion validation attempt
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS audit_log (
    id                UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id       UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    passed            BOOLEAN     NOT NULL,
    violation_count   INTEGER     NOT NULL DEFAULT 0,
    violation_details JSONB       NOT NULL DEFAULT '[]'::jsonb,
    raw_event         JSONB       NOT NULL DEFAULT '{}'::jsonb,
    validation_us     BIGINT      NOT NULL DEFAULT 0,   -- microseconds
    source_ip         TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for dashboard queries
CREATE INDEX IF NOT EXISTS idx_audit_contract_id  ON audit_log (contract_id);
CREATE INDEX IF NOT EXISTS idx_audit_created_at   ON audit_log (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_passed        ON audit_log (passed);
CREATE INDEX IF NOT EXISTS idx_audit_contract_time ON audit_log (contract_id, created_at DESC);

-- Partial index for fast violation queries
CREATE INDEX IF NOT EXISTS idx_audit_violations ON audit_log (contract_id, created_at DESC)
    WHERE passed = FALSE;

-- ---------------------------------------------------------------------------
-- forwarded_events: validated events forwarded to downstream destinations
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS forwarded_events (
    id          UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    payload     JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_forwarded_contract_id ON forwarded_events (contract_id);
CREATE INDEX IF NOT EXISTS idx_forwarded_created_at  ON forwarded_events (created_at DESC);

-- ---------------------------------------------------------------------------
-- Supabase Row Level Security (RLS) — enable and lock down to authenticated users
-- ---------------------------------------------------------------------------

ALTER TABLE contracts       ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_log       ENABLE ROW LEVEL SECURITY;
ALTER TABLE forwarded_events ENABLE ROW LEVEL SECURITY;

-- Allow all operations for authenticated users (customize as needed)
CREATE POLICY "auth_all" ON contracts
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "auth_all" ON audit_log
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "auth_all" ON forwarded_events
    FOR ALL TO authenticated USING (TRUE) WITH CHECK (TRUE);

-- Allow the service role (backend Rust API) full access
CREATE POLICY "service_all" ON contracts
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "service_all" ON audit_log
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);

CREATE POLICY "service_all" ON forwarded_events
    FOR ALL TO service_role USING (TRUE) WITH CHECK (TRUE);

-- ---------------------------------------------------------------------------
-- Helpful views for the dashboard
-- ---------------------------------------------------------------------------

CREATE OR REPLACE VIEW v_ingestion_summary AS
SELECT
    c.id            AS contract_id,
    c.name          AS contract_name,
    c.version,
    COUNT(a.id)     AS total_events,
    SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)  AS passed_events,
    SUM(CASE WHEN NOT a.passed THEN 1 ELSE 0 END) AS failed_events,
    ROUND(
        SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)::NUMERIC / NULLIF(COUNT(a.id), 0) * 100,
        2
    )               AS pass_rate_pct,
    AVG(a.validation_us)::BIGINT AS avg_validation_us,
    MAX(a.created_at) AS last_event_at
FROM contracts c
LEFT JOIN audit_log a ON a.contract_id = c.id
GROUP BY c.id, c.name, c.version;
