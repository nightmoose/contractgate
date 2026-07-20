-- ContractGate — Migration 034: Gated event-payload storage (RFC-086)
-- Run after 033_slack_leads.sql
--
-- Store customer event bodies only when the owning org is on a paid plan AND
-- has opted in (org master switch), with an optional per-contract opt-out.
-- Otherwise persist the metadata row with an empty/NULL body and a redacted
-- marker. See docs/rfcs/086-gated-event-payload-storage.md.
--
-- Additive + one NOT NULL relaxation. No data is deleted.

-- ---------------------------------------------------------------------------
-- 1. Storage opt-in flags
-- ---------------------------------------------------------------------------
--
-- Org master switch: default OFF — nobody stores bodies until they choose to.
-- Per-contract override: default TRUE (inherit the org switch); set FALSE to
-- opt a single pipeline out. Only consulted when the org switch is ON.

ALTER TABLE public.orgs
    ADD COLUMN IF NOT EXISTS store_event_payloads BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE public.contracts
    ADD COLUMN IF NOT EXISTS store_event_payloads BOOLEAN NOT NULL DEFAULT true;

-- ---------------------------------------------------------------------------
-- 2. Allow metadata-only quarantine rows
-- ---------------------------------------------------------------------------
--
-- payload was NOT NULL with no default. Metadata-only and purged rows need a
-- NULL body. audit_log.raw_event already carries NOT NULL DEFAULT '{}'::jsonb
-- (001_initial_schema.sql), so redacted audit rows write '{}' — no change here.

ALTER TABLE public.quarantine_events
    ALTER COLUMN payload DROP NOT NULL;

-- ---------------------------------------------------------------------------
-- 3. Redacted markers (audit honesty)
-- ---------------------------------------------------------------------------
--
-- Distinguish "body intentionally not stored / purged" from a genuinely empty
-- or NULL payload. A NULL/`{}` body must never be mistaken for captured data.

ALTER TABLE public.quarantine_events
    ADD COLUMN IF NOT EXISTS payload_redacted BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE public.audit_log
    ADD COLUMN IF NOT EXISTS raw_event_redacted BOOLEAN NOT NULL DEFAULT false;

-- ---------------------------------------------------------------------------
-- Notes
-- ---------------------------------------------------------------------------
-- Purge (RFC-086 §4) redacts in place — UPDATE payload=NULL / raw_event='{}'
-- and sets the *_redacted marker. It NEVER deletes a row. audit_log and
-- quarantine_events rows and all their metadata are always retained.
