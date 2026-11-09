-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 050: Contract families
--
-- 049 is quarantine batch_id. This file is the next number.
--
-- A family groups contracts that describe the same data as it changes shape
-- through a pipeline. Each link records which contract this one is derived
-- from and which boundary it sits at:
--   ingest  — source → raw          (POST /v1/ingest)
--   stage   — between stages        (POST /v1/ingest on the next shape)
--   egress  — mart → partner / API  (POST /egress)
--
-- Private contracts: contract_families + contract_family_links (org-scoped).
-- Public catalog:    public_contract_families + public_contract_family_links.
--
-- Seeds the Stripe charges chain from examples/contract-chain/stripe/ as the
-- first public catalog family (stripe-charges-pipeline). Not affiliated with
-- Stripe. Sample ids in the YAML and sample_json are fake.
--
-- Idempotent: catalog rows upsert on name; the family upserts on slug; links
-- upsert on (family_id, public_contract_id).
-- ─────────────────────────────────────────────────────────────────────────────

-- ── 1. Private (org-scoped) families ─────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.contract_families (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      uuid        NOT NULL REFERENCES public.orgs(id) ON DELETE CASCADE,
    name        text        NOT NULL,
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT contract_families_org_name_unique UNIQUE (org_id, name)
);

COMMENT ON TABLE public.contract_families IS
    'Named group of private contracts that describe one dataset as it moves through a pipeline. Org-scoped. Service-role only: the gateway reads and writes these; anon and authenticated have no policy.';

COMMENT ON COLUMN public.contract_families.org_id IS
    'Owning org. Family reads and writes are limited to this org, matching contract identity routes.';

CREATE INDEX IF NOT EXISTS contract_families_org_id_idx
    ON public.contract_families (org_id);

CREATE TABLE IF NOT EXISTS public.contract_family_links (
    id                       uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id                uuid        NOT NULL REFERENCES public.contract_families(id) ON DELETE CASCADE,
    contract_id              uuid        NOT NULL REFERENCES public.contracts(id) ON DELETE CASCADE,
    -- Previous contract in the chain. NULL on the root.
    derived_from_contract_id uuid        REFERENCES public.contracts(id) ON DELETE CASCADE,
    -- ingest | stage (between stages) | egress
    boundary                 text        NOT NULL
        CHECK (boundary IN ('ingest', 'stage', 'egress')),
    position                 integer     NOT NULL CHECK (position >= 0),
    created_at               timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT contract_family_links_family_contract_unique UNIQUE (family_id, contract_id),
    CONSTRAINT contract_family_links_family_position_unique UNIQUE (family_id, position),
    CONSTRAINT contract_family_links_no_self_parent
        CHECK (derived_from_contract_id IS NULL OR derived_from_contract_id <> contract_id)
);

COMMENT ON TABLE public.contract_family_links IS
    'One row per contract in a family. derived_from_contract_id is the parent contract. boundary is ingest, stage (between stages), or egress. A contract belongs to at most one family. Service-role only.';

COMMENT ON COLUMN public.contract_family_links.boundary IS
    'Pipeline boundary for this contract: ingest, stage (between stages), or egress.';

COMMENT ON COLUMN public.contract_family_links.derived_from_contract_id IS
    'Contract this member is derived from. NULL for the root of the chain.';

-- A contract is in at most one family, so the catalog and the API can show one chain.
CREATE UNIQUE INDEX IF NOT EXISTS contract_family_links_contract_unique
    ON public.contract_family_links (contract_id);

CREATE INDEX IF NOT EXISTS contract_family_links_family_position_idx
    ON public.contract_family_links (family_id, position);

