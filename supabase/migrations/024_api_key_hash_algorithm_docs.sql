-- Migration 024: correct api_keys.key_hash column comment + add length CHECK
--
-- Migration 006 documented key_hash as "bcrypt (cost 10)" but the actual
-- implementation (dashboard SubtleCrypto + api_key_auth.rs) has always used
-- SHA-256 base64.  RFC-041 / P0-3 fixes the mismatch.
--
-- A CHECK constraint on length (44 = base64(32 bytes)) catches any future
-- attempt to store a different hash format at the DB layer.

comment on column public.api_keys.key_hash is
    'SHA-256 hash of the raw key, base64-encoded (standard alphabet, with '
    'padding). Computed client-side via SubtleCrypto and verified server-side '
    'by api_key_auth.rs using the same SHA-256/base64 digest.  The raw key is '
    'never stored.  High-entropy keys (224+ bits) make SHA-256 equivalent to '
    'bcrypt for this use case.';

alter table public.api_keys
    add constraint api_keys_key_hash_length
        check (length(key_hash) = 44);
