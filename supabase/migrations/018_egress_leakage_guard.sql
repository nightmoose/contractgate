-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 018: Egress PII & Leakage Guard (RFC-030)
--
-- Adds the `egress_leakage_mode` column to `contract_versions` so each
-- version can declare how undeclared fields in outbound payloads are handled:
--
--   'off'   — pass through untouched (backwards-compatible default)
--   'strip' — remove from response; names recorded in per-record outcome
--   'fail'  — treat each undeclared field as a LeakageViolation, subject
--              to the RFC-029 disposition (block / fail / tag)
--
-- No new salt column: RFC-004's `contracts.pii_salt` is reused verbatim.
-- Hash output for a given value is identical on ingest and egress — downstream
-- join keys remain consistent across both directions (RFC-030 §Salt continuity).
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE contract_versions
    ADD COLUMN IF NOT EXISTS egress_leakage_mode text NOT NULL DEFAULT 'off'
    CHECK (egress_leakage_mode IN ('off', 'strip', 'fail'));

COMMENT ON COLUMN contract_versions.egress_leakage_mode IS
    'RFC-030: controls egress leakage enforcement. '
    'off = undeclared fields pass through; '
    'strip = undeclared fields removed silently; '
    'fail = undeclared fields become LeakageViolation, subject to RFC-029 disposition.';
