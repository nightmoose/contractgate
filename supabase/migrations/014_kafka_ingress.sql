-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 014: Kafka Ingress (RFC-025)
--
-- Adds:
--   kafka_ingress     — per-contract Confluent Cloud provisioning state
--   audit_log.source  — ingestion source tag ('http' | 'kafka'); default 'http'
--                       so existing rows retain their meaning.
--
-- RLS on kafka_ingress follows the same pattern as all other tables:
-- org membership is resolved via the SECURITY DEFINER helper
-- get_my_org_ids() to avoid the PG 42P17 recursion fixed in migration 008.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. audit_log: add source column ──────────────────────────────────────────

ALTER TABLE audit_log
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'http';

-- Index for filtering audit log by source (e.g. dashboard 'Kafka' tab).
CREATE INDEX IF NOT EXISTS idx_audit_source
    ON audit_log (source)
    WHERE source <> 'http';

-- ── 2. kafka_ingress table ────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS kafka_ingress (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id             UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    org_id                  UUID        NOT NULL REFERENCES orgs(id)      ON DELETE CASCADE,
    enabled                 BOOLEAN     NOT NULL DEFAULT FALSE,

    -- Confluent Cloud connection details.
    -- confluent_api_secret_enc stores the Confluent API secret encrypted with
    -- the application's ENCRYPTION_KEY env var (AES-256-GCM).  The plaintext
    -- secret is NEVER stored.
    confluent_bootstrap     TEXT        NOT NULL,
    confluent_api_key       TEXT        NOT NULL,
    confluent_api_secret_enc TEXT       NOT NULL,

    -- Topic provisioning metadata.
    partition_count         INTEGER     NOT NULL DEFAULT 3,
    drain_window_hours      INTEGER     NOT NULL DEFAULT 24,

    -- Soft-delete timestamp.  Set when the user disables ingress; topics are
    -- deleted and the row is hard-deleted after drain_window_hours elapses.
    disabled_at             TIMESTAMPTZ,

    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- One active config per contract (unique on contract_id while enabled).
CREATE UNIQUE INDEX IF NOT EXISTS kafka_ingress_contract_unique
    ON kafka_ingress (contract_id)
    WHERE disabled_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_kafka_ingress_contract
    ON kafka_ingress (contract_id);

CREATE INDEX IF NOT EXISTS idx_kafka_ingress_org
    ON kafka_ingress (org_id);

-- Auto-update updated_at on every row change.
CREATE OR REPLACE TRIGGER kafka_ingress_updated_at
    BEFORE UPDATE ON kafka_ingress
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- ── 3. RLS ───────────────────────────────────────────────────────────────────

ALTER TABLE kafka_ingress ENABLE ROW LEVEL SECURITY;

-- Org members can read their own kafka_ingress rows.
CREATE POLICY "org members can read kafka ingress"
    ON kafka_ingress FOR SELECT
    USING (org_id IN (SELECT public.get_my_org_ids()));

-- Org members can insert kafka_ingress rows for their org.
CREATE POLICY "org members can insert kafka ingress"
    ON kafka_ingress FOR INSERT
    WITH CHECK (org_id IN (SELECT public.get_my_org_ids()));

-- Org members can update kafka_ingress rows for their org.
CREATE POLICY "org members can update kafka ingress"
    ON kafka_ingress FOR UPDATE
    USING (org_id IN (SELECT public.get_my_org_ids()));

-- Org members can delete kafka_ingress rows for their org.
CREATE POLICY "org members can delete kafka ingress"
    ON kafka_ingress FOR DELETE
    USING (org_id IN (SELECT public.get_my_org_ids()));
