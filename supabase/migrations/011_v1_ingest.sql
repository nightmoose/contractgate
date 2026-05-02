-- Migration 011: v1 bulk ingest endpoint support (RFC-021)
--
-- 1. idempotency_keys  — 24h cache for Idempotency-Key header deduplication.
--    TTL sweep is handled by a Supabase scheduled function (not in-process).
-- 2. api_keys overrides — per-key rate-limit columns (NULL = use default).

-- ---------------------------------------------------------------------------
-- Idempotency cache
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS idempotency_keys (
    key          TEXT        PRIMARY KEY CHECK (char_length(key) <= 255),
    contract_id  UUID        NOT NULL,
    -- SHA-256 (hex) of the raw request body bytes.
    -- Same key + different hash → 422 conflict.
    body_hash    TEXT        NOT NULL,
    status_code  SMALLINT    NOT NULL,
    response     JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Minimum 24h retention; set at insert time.
    expires_at   TIMESTAMPTZ NOT NULL
);

-- Index for the Supabase scheduled cleanup function:
--   DELETE FROM idempotency_keys WHERE expires_at < now();
CREATE INDEX IF NOT EXISTS idempotency_keys_expires_at_idx
    ON idempotency_keys (expires_at);

-- ---------------------------------------------------------------------------
-- Per-key rate-limit overrides on api_keys
-- ---------------------------------------------------------------------------

ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS rate_limit_rps   INT CHECK (rate_limit_rps > 0),
    ADD COLUMN IF NOT EXISTS rate_limit_burst INT CHECK (rate_limit_burst > 0);

COMMENT ON COLUMN api_keys.rate_limit_rps   IS
    'Sustained request rate (req/sec). NULL = default (100). RFC-021.';
COMMENT ON COLUMN api_keys.rate_limit_burst IS
    'Token bucket burst capacity. NULL = default (1000). RFC-021.';
