/**
 * Generate supabase/migrations/047_public_catalog_expansion.sql from the
 * reviewable seed files in catalog/seeds/<slug>/.
 *
 * Keeping the migration generated (never hand-edited) guarantees the rows that
 * land in staging/prod match the in-repo seed files the tests validate. Re-run
 * after changing any seed:
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
const outFile = join(here, "..", "..", "supabase", "migrations", "047_public_catalog_expansion.sql");

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
-- Migration 047: Public Catalog phase 1 expansion
--
-- GENERATED FILE — do not hand-edit. Source of truth is catalog/seeds/<slug>/.
-- Regenerate with:  node catalog/scripts/generate-migration.mjs
--
-- Adds the provenance / discovery columns the public catalog pages need, makes
-- source_url nullable (vendor handover contracts are file-based — there is no
-- public unauthenticated data URL to fetch), backfills the existing ACS row,
-- and upserts all 12 launch contracts (11 vendor/generic + ACS).
--
-- Do NOT apply this to prod/staging until sign-off. It belongs in the PR only.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. New columns on public_contracts ───────────────────────────────────────

ALTER TABLE public_contracts
    ADD COLUMN IF NOT EXISTS slug                text,
    ADD COLUMN IF NOT EXISTS vendor              text,
    ADD COLUMN IF NOT EXISTS category            text,
    ADD COLUMN IF NOT EXISTS verified_source_url text,
    ADD COLUMN IF NOT EXISTS verified_at         date,
    ADD COLUMN IF NOT EXISTS sample_json         jsonb,
    ADD COLUMN IF NOT EXISTS license_note        text,
    ADD COLUMN IF NOT EXISTS author_org_id       uuid REFERENCES public.orgs(id);

COMMENT ON COLUMN public_contracts.slug IS
    'URL-safe public catalog slug, e.g. stripe-charges. NULL = not published to the public directory.';
COMMENT ON COLUMN public_contracts.author_org_id IS
    'Community author org (phase 2). NULL = authored by DataContractGate.';

CREATE UNIQUE INDEX IF NOT EXISTS idx_public_contracts_slug
    ON public_contracts (slug)
    WHERE slug IS NOT NULL;

-- ── 2. source_url becomes nullable (file-based vendor handover contracts) ──────
-- ACS keeps a real source_url so its live CSV export still works; vendor
-- handover contracts (Stripe, Shopify, …) have NULL source_url — the client
-- hands over a file, there is nothing to fetch. The docs citation lives in
-- verified_source_url instead.

ALTER TABLE public_contracts ALTER COLUMN source_url DROP NOT NULL;

-- ── 3. Upsert the 12 launch contracts ─────────────────────────────────────────
`;

const body = SLUGS.map(upsertFor).join("\n");
writeFileSync(outFile, header + "\n" + body);
console.log(`wrote ${outFile}`);