-- ── 2. Public catalog families ───────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS public.public_contract_families (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text        NOT NULL UNIQUE,
    slug        text        NOT NULL UNIQUE,
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.public_contract_families IS
    'Named chain of public catalog contracts. Admin-seeded. Service-role only; reads go through the gateway, not PostgREST. author_org_id stays NULL — this is not community publishing.';

CREATE TABLE IF NOT EXISTS public.public_contract_family_links (
    id                              uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id                       uuid        NOT NULL REFERENCES public.public_contract_families(id) ON DELETE CASCADE,
    public_contract_id              uuid        NOT NULL REFERENCES public.public_contracts(id) ON DELETE CASCADE,
    derived_from_public_contract_id uuid        REFERENCES public.public_contracts(id) ON DELETE CASCADE,
    boundary                        text        NOT NULL
        CHECK (boundary IN ('ingest', 'stage', 'egress')),
    position                        integer     NOT NULL CHECK (position >= 0),
    created_at                      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT public_contract_family_links_family_contract_unique UNIQUE (family_id, public_contract_id),
    CONSTRAINT public_contract_family_links_family_position_unique UNIQUE (family_id, position),
    CONSTRAINT public_contract_family_links_no_self_parent
        CHECK (derived_from_public_contract_id IS NULL OR derived_from_public_contract_id <> public_contract_id)
);

COMMENT ON TABLE public.public_contract_family_links IS
    'Parent link for a public catalog contract inside a family. boundary is ingest, stage (between stages), or egress. One public contract belongs to at most one family.';

CREATE UNIQUE INDEX IF NOT EXISTS public_contract_family_links_contract_unique
    ON public.public_contract_family_links (public_contract_id);

CREATE INDEX IF NOT EXISTS public_contract_family_links_family_position_idx
    ON public.public_contract_family_links (family_id, position);

-- ── 3. updated_at ────────────────────────────────────────────────────────────

DROP TRIGGER IF EXISTS contract_families_updated_at ON public.contract_families;
CREATE TRIGGER contract_families_updated_at
    BEFORE UPDATE ON public.contract_families
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

DROP TRIGGER IF EXISTS public_contract_families_updated_at ON public.public_contract_families;
CREATE TRIGGER public_contract_families_updated_at
    BEFORE UPDATE ON public.public_contract_families
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- ── 4. RLS: deny anon/authenticated; gateway role bypasses RLS ───────────────

ALTER TABLE public.contract_families ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.contract_family_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.public_contract_families ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.public_contract_family_links ENABLE ROW LEVEL SECURITY;

-- ── 5. Seed the Stripe charges pipeline as a public catalog family ───────────

-- ── stripe-charges-raw (ingest) ─────────────────────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'stripe_charges_raw',
    'stripe-charges-raw',
    'Raw Stripe Charge object at ingest. Compatible with Stripe''s API Charge object; not affiliated with Stripe. Root of the Stripe charges contract family.',
    'Stripe',
    'Payments ingest',
    NULL,
    'json',
    $yaml$# Ingest-gate contract — mirrors catalog/seeds/stripe-charges (Stripe Charge API export).
# Pipeline stage: Sources / vendor APIs → POST /v1/ingest/{contract_id} → Raw
# Chain: stripe_charges_raw → stripe_charges_staged → revenue_daily_mart → partner_payout_feed
# Not affiliated with Stripe. Sample ids are obviously fake.
version: "1.0"
name: "stripe_charges_raw"
description: >
  Raw Stripe Charge object at ingest (core fields of an exported Stripe charge /
  webhook-style payload). Compatible with Stripe's API Charge object; not affiliated
  with Stripe. derived_from: catalog/seeds/stripe-charges (public catalog seed).
  chain: stripe_charges_raw (ingest) → stripe_charges_staged (stage) →
  revenue_daily_mart (mart / alongside dbt) → partner_payout_feed (egress).

ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^(ch|py)_[A-Za-z0-9]+$"
    - name: object
      type: string
      required: true
      enum: ["charge"]
    - name: amount
      type: integer
      required: true
      min: 0
    - name: amount_captured
      type: integer
      required: false
      min: 0
    - name: amount_refunded
      type: integer
      required: false
      min: 0
    - name: currency
      type: string
      required: true
      pattern: "^[a-z]{3}$"
    - name: customer
      type: string
      required: false
      pattern: "^cus_[A-Za-z0-9]+$"
    - name: description
      type: string
      required: false
    - name: paid
      type: boolean
      required: true
    - name: refunded
      type: boolean
      required: true
    - name: captured
      type: boolean
      required: false
    - name: status
      type: string
      required: true
      enum: ["succeeded", "pending", "failed"]
    - name: created
      type: integer
      required: true
      min: 0
    - name: livemode
      type: boolean
      required: false
    - name: receipt_email
      type: string
      required: false
      format: email
    - name: payment_intent
      type: string
      required: false
      pattern: "^pi_[A-Za-z0-9]+$"

glossary:
  - field: id
    description: "Charge id. Card charges use ch_; some bank-debit / Connect objects use py_."
    constraints: "ch_ or py_, then alphanumeric"
  - field: amount
    description: "Amount intended to be collected, in the smallest currency unit (e.g. cents)."
    constraints: "non-negative integer"
  - field: currency
    description: "Three-letter ISO currency code, lowercase."
    constraints: "ISO 4217, lowercase"
  - field: status
    description: "The status of the payment."
    constraints: "one of succeeded, pending, failed"
  - field: created
    description: "Unix epoch seconds when the object was created."
    constraints: "non-negative integer"
$yaml$,
    '1.0',
    'https://docs.stripe.com/api/charges/object',
    '2026-11-09'::date,
    $json$[
{
  "id": "ch_3Qdemo0001AaBbCcDd",
  "object": "charge",
  "amount": 4299,
  "amount_captured": 4299,
  "amount_refunded": 0,
  "currency": "usd",
  "customer": "cus_DemoAcme0001",
  "description": "Demo invoice INV-DEMO-1001",
  "paid": true,
  "refunded": false,
  "captured": true,
  "status": "succeeded",
  "created": 1735689600,
  "livemode": false,
  "receipt_email": "billing-demo@example.com",
  "payment_intent": "pi_3Qdemo0001AaBbCc"
}
]$json$::jsonb,
    'MIT',
    NULL
)
ON CONFLICT (name) DO UPDATE SET
    slug                = EXCLUDED.slug,
    description         = EXCLUDED.description,
    vendor              = EXCLUDED.vendor,
    category            = EXCLUDED.category,
    source_url          = EXCLUDED.source_url,
    source_format       = EXCLUDED.source_format,
    contract_yaml       = EXCLUDED.contract_yaml,
    version             = EXCLUDED.version,
    verified_source_url = EXCLUDED.verified_source_url,
    verified_at         = EXCLUDED.verified_at,
    sample_json         = EXCLUDED.sample_json,
    license_note        = EXCLUDED.license_note,
    updated_at          = now();

