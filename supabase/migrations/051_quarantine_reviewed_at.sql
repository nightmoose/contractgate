-- Migration 051: quarantine_events.reviewed_at
--
-- `POST /contracts/{id}/quarantine/review` sets `reviewed_at = now()`.
-- Migration 002 declared the column inside `CREATE TABLE IF NOT EXISTS`.
-- A database whose `quarantine_events` table already existed when 002 ran
-- never gained the column, and the review UPDATE fails with
-- `column "reviewed_at" does not exist` (SQLSTATE 42703). The gateway
-- returns that as a generic 500.
--
-- Additive. Existing rows stay `reviewed_at` NULL until they are reviewed.
-- Safe to run more than once.

alter table public.quarantine_events
    add column if not exists reviewed_at timestamptz;

comment on column public.quarantine_events.reviewed_at is
    'When a pending row was marked reviewed. NULL until POST /contracts/{id}/quarantine/review.';
