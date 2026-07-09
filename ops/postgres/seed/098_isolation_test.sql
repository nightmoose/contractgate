-- ContractGate — Compose-only isolation-test seed (RFC-073).
--
-- Seeds the fixtures that tests/rfc_001_isolation.rs::cross_org_ingest_is_rejected
-- needs: two orgs, one contract owned by org A (with a live stable version),
-- and one API key per org. The raw keys are committed as test fixtures in
-- tests/compose_demo_smoke.sh — they grant access to nothing but this
-- throwaway compose database.
--
-- Lives under ops/postgres/seed/ — mounted into the compose Postgres only,
-- applied by ops/postgres/initdb-wrapper.sh AFTER all migrations. NOT part of
-- supabase/migrations/ and never runs in real Supabase.
--
-- Runs before 099_demo_org.sql (numeric order). Fixed, memorable UUIDs:
--   org A      aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa
--   org B      bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb
--   contract A a0000000-0000-0000-0000-000000000001
--   user A/B   a0000000-…-00a / b0000000-…-00b (api_keys.user_id FK target)
--
-- api_keys.key_hash = base64(SHA-256(raw_key))  (migration 024 scheme).
--   TEST_API_KEY_A = cg_live_orgA_isolationtest_000000000001
--     prefix cg_live_orgA  hash 4NsMO1Zse0aEiNQ1A4a+wcHggWWHYJbYY/LqYkYaT7E=
--   TEST_API_KEY_B = cg_live_orgB_isolationtest_000000000002
--     prefix cg_live_orgB  hash EEr2wK1BxOubLC0prqT40FeuX0pWBwG2ihy9D+avBrs=
-- If you change a raw key, recompute the hash and update both here and the
-- smoke script.

-- ── auth.users stand-ins (FK target for api_keys.user_id) ───────────────────
-- The compose Postgres has a stubbed auth.users (migration 006).
INSERT INTO auth.users (id, email)
VALUES
    ('a0000000-0000-0000-0000-0000000000aa', 'orga-iso@example.test'),
    ('b0000000-0000-0000-0000-0000000000bb', 'orgb-iso@example.test')
ON CONFLICT (id) DO NOTHING;

-- ── orgs ────────────────────────────────────────────────────────────────────
INSERT INTO public.orgs (id, name, slug, plan, created_at)
VALUES
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa', 'Isolation Test Org A', 'iso-org-a', 'free', NOW()),
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb', 'Isolation Test Org B', 'iso-org-b', 'free', NOW())
ON CONFLICT (id) DO NOTHING;

-- ── contract owned by org A ─────────────────────────────────────────────────
-- Post-migration-003, `contracts` has no version/active/yaml_content columns —
-- those moved to contract_versions. The contract row is identity + org scope;
-- the YAML lives on the version row below.
INSERT INTO public.contracts (id, org_id, name, description, created_at, updated_at)
VALUES (
    'a0000000-0000-0000-0000-000000000001',
    'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
    'iso_test_contract',
    'RFC-073 isolation fixture',
    NOW(),
    NOW()
)
ON CONFLICT (id) DO NOTHING;

-- Live stable version so unpinned /v1/ingest resolves a version for the
-- contract (the request still gets rejected by org scope for org B's key,
-- but org A's key path needs a resolvable version for the sibling tests).
INSERT INTO public.contract_versions (id, contract_id, version, state, yaml_content, created_at, promoted_at)
VALUES (
    'a0000000-0000-0000-0000-0000000000c1',
    'a0000000-0000-0000-0000-000000000001',
    '1.0.0',
    'stable',
    E'version: "1.0"\nname: "iso_test_contract"\ndescription: "RFC-073 isolation fixture"\nontology:\n  entities:\n    - name: user_id\n      type: string\n      required: true\nglossary: []\nmetrics: []\n',
    NOW(),
    NOW()
)
ON CONFLICT (contract_id, version) DO NOTHING;

-- ── one API key per org ─────────────────────────────────────────────────────
INSERT INTO public.api_keys (id, user_id, org_id, name, key_prefix, key_hash, created_at)
VALUES
    ('a0000000-0000-0000-0000-0000000000a1',
     'a0000000-0000-0000-0000-0000000000aa',
     'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
     'iso-test-key-a', 'cg_live_orgA',
     '4NsMO1Zse0aEiNQ1A4a+wcHggWWHYJbYY/LqYkYaT7E=', NOW()),
    ('b0000000-0000-0000-0000-0000000000b1',
     'b0000000-0000-0000-0000-0000000000bb',
     'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb',
     'iso-test-key-b', 'cg_live_orgB',
     'EEr2wK1BxOubLC0prqT40FeuX0pWBwG2ihy9D+avBrs=', NOW())
ON CONFLICT (id) DO NOTHING;
