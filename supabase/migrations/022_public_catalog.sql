-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 022: Public Catalog (RFC-034)
--
-- Adds:
--   public_contracts  — curated gov/open-data sources with ContractGate
--                       YAML attached.  Admin-managed; no RLS (public read).
--   contracts.parent_public_contract_id — fork lineage back to a public source.
--   contracts.fork_filter  — JSONB {fields, predicates} applied at export time.
--
-- source_format values:
--   'json_rows'  Array-of-arrays JSON (row 0 = headers) — Census API default.
--   'json'       Array of objects [{col: val}, …].
--   'csv'        Comma/semicolon/tab-delimited text; delimiter auto-detected.
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. public_contracts table ─────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public_contracts (
    id              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name            text        NOT NULL UNIQUE,
    description     text,
    -- Upstream HTTP endpoint (no auth required — public sources only in v1).
    source_url      text        NOT NULL,
    source_format   text        NOT NULL DEFAULT 'json_rows'
                                CHECK (source_format IN ('json', 'json_rows', 'csv')),
    -- Canonical ContractGate YAML for this source.
    contract_yaml   text        NOT NULL,
    version         text        NOT NULL DEFAULT '1.0',
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- ── 2. Fork columns on contracts ──────────────────────────────────────────────

ALTER TABLE contracts
    ADD COLUMN IF NOT EXISTS parent_public_contract_id uuid
        REFERENCES public_contracts(id),
    ADD COLUMN IF NOT EXISTS fork_filter jsonb;
    -- fork_filter shape: { "fields": ["col1","col2"] | null,
    --                      "predicates": [{"field":"x","op":"eq","value":"y"}] }

CREATE INDEX IF NOT EXISTS idx_contracts_parent_public
    ON contracts (parent_public_contract_id)
    WHERE parent_public_contract_id IS NOT NULL;

-- ── 3. Seed: ACS 5-Year Estimates 2023 (Census Bureau) ───────────────────────
--
-- Endpoint docs: https://www.census.gov/data/developers/data-sets/acs/acs5.html
-- No API key needed for < 500 requests / day.
--
-- Variables requested:
--   NAME         County + state name string
--   B01003_001E  Total population estimate
--   B19013_001E  Median household income (USD, inflation-adjusted)
--   B19013_001MA Median household income margin of error (USD)
--   state        State FIPS code (2-char string, e.g. "01" = Alabama)
--   county       County FIPS code (3-char string, e.g. "001" = Autauga)
--
-- The Census API returns JSON array-of-arrays where the first element is the
-- header row — hence source_format = 'json_rows'.

INSERT INTO public_contracts (
    name,
    description,
    source_url,
    source_format,
    contract_yaml,
    version
) VALUES (
    'acs_5year_2023_county',
    'ACS 5-Year Estimates 2023 — county-level population and income. Source: US Census Bureau (data.census.gov). No API key required for < 500 requests/day.',
    'https://api.census.gov/data/2023/acs/acs5?get=NAME,B01003_001E,B19013_001E,B19013_001MA&for=county:*&in=state:*',
    'json_rows',
    $yaml$
version: "1.0"
name: "acs_5year_2023_county"
description: "ACS 5-Year Estimates 2023 — county-level population and income. Source: US Census Bureau."

ontology:
  entities:
    - name: NAME
      type: string
      required: true
    - name: B01003_001E
      type: integer
      required: true
      min: 0
    - name: B19013_001E
      type: integer
      required: false
    - name: B19013_001MA
      type: integer
      required: false
    - name: state
      type: string
      required: true
    - name: county
      type: string
      required: true

glossary:
  - field: NAME
    description: "County and state name (e.g. Autauga County, Alabama)"
    constraints: "non-empty string"
  - field: B01003_001E
    description: "Total population estimate"
    constraints: "non-negative integer"
  - field: B19013_001E
    description: "Median household income (USD, inflation-adjusted)"
    constraints: "-666666666 indicates data not available for this county"
  - field: B19013_001MA
    description: "Median household income margin of error (USD)"
    constraints: "null when income estimate unavailable"
  - field: state
    description: "State FIPS code (2-digit string, e.g. 01 for Alabama)"
    constraints: "always 2 characters, zero-padded"
  - field: county
    description: "County FIPS code (3-digit string, e.g. 001 for Autauga County AL)"
    constraints: "always 3 characters, zero-padded"
$yaml$,
    '1.0'
) ON CONFLICT (name) DO NOTHING;
