-- Migration 027: api_keys RLS tightening — server-side issuance (RFC-056)
--
-- Context: RFC-056 moves API key issuance from a direct browser INSERT into
-- a Next.js route handler (dashboard/app/api/keys/route.ts) that runs with
-- the service-role key.  Now that issuance and revocation are server-only,
-- the `authenticated` role no longer needs INSERT or UPDATE on api_keys.
--
-- Rollout ordering: this migration is applied AFTER the route.ts handler is
-- deployed.  The window between route deployment and migration apply is
-- momentary, and during that window the old browser INSERT path continues to
-- work (INSERT policy still exists).  Once the migration lands, both the old
-- browser path and any direct REST INSERT are rejected by RLS.  See RFC-056.
--
-- RLS changes:
--   SELECT  authenticated  — keep; users can read their org's key metadata.
--           key_hash is NOT in the dashboard query (only id, name, key_prefix,
--           created_at, last_used_at, revoked_at, is_active); we document the
--           column exposure here and rely on the service-role route to never
--           return key_hash to the browser.  A column-level GRANT revoke is
--           not available in Supabase's RLS model without view indirection;
--           adding a security_invoker view is deferred to a future migration.
--
--   INSERT  authenticated  — REVOKED.  Issuance now goes through service role.
--   UPDATE  authenticated  — REVOKED.  Revocation now goes through service role.
--
--   service_role — unchanged (select + update for validation / last_used_at).
--   New: service_role INSERT — added so route.ts can insert rows.
--
-- get_my_org_ids() is used for org scoping (per project convention —
-- inline org_memberships subqueries cause PG 42P17 recursion, see migration 008).

-- ── 1. DROP the authenticated INSERT and UPDATE policies ──────────────────────

DROP POLICY IF EXISTS "users can create own api keys" ON public.api_keys;
DROP POLICY IF EXISTS "users can revoke own api keys" ON public.api_keys;

-- ── 2. Re-scope the SELECT policy to use get_my_org_ids() ────────────────────
-- (Previous policy from migration 013 used auth.uid() = user_id; widen to
-- org scope so all members of the org can see the key list.)

DROP POLICY IF EXISTS "users can read own api keys" ON public.api_keys;

CREATE POLICY "org members can read api key metadata"
    ON public.api_keys
    FOR SELECT
    TO authenticated
    USING (
        org_id IN (SELECT public.get_my_org_ids())
        AND deleted_at IS NULL
    );

-- ── 3. Add service_role INSERT policy (route.ts uses service role) ────────────

CREATE POLICY "service role can insert api keys"
    ON public.api_keys
    FOR INSERT
    TO service_role
    WITH CHECK (true);

-- ── 4. Ensure service_role UPDATE policy covers revocation ───────────────────
-- (Policy "service role can update last_used_at" from migration 007 already
-- covers this; no change needed. Verify its existence here via a DO block
-- so CI catches any drift.)

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM   pg_policies
        WHERE  schemaname = 'public'
          AND  tablename  = 'api_keys'
          AND  policyname = 'service role can update last_used_at'
    ) THEN
        RAISE EXCEPTION
            'Expected policy "service role can update last_used_at" on api_keys not found. '
            'Check migration 007 / 012 applied correctly.';
    END IF;
END $$;

-- ── 5. Comment documenting key_hash exposure posture ─────────────────────────

COMMENT ON COLUMN public.api_keys.key_hash IS
    'SHA-256 of the raw key, base64-encoded (standard). '
    'Verified by api_key_auth.rs on each cache-miss. '
    'The raw key is never stored. '
    'key_hash is readable by service_role (for validation) and by authenticated '
    'users via the SELECT policy above, but the dashboard SELECT query '
    '(id, name, key_prefix, created_at, last_used_at, revoked_at, is_active) '
    'deliberately omits key_hash so it is never sent to the browser. '
    'RFC-056: column-level GRANT restriction deferred to a future security-hardening migration.';