-- ── stripe-charges-staged (stage) ─────────────────────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'stripe_charges_staged',
    'stripe-charges-staged',
    'Cleaned Stripe charge row after the raw ingest transform. Between-stages gate. Derived from stripe_charges_raw. Not affiliated with Stripe.',
    'Stripe',
    'Payments staging',
    NULL,
    'json',
    $yaml$# Stage-gate contract — cleaned / typed row after raw landing.
# Pipeline stage: Raw → POST /v1/ingest/{contract_id} → Staging
# Chain: derived_from stripe_charges_raw
version: "1.0"
name: "stripe_charges_staged"
description: >
  Cleaned Stripe charge row after transform out of raw: amount in decimal currency
  units (not cents), uppercase ISO currency, flattened customer_id, status enum,
  UTC ISO-8601 charged_at. derived_from: stripe_charges_raw (ingest gate).
  chain: stripe_charges_raw → stripe_charges_staged → revenue_daily_mart →
  partner_payout_feed. Not affiliated with Stripe.

ontology:
  entities:
    - name: charge_id
      type: string
      required: true
      pattern: "^(ch|py)_[A-Za-z0-9]+$"
    - name: customer_id
      type: string
      required: true
      pattern: "^cus_[A-Za-z0-9]+$"
    - name: amount
      type: number
      required: true
      min: 0
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: status
      type: string
      required: true
      enum: ["succeeded", "pending", "failed"]
    - name: paid
      type: boolean
      required: true
    - name: refunded
      type: boolean
      required: true
    - name: charged_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
    - name: description
      type: string
      required: false
    - name: payment_intent_id
      type: string
      required: false
      pattern: "^pi_[A-Za-z0-9]+$"

glossary:
  - field: amount
    description: "Charge amount in major currency units (e.g. 42.99 USD), not cents."
    constraints: "non-negative number"
  - field: currency
    description: "Normalized three-letter ISO 4217 currency code, uppercase."
    constraints: "ISO 4217, uppercase"
  - field: charged_at
    description: "Charge creation time as UTC ISO-8601 with trailing Z."
    constraints: "YYYY-MM-DDTHH:MM:SSZ"
  - field: customer_id
    description: "Flattened Stripe customer id from the raw customer field."
    constraints: "starts with cus_"
