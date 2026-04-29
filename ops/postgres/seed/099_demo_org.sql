-- ContractGate — Compose-only demo org.
--
-- Inserts a single fixed-UUID org so that:
--   * tests/compose_smoke.sh can POST a contract with `x-org-id: <uuid>`
--   * the demo-seeder service (RFC-017) can do the same without going
--     through a full sign-up + first-org-provision flow.
--
-- This file lives under ops/postgres/seed/ — it is mounted into the
-- compose Postgres container only.  It is NOT part of supabase/migrations/
-- and never runs in real Supabase deployments.
--
-- The UUID is intentionally memorable (`cccccccc-...`) and is referenced
-- as a literal in tests/compose_smoke.sh and in docker-compose.yml's
-- demo-seeder env (`CONTRACTGATE_ORG_ID`).  If you change it, change all
-- three places.
INSERT INTO public.orgs (id, name, slug, plan, created_at)
VALUES (
    'cccccccc-cccc-cccc-cccc-cccccccccccc',
    'ContractGate Demo Org',
    'cg-demo',
    'free',
    NOW()
)
ON CONFLICT (id) DO NOTHING;
