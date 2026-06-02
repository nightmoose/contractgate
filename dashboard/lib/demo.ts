/**
 * Demo-mode constants — single source of truth for RFC-023.
 *
 * NEXT_PUBLIC_DEMO_MODE is inlined by Next.js at build time, so the
 * three chokepoints (middleware, AuthGate, OrgProvider) all share the
 * same compile-time flag without any runtime fetch.
 *
 * DEMO_ORG_UUID must match:
 *   - ops/postgres/seed/099_demo_org.sql  (INSERT INTO orgs)
 *   - demo-seeder x-org-id header
 *   - docker-compose.yml DEMO_ORG_ID env var for seeder
 */

export const DEMO_MODE = process.env.NEXT_PUBLIC_DEMO_MODE === "1";

export const DEMO_ORG_UUID = "cccccccc-cccc-cccc-cccc-cccccccccccc";
export const DEMO_ORG_NAME = "Demo Org";