$yaml$,
    '1.0',
    NULL,
    NULL,
    $json$[
{
  "charge_id": "ch_3Qdemo0001AaBbCcDd",
  "customer_id": "cus_DemoAcme0001",
  "amount": 42.99,
  "currency": "USD",
  "status": "succeeded",
  "paid": true,
  "refunded": false,
  "charged_at": "2025-01-01T00:00:00Z",
  "description": "Demo invoice INV-DEMO-1001",
  "payment_intent_id": "pi_3Qdemo0001AaBbCc"
}
]$json$::jsonb,
    'MIT',
    NULL
)
ON CONFLICT (name) DO UPDATE SET
    slug                = EXCLUDED.slug,
    description         = EXCLUDED.description,
    vendor              = EXCLUDED.vendor,
    category            = EXCLUDED.category,
    source_url          = EXCLUDED.source_url,
    source_format       = EXCLUDED.source_format,
    contract_yaml       = EXCLUDED.contract_yaml,
    version             = EXCLUDED.version,
    verified_source_url = EXCLUDED.verified_source_url,
    verified_at         = EXCLUDED.verified_at,
    sample_json         = EXCLUDED.sample_json,
    license_note        = EXCLUDED.license_note,
    updated_at          = now();

-- ── revenue-daily-mart (stage) ─────────────────────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'revenue_daily_mart',
    'revenue-daily-mart',
    'Daily revenue mart grain built from staged charges. Between-stages gate, kept alongside dbt, Great Expectations, or Soda. Derived from stripe_charges_staged. Not affiliated with Stripe.',
    'Stripe',
    'Payments mart',
    NULL,
    'json',
    $yaml$# Mart / stage-gate contract — daily revenue aggregates (use alongside dbt/GE/Soda).
# Pipeline stage: Staging → POST /v1/ingest/{contract_id} (or egress if publishing) → Marts
# Complementary to post-modeling tests; does not replace them.
# Chain: derived_from stripe_charges_staged
version: "1.0"
name: "revenue_daily_mart"
description: >
  Daily revenue mart grain: one row per revenue_date × currency × status aggregate
  built from staged charges. Enforce at a stage gate with POST /v1/ingest (or
  POST /egress if this mart is published outbound). Keep dbt / Great Expectations /
  Soda tests on the modeled table — complementary, not either/or.
  derived_from: stripe_charges_staged (stage gate).
  chain: stripe_charges_raw → stripe_charges_staged → revenue_daily_mart →
  partner_payout_feed. Synthetic demo grain; not affiliated with Stripe.

ontology:
  entities:
    - name: revenue_date
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: status
      type: string
      required: true
      enum: ["succeeded", "pending", "failed"]
    - name: charge_count
      type: integer
      required: true
      min: 0
    - name: gross_amount
      type: number
      required: true
      min: 0
    - name: refunded_amount
      type: number
      required: true
      min: 0
    - name: net_amount
      type: number
      required: true
    - name: as_of
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"

glossary:
  - field: revenue_date
    description: "UTC calendar date of the aggregate (from charged_at)."
    constraints: "YYYY-MM-DD"
  - field: gross_amount
    description: "Sum of staged charge amounts for the grain, major currency units."
    constraints: "non-negative number"
  - field: net_amount
    description: "gross_amount minus refunded_amount for the grain."
    constraints: "number (may be zero)"
  - field: as_of
    description: "When this mart row was computed, UTC ISO-8601."
    constraints: "YYYY-MM-DDTHH:MM:SSZ"
$yaml$,
    '1.0',
    NULL,
    NULL,
    $json$[
{
  "revenue_date": "2025-01-01",
  "currency": "USD",
  "status": "succeeded",
  "charge_count": 3,
  "gross_amount": 128.97,
  "refunded_amount": 0.0,
  "net_amount": 128.97,
  "as_of": "2025-01-02T06:00:00Z"
}
]$json$::jsonb,
    'MIT',
    NULL
)
ON CONFLICT (name) DO UPDATE SET
    slug                = EXCLUDED.slug,
    description         = EXCLUDED.description,
    vendor              = EXCLUDED.vendor,
    category            = EXCLUDED.category,
    source_url          = EXCLUDED.source_url,
    source_format       = EXCLUDED.source_format,
    contract_yaml       = EXCLUDED.contract_yaml,
    version             = EXCLUDED.version,
    verified_source_url = EXCLUDED.verified_source_url,
    verified_at         = EXCLUDED.verified_at,
    sample_json         = EXCLUDED.sample_json,
    license_note        = EXCLUDED.license_note,
    updated_at          = now();

