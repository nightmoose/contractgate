-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 015: Kinesis Ingress (RFC-026)
--
-- Adds:
--   kinesis_ingress   — per-contract AWS Kinesis provisioning state
--
-- audit_log.source already supports arbitrary source tags (added in 014);
-- the Kinesis consumer will write source = 'kinesis'.
--
-- RLS follows the same pattern as kafka_ingress: org membership resolved via
-- get_my_org_ids() SECURITY DEFINER helper to avoid PG 42P17 recursion.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS kinesis_ingress (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id             UUID        NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    org_id                  UUID        NOT NULL REFERENCES orgs(id)      ON DELETE CASCADE,
    enabled                 BOOLEAN     NOT NULL DEFAULT FALSE,

    -- AWS region (fixed to us-east-1 for MVP; stored per-row for future expansion).
    aws_region              TEXT        NOT NULL DEFAULT 'us-east-1',

    -- Stream ARNs populated after provisioning.
    raw_stream_arn          TEXT,
    clean_stream_arn        TEXT,
    quarantine_stream_arn   TEXT,

    -- IAM user + credentials.
    -- iam_secret_enc stores the IAM secret access key encrypted with
    -- ENCRYPTION_KEY env var (AES-256-GCM).  Plaintext is NEVER stored.
    iam_user_arn            TEXT,
    iam_access_key_id       TEXT,
    iam_secret_enc          TEXT,

    -- Stream configuration.
    shard_count             INTEGER     NOT NULL DEFAULT 1,
    drain_window_hours      INTEGER     NOT NULL DEFAULT 24,

    -- Consumer checkpoint: last processed sequence number per shard.
    -- Stored as JSONB to support multi-shard configs without schema changes.
    -- Format: { "<shard_id>": "<sequence_number>" }
    -- NULL means start from TRIM_HORIZON (beginning of retention window).
    last_sequence_numbers   JSONB,

    -- Soft-delete timestamp.  Set when disabled; streams + IAM cleaned up
    -- after drain_window_hours, then row is hard-deleted.
    disabled_at             TIMESTAMPTZ,

    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- One active config per contract.
CREATE UNIQUE INDEX IF NOT EXISTS kinesis_ingress_contract_unique
    ON kinesis_ingress (contract_id)
    WHERE disabled_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_kinesis_ingress_contract
    ON kinesis_ingress (contract_id);

CREATE INDEX IF NOT EXISTS idx_kinesis_ingress_org
    ON kinesis_ingress (org_id);

CREATE INDEX IF NOT EXISTS idx_kinesis_ingress_enabled
    ON kinesis_ingress (enabled)
    WHERE enabled = TRUE;

-- Auto-update updated_at.
CREATE OR REPLACE TRIGGER kinesis_ingress_updated_at
    BEFORE UPDATE ON kinesis_ingress
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- ── RLS ──────────────────────────────────────────────────────────────────────

ALTER TABLE kinesis_ingress ENABLE ROW LEVEL SECURITY;

CREATE POLICY "org members can read kinesis ingress"
    ON kinesis_ingress FOR SELECT
    USING (org_id IN (SELECT public.get_my_org_ids()));

CREATE POLICY "org members can insert kinesis ingress"
    ON kinesis_ingress FOR INSERT
    WITH CHECK (org_id IN (SELECT public.get_my_org_ids()));

CREATE POLICY "org members can update kinesis ingress"
    ON kinesis_ingress FOR UPDATE
    USING (org_id IN (SELECT public.get_my_org_ids()));

CREATE POLICY "org members can delete kinesis ingress"
    ON kinesis_ingress FOR DELETE
    USING (org_id IN (SELECT public.get_my_org_ids()));
