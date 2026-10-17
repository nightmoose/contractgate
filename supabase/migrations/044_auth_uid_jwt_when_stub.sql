-- 044: restore a JWT-aware auth.uid() only when 006 left the NULL stub.
--
-- Do not edit 006. Hosted projects already applied it, and changing an
-- applied file breaks migration checksums.
--
-- On hosted Supabase, auth.uid() reads the request JWT. This file sees that
-- body and does nothing. On plain Postgres, 006 replaces auth.uid() with
-- `SELECT NULL::uuid`, which would make every RLS check fail. This file
-- replaces only that stub.
--
-- An owner must apply this on production. Until then it stays listed in
-- supabase/unapplied-migrations.txt. Applying it on hosted Supabase is a
-- no-op when auth.uid() is not the stub. A fresh hosted apply can still
-- fail inside 006 itself (permission denied for schema auth); this file
-- cannot change that.

DO $$
DECLARE
    def text;
    compact text;
BEGIN
    SELECT pg_get_functiondef('auth.uid()'::regprocedure) INTO def;
    IF def IS NULL THEN
        RETURN;
    END IF;
    compact := lower(regexp_replace(def, '\s+', '', 'g'));
    IF compact NOT LIKE '%selectnull::uuid%' THEN
        RETURN;
    END IF;
    IF position('current_setting' IN compact) > 0 OR position('request.jwt' IN compact) > 0 THEN
        RETURN;
    END IF;

    EXECUTE $fn$
        CREATE OR REPLACE FUNCTION auth.uid()
        RETURNS uuid
        LANGUAGE sql
        STABLE
        AS $body$
            SELECT COALESCE(
                NULLIF(current_setting('request.jwt.claim.sub', true), ''),
                NULLIF(current_setting('request.jwt.claims', true), '')::jsonb ->> 'sub'
            )::uuid
        $body$
    $fn$;
EXCEPTION
    WHEN insufficient_privilege OR undefined_function OR undefined_object OR invalid_schema_name THEN
        NULL;
END $$;
