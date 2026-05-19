-- Migration 025: re-add api_keys_key_hash_length as NOT VALID + VALIDATE
--
-- Migration 024 added the CHECK constraint inline, taking ACCESS EXCLUSIVE for
-- full-table validation.  On a busy table this blocks all reads/writes for the
-- duration, and any pre-existing row with length != 44 aborts the deploy.
--
-- This migration drops the original constraint (idempotent — safe whether 024
-- succeeded or not) and re-adds it using the two-step NOT VALID + VALIDATE
-- pattern that is safe under concurrent load.

alter table public.api_keys
    drop constraint if exists api_keys_key_hash_length;

-- Step 1: add as NOT VALID — brief lock, rejects future bad inserts immediately.
alter table public.api_keys
    add constraint api_keys_key_hash_length
        check (length(key_hash) = 44) not valid;

-- Step 2: validate existing rows — uses SHARE UPDATE EXCLUSIVE, allows
-- concurrent reads and writes.  If this errors, run:
--   SELECT id, length(key_hash) FROM api_keys WHERE length(key_hash) != 44;
-- to find the offending rows, fix them, then re-run this statement.
alter table public.api_keys
    validate constraint api_keys_key_hash_length;