-- ── partner-payout-feed (egress) ─────────────────────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'partner_payout_feed',
    'partner-payout-feed',
    'Outbound partner payout feed. Egress gate before reverse ETL or a partner API. Derived from revenue_daily_mart. Not affiliated with Stripe.',
    'Stripe',
    'Payments egress',
    NULL,
    'json',
    $yaml$# Egress-gate contract — outbound partner payout feed.
# Pipeline stage: Marts → POST /egress/{contract_id} → Reverse ETL / partners / APIs
# Chain: derived_from revenue_daily_mart
version: "1.0"
name: "partner_payout_feed"
description: >
  Outbound partner payout feed row. Validate with POST /egress/{contract_id} before
  reverse ETL or partner API delivery; forward only response.payload (default
  disposition=block omits failing records). derived_from: revenue_daily_mart
  (mart / stage gate alongside dbt). chain: stripe_charges_raw →
  stripe_charges_staged → revenue_daily_mart → partner_payout_feed.
  Demo partner ids are fake; not affiliated with Stripe.

ontology:
  entities:
    - name: payout_id
      type: string
      required: true
      pattern: "^po_demo_[A-Za-z0-9_]+$"
    - name: partner_id
      type: string
      required: true
      pattern: "^ptr_demo_[A-Za-z0-9_]+$"
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: amount
      type: number
      required: true
      min: 0.01
    - name: revenue_date
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
    - name: status
      type: string
      required: true
      enum: ["ready", "held", "sent"]
    - name: generated_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"

glossary:
  - field: payout_id
    description: "Stable id for this outbound payout line (demo prefix po_demo_)."
    constraints: "po_demo_ + alphanumeric or underscore"
  - field: amount
    description: "Payout amount in major currency units derived from mart net_amount."
    constraints: ">= 0.01"
  - field: status
    description: "Outbound lifecycle before/after partner delivery."
    constraints: "ready | held | sent"
$yaml$,
    '1.0',
    NULL,
    NULL,
    $json$[
{
  "payout_id": "po_demo_20250101_USD_001",
  "partner_id": "ptr_demo_acme_revshare",
  "currency": "USD",
  "amount": 128.97,
  "revenue_date": "2025-01-01",
  "status": "ready",
  "generated_at": "2025-01-02T07:00:00Z"
}
]$json$::jsonb,
    'MIT',
    NULL
)
ON CONFLICT (name) DO UPDATE SET
    slug                = EXCLUDED.slug,
    description         = EXCLUDED.description,
    vendor              = EXCLUDED.vendor,
    category            = EXCLUDED.category,
    source_url          = EXCLUDED.source_url,
    source_format       = EXCLUDED.source_format,
    contract_yaml       = EXCLUDED.contract_yaml,
    version             = EXCLUDED.version,
    verified_source_url = EXCLUDED.verified_source_url,
    verified_at         = EXCLUDED.verified_at,
    sample_json         = EXCLUDED.sample_json,
    license_note        = EXCLUDED.license_note,
    updated_at          = now();

INSERT INTO public.public_contract_families (name, slug, description)
VALUES (
    'Stripe charges pipeline',
    'stripe-charges-pipeline',
    'Stripe charges as they move from raw ingest, through a cleaned stage and a daily revenue mart, to a partner payout feed. Not affiliated with Stripe. Sample ids are fake.'
)
ON CONFLICT (slug) DO UPDATE SET
    name        = EXCLUDED.name,
    description = EXCLUDED.description,
    updated_at  = now();

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, NULL, 'ingest', 0
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'stripe_charges_raw'

WHERE f.slug = 'stripe-charges-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'stage', 1
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'stripe_charges_staged'
JOIN public_contracts parent ON parent.name = 'stripe_charges_raw'
WHERE f.slug = 'stripe-charges-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'stage', 2
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'revenue_daily_mart'
JOIN public_contracts parent ON parent.name = 'stripe_charges_staged'
WHERE f.slug = 'stripe-charges-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'egress', 3
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'partner_payout_feed'
JOIN public_contracts parent ON parent.name = 'revenue_daily_mart'
WHERE f.slug = 'stripe-charges-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

-- The chain is a single path: each position's parent is the previous contract.
