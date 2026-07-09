-- backfill_schema_migrations.sql (2026-07-09 drift reconciliation)
--
-- Prod's supabase_migrations.schema_migrations tracks only 5 of the repo's
-- migrations, because most were applied by hand / via tooling that didn't
-- record them. This script backfills the ledger so the tracked history
-- matches supabase/migrations/*.sql.
--
-- IT DOES NOT RUN ANY MIGRATION. It only inserts bookkeeping rows for
-- migrations whose objects are already verified present in prod
-- (verified 2026-07-09 by schema inspection).
--
-- EXECUTED against prod 2026-07-09. Kept for audit trail; re-running is a
-- no-op (ON CONFLICT DO NOTHING).
--
-- NOT in this list (applied for real on 2026-07-09, auto-tracked with real
-- timestamps):
--   024_api_key_hash_algorithm_docs    (was missing from prod)
--   025_api_key_hash_length_check_safe (was missing from prod)
--   027_api_keys_server_side_issuance  (RFC-056; was missing from prod)
--
-- Naming notes:
--   * '028_stripe_billing' row below covers origin/main's 028 file; all its
--     objects (incl. orgs.plan_status + both stripe indexes) verified in prod.
--   * early_access is tracked as 'create_early_access' (20260504212436);
--     its retroactive repo file is 030_early_access.sql. Name mismatch in
--     the ledger is cosmetic and left as-is.
--
-- Already tracked (skipped): 003, 004, 005, create_early_access (= repo file
-- 027_early_access.sql), 029_stripe_failed_events.
--
-- Synthetic version timestamps: chosen to sort correctly against the five
-- existing rows. They are bookkeeping identifiers, not actual apply times.

INSERT INTO supabase_migrations.schema_migrations (version, name)
VALUES
    -- before 003 (20260420210749)
    ('20260420210700', '001_initial_schema'),
    ('20260420210710', '002_quarantine_and_p99'),
    -- after 005 (20260420210818), before create_early_access (20260504212436)
    ('20260420211000', '006_accounts_and_api_keys'),
    ('20260420211010', '007_org_scoped_tenancy'),
    ('20260420211020', '008_fix_rls_recursion'),
    ('20260420211030', '009_github_integration'),
    ('20260420211040', '010_odcs_import'),
    ('20260420211050', '011_v1_ingest'),
    ('20260420211100', '012_rfc_001_finish'),
    ('20260420211110', '013_fix_rls_recursion_after_012'),
    ('20260420211120', '014_kafka_ingress'),
    ('20260420211130', '015_kinesis_ingress'),
    ('20260420211140', '016_contract_queryability'),
    ('20260420211150', '017_egress_validation'),
    ('20260420211200', '018_egress_leakage_guard'),
    ('20260420211210', '019_provider_scorecard'),
    ('20260420211220', '020_contract_publication'),
    ('20260420211230', '021_contract_collaboration'),
    ('20260420211240', '022_public_catalog'),
    ('20260420211250', '023_rls_contract_versions_quarantine'),
    -- 026 shipped with RFC-045 plan gating (orgs_plan_valid verified in prod)
    ('20260515000000', '026_plan_tier'),
    -- 028 objects (orgs stripe cols + stripe_processed_events) predate 029
    ('20260607153900', '028_stripe_billing')
ON CONFLICT (version) DO NOTHING;
