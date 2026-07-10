-- 031_security_advisor_fixes.sql
--
-- Fixes every ERROR/WARN raised by the Supabase security advisor as of the
-- 2026-07-09 maintenance sweep (see REVIEW-2026-07-09-maintenance-sweep.md).
-- Five independent fixes, documented in place. INFO-level items get a
-- COMMENT ON TABLE only (no behavior change). "Leaked password protection"
-- is an Auth dashboard toggle, not SQL — out of scope here.

-- ─────────────────────────────────────────────────────────────────────────────
-- 1. SECURITY DEFINER views (ERROR, 4) → security_invoker = true
--
-- Views created under a superuser/owner role evaluate the underlying tables'
-- RLS policies as the view OWNER, not the querying session — i.e. any grantee
-- sees every row the owner can see, regardless of the caller's own RLS.
-- `security_invoker = true` (PG 15+; prod is PG 17) makes the view re-evaluate
-- RLS as the calling role instead.
--
-- provider_scorecard / provider_field_health (migration 019): read ONLY by
-- the Rust backend via `DATABASE_URL` (src/scorecard.rs, sqlx::PgPool) — a
-- direct Postgres connection, not a PostgREST session, so it is unaffected
-- by security_invoker either way. No dashboard code queries these views
-- (verified: no references in dashboard/). Safe to flip.
--
-- active_contracts_public (migration 016): GRANT SELECT ... TO authenticated
-- only — there is no `TO anon` grant anywhere in the migration history, so
-- despite the "public" name this was never meant for anonymous reads; it is
-- an internal auditor view. Underlying `contracts` / `contract_versions` RLS
-- was tightened to per-org policies by migrations 007/012/013 (see
-- get_my_org_ids()), well after this view was written against the original
-- migration-001/003 blanket "auth_all USING (true)" policies. Net effect
-- today: this view is a live cross-org leak — any authenticated user, from
-- any org, can currently read every org's deployed contract YAML
-- (parsed_json) through it. security_invoker = true closes that: reads
-- become scoped to the caller's own org, matching every other authenticated
-- surface in the app. There is no anon path to preserve.
--
-- v_ingestion_summary (migration 001/003): same reasoning as
-- active_contracts_public — no application code queries it directly
-- (grep: zero hits in src/ or dashboard/), so it exists for ad-hoc
-- PostgREST/reporting access by authenticated users. Scoping it to the
-- caller's own org via security_invoker is the correct, not just
-- lint-satisfying, fix.

ALTER VIEW public.provider_scorecard      SET (security_invoker = true);
ALTER VIEW public.provider_field_health   SET (security_invoker = true);
ALTER VIEW public.active_contracts_public SET (security_invoker = true);
ALTER VIEW public.v_ingestion_summary     SET (security_invoker = true);

-- ─────────────────────────────────────────────────────────────────────────────
-- 2. provider_field_baseline: drop the blanket authenticated policy
--
-- Migration 019 gave `authenticated` FOR ALL USING (true) WITH CHECK (true)
-- on this table. It has no org_id column (keyed by `source`), so today any
-- signed-in user from any org can read AND write every provider's rolling
-- drift baseline. The only writer is the Rust `scorecard-rollup` job via
-- `DATABASE_URL` (service role / direct pool — verified in src/scorecard.rs);
-- no dashboard code queries provider_field_baseline directly (grep: zero
-- hits outside src/scorecard.rs). Dropping the authenticated policy removes
-- the leak with no functional loss (option (a) from the worklist).

DROP POLICY IF EXISTS "auth_all" ON public.provider_field_baseline;
-- "service_all" (service_role, FOR ALL USING/CHECK true) is left in place —
-- that is how the rollup job reads and upserts baselines.

