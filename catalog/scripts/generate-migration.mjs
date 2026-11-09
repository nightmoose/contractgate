/**
 * Generate supabase/migrations/048_public_catalog_seed_refresh.sql from the
 * reviewable seed files in catalog/seeds/<slug>/.
 *
 * Migration 047 already shipped the original 12 rows and is not rewritten.
 * This file is an idempotent upsert so a database that applied 047 picks up
 * later seed corrections. Re-run after changing any seed:
 *
 *   node catalog/scripts/generate-migration.mjs
 *
 * This only writes a .sql file; it never touches a live database.
 */
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const seedsDir = join(here, "..", "seeds");
const outFile = join(here, "..", "..", "supabase", "migrations", "048_public_catalog_seed_refresh.sql");

// Launch order (ACS last — it is an UPDATE of the existing 022 row).
const SLUGS = [
  "stripe-charges",
  "stripe-customers",
  "shopify-orders",
  "quickbooks-invoices",
  "hubspot-contacts",
  "salesforce-account",
  "salesforce-opportunity",
  "ga4-bigquery-events",
  "square-payments",
  "xero-invoices",
  "generic-sheets-transactions-csv",
  "acs-5year-2023-county",
];

function sqlStr(s) {
  if (s === null || s === undefined) return "NULL";
  return `'${String(s).replace(/'/g, "''")}'`;
}

function upsertFor(slug) {
  const dir = join(seedsDir, slug);
  const meta = JSON.parse(readFileSync(join(dir, "meta.json"), "utf8"));
  const yaml = readFileSync(join(dir, "contract.yaml"), "utf8");
  const samples = readFileSync(join(dir, "samples.json"), "utf8").trim();
  const name = meta.name ?? meta.slug; // DB name column; ACS keeps its 022 name

  return `-- ── ${slug} (${meta.vendor} · ${meta.category}) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    ${sqlStr(name)},
    ${sqlStr(meta.slug)},
    ${sqlStr(meta.description)},
    ${sqlStr(meta.vendor)},
    ${sqlStr(meta.category)},
    ${meta.source_url ? sqlStr(meta.source_url) : "NULL"},
    ${sqlStr(meta.source_format)},
    $yaml$
${yaml.trimEnd()}
$yaml$,
    ${sqlStr(meta.version ?? "1.0")},
    ${sqlStr(meta.verified_source_url)},
    ${sqlStr(meta.verified_at)}::date,
    $json$${samples}$json$::jsonb,
    ${sqlStr(meta.license_note)},
    NULL
)
ON CONFLICT (name) DO UPDATE SET
    slug                = EXCLUDED.slug,
    description         = EXCLUDED.description,
    vendor             = EXCLUDED.vendor,
    category           = EXCLUDED.category,
    source_url         = EXCLUDED.source_url,
    source_format      = EXCLUDED.source_format,
    contract_yaml      = EXCLUDED.contract_yaml,
    version            = EXCLUDED.version,
    verified_source_url = EXCLUDED.verified_source_url,
    verified_at        = EXCLUDED.verified_at,
    sample_json        = EXCLUDED.sample_json,
    license_note       = EXCLUDED.license_note,
    updated_at         = now();
`;
}

const header = `-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 048: Public catalog seed refresh
--
-- GENERATED FILE — do not hand-edit. Source of truth is catalog/seeds/<slug>/.
-- Regenerate with:  node catalog/scripts/generate-migration.mjs
--
-- 047 already created the columns and inserted the original 12 rows. This
-- upserts those same slugs with the corrected contracts, samples, citations,
-- and blurbs. Idempotent: a fresh database applies 047, then this overwrites
-- the rows with the current seeds.
-- ─────────────────────────────────────────────────────────────────────────────

`;

const body = SLUGS.map(upsertFor).join("\n");
writeFileSync(outFile, header + "\n" + body);
console.log(`wrote ${outFile}`);