-- ─────────────────────────────────────────────────────────────────────────────
-- 3. Anon-executable SECURITY DEFINER functions (WARN)
--
-- handle_new_user() and get_my_org_ids() are trigger/helper functions that
-- should never be invoked directly via PostgREST's /rest/v1/rpc/ endpoint.
--
--   handle_new_user()  — trigger-only (on_auth_user_created on auth.users,
--                        migrations 006/007/012). Revoke from anon AND
--                        authenticated; Postgres trigger firing does not
--                        need an EXECUTE grant to the invoking session role.
--   get_my_org_ids()   — must stay executable by `authenticated`: every
--                        org-scoped RLS policy in this schema calls it as
--                        the querying user (migrations 008/013). Only the
--                        `anon` grant is a mistake — revoke that one only.

REVOKE EXECUTE ON FUNCTION public.handle_new_user() FROM anon, authenticated;
REVOKE EXECUTE ON FUNCTION public.get_my_org_ids()  FROM anon;

-- rls_auto_enable(): flagged by the 2026-07-09 advisor scan but NOT defined
-- by any migration file in this repo — grepped the full history, no match.
-- It may have been created directly against prod outside of migrations (the
-- same class of drift migration 030 reconciled). Revoke defensively, by
-- signature lookup, so this is a no-op on a fresh CI database (where the
-- function doesn't exist) and effective on prod (where the advisor says it
-- does). Flagged for Alex: if this fires on prod, the function should be
-- reverse-engineered into a proper migration file so it stops being drift.
DO $$
DECLARE
    fn_sig regprocedure;
BEGIN
    SELECT p.oid::regprocedure INTO fn_sig
    FROM pg_proc p
    JOIN pg_namespace n ON n.oid = p.pronamespace
    WHERE p.proname = 'rls_auto_enable'
      AND n.nspname = 'public'
    LIMIT 1;

    IF fn_sig IS NOT NULL THEN
        EXECUTE format('REVOKE EXECUTE ON FUNCTION %s FROM anon, authenticated', fn_sig);
        RAISE NOTICE 'rls_auto_enable() found and revoked from anon, authenticated: %', fn_sig;
    ELSE
        RAISE NOTICE 'rls_auto_enable() not found in public schema — nothing to revoke (expected on fresh/CI databases).';
    END IF;
END;
$$;

-- ─────────────────────────────────────────────────────────────────────────────
-- 4. Mutable search_path (WARN, 8 functions)
--
-- Trigger functions without a pinned search_path are resolvable-object
-- hijack risk if a role can create objects earlier in the caller's
-- search_path. All eight are argument-less trigger functions, so the
-- signature is unambiguous.

ALTER FUNCTION public.contract_versions_immutability_guard()     SET search_path = public;
ALTER FUNCTION public.contract_versions_delete_guard()           SET search_path = public;
ALTER FUNCTION public.contracts_name_history_trigger()           SET search_path = public;
ALTER FUNCTION public.quarantine_replay_stamp_guard()            SET search_path = public;
ALTER FUNCTION public.contract_versions_compliance_mode_guard()  SET search_path = public;
ALTER FUNCTION public.set_updated_at()                           SET search_path = public;
ALTER FUNCTION public.guard_api_key_hash_immutable()             SET search_path = public;
ALTER FUNCTION public.update_updated_at()                        SET search_path = public;

-- ─────────────────────────────────────────────────────────────────────────────
-- 5. RLS-enabled-no-policy tables (INFO, intentional — service-role only)
--
-- No policy change; documenting intent so future advisor runs are
-- self-explaining instead of re-flagging these every sweep.

COMMENT ON TABLE public.idempotency_keys IS
    'Service-role only by design: no RLS policy for anon/authenticated. Written by the Rust ingest path to dedupe replayed events.';
COMMENT ON TABLE public.public_contracts IS
    'Service-role only by design: no RLS policy for anon/authenticated. Admin-managed catalog seed data (see migration 022); reads go through the Rust backend, not PostgREST directly.';
COMMENT ON TABLE public.stripe_processed_events IS
    'Service-role only by design: no RLS policy for anon/authenticated. Written by the Stripe webhook handler for idempotency (migration 028).';
COMMENT ON TABLE public.stripe_failed_events IS
    'Service-role only by design: no RLS policy for anon/authenticated. Failure-visibility log written by the Stripe webhook handler (migration 029).';
