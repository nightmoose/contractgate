-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 052: Shopify orders and GA4 events public catalog families
--
-- 050 created contract families and seeded stripe-charges-pipeline.
-- 051 is quarantine reviewed_at. This file does not edit 050.
--
-- Two more catalog chains, each ingest → stage → egress, copied from
-- examples/contract-chain/shopify/ and examples/contract-chain/ga4/.
-- The pass sample is sample_json. Not affiliated with Shopify or Google.
-- Sample ids are fake. These rows are in addition to shopify-orders and
-- ga4-bigquery-events.
--
-- Idempotent: catalog rows upsert on name; families upsert on slug; links
-- upsert on (family_id, public_contract_id).
-- ─────────────────────────────────────────────────────────────────────────────


-- ── shopify-orders-pipeline ─────────────────────────────────────────

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'shopify_orders_raw',
    'shopify-orders-raw',
    'Raw Shopify orders CSV row at ingest. Compatible with Shopify''s documented orders export; not affiliated with Shopify. Root of the Shopify orders contract family.',
    'Shopify',
    'Commerce',
    NULL,
    'json',
    $yaml$# Ingest-gate contract — mirrors catalog/seeds/shopify-orders (orders CSV export).
# Pipeline stage: Sources → POST /v1/ingest/{contract_id} → Raw
# Chain: shopify_orders_raw → shopify_orders_staged → shopify_fulfillment_notice
# Not affiliated with Shopify. Sample order names are fake.
version: "1.0"
name: "shopify_orders_raw"
description: >
  Raw Shopify orders export row at ingest (core columns of the Store admin
  Orders CSV). Compatible with Shopify's documented orders export; not
  affiliated with Shopify. derived_from: catalog/seeds/shopify-orders.
  chain: shopify_orders_raw (ingest) → shopify_orders_staged (stage) →
  shopify_fulfillment_notice (egress).

ontology:
  entities:
    - name: Name
      type: string
      required: true
      pattern: "^#?[A-Za-z_-]{0,20}[0-9]{1,12}[A-Za-z0-9_-]{0,20}$"
    - name: Email
      type: string
      required: true
      format: email
    - name: Financial Status
      type: string
      required: true
      enum:
        - "pending"
        - "authorized"
        - "paid"
        - "partially_paid"
        - "refunded"
        - "voided"
        - "partially_refunded"
        - "expired"
    - name: Currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: Total
      type: string
      required: true
      pattern: "^[0-9]+(\\.[0-9]{1,2})?$"
    - name: Created at
      type: string
      required: false

glossary:
  - field: Name
    description: "Order name as shown in the Shopify admin, for example #1001."
    constraints: "optional leading #, a letter prefix, and at least one digit"
  - field: Total
    description: "Order total in the order currency, as a decimal string from the CSV."
    constraints: "non-negative decimal string"
  - field: Financial Status
    description: "Admin payment status."
    constraints: "pending, authorized, paid, partially_paid, partially_refunded, refunded, voided, or expired"
$yaml$,
    '1.0',
    'https://help.shopify.com/en/manual/orders/export-orders',
    '2026-11-14',
    $json$[
  {
    "Name": "#1001",
    "Email": "shopper@example.com",
    "Financial Status": "paid",
    "Currency": "USD",
    "Total": "50.36",
    "Created at": "2026-01-15 09:30:00 -0500"
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

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'shopify_orders_staged',
    'shopify-orders-staged',
    'Typed Shopify order between stages: money as a number and ordered_at in UTC. Not affiliated with Shopify.',
    'Shopify',
    'Commerce',
    NULL,
    'json',
    $yaml$# Stage-gate contract — typed order after the CSV landing.
# Pipeline stage: Raw → POST /v1/ingest/{contract_id} → Staging
# Chain: derived_from shopify_orders_raw
version: "1.0"
name: "shopify_orders_staged"
description: >
  Cleaned Shopify order after the CSV landing: money as a number, uppercase
  currency, and ordered_at in UTC. derived_from: shopify_orders_raw (ingest).
  chain: shopify_orders_raw → shopify_orders_staged →
  shopify_fulfillment_notice. Not affiliated with Shopify.

ontology:
  entities:
    - name: order_name
      type: string
      required: true
      pattern: "^#?[A-Za-z_-]{0,20}[0-9]{1,12}[A-Za-z0-9_-]{0,20}$"
    - name: email
      type: string
      required: true
      format: email
    - name: financial_status
      type: string
      required: true
      enum:
        - "pending"
        - "authorized"
        - "paid"
        - "partially_paid"
        - "refunded"
        - "voided"
        - "partially_refunded"
        - "expired"
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: total
      type: number
      required: true
      min: 0
    - name: ordered_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"

glossary:
  - field: total
    description: "Order total in major currency units, parsed from the CSV decimal string."
    constraints: ">= 0"
  - field: ordered_at
    description: "Order created time in UTC."
    constraints: "YYYY-MM-DDTHH:MM:SSZ"
$yaml$,
    '1.0',
    'https://help.shopify.com/en/manual/orders/export-orders',
    '2026-11-14',
    $json$[
  {
    "order_name": "#1001",
    "email": "shopper@example.com",
    "financial_status": "paid",
    "currency": "USD",
    "total": 50.36,
    "ordered_at": "2026-01-15T14:30:00Z"
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

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'shopify_fulfillment_notice',
    'shopify-fulfillment-notice',
    'Outbound Shopify fulfillment notice at egress, for a warehouse or 3PL. Not affiliated with Shopify. Sample ids are fake.',
    'Shopify',
    'Commerce',
    NULL,
    'json',
    $yaml$# Egress-gate contract — outbound fulfillment notice for a warehouse or 3PL.
# Pipeline stage: Staging → POST /egress/{contract_id} → Partner / warehouse
# Chain: derived_from shopify_orders_staged
version: "1.0"
name: "shopify_fulfillment_notice"
description: >
  Outbound fulfillment notice. Validate with POST /egress/{contract_id}
  before sending it to a warehouse or 3PL; forward only the conforming
  payload. derived_from: shopify_orders_staged (stage).
  chain: shopify_orders_raw → shopify_orders_staged →
  shopify_fulfillment_notice. Not affiliated with Shopify. Sample ids are fake.

ontology:
  entities:
    - name: notice_id
      type: string
      required: true
      pattern: "^ntc_demo_[A-Za-z0-9_]+$"
    - name: order_name
      type: string
      required: true
      pattern: "^#?[A-Za-z_-]{0,20}[0-9]{1,12}[A-Za-z0-9_-]{0,20}$"
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: total
      type: number
      required: true
      min: 0.01
    - name: status
      type: string
      required: true
      enum: ["ready", "held", "sent"]
    - name: generated_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"

glossary:
  - field: notice_id
    description: "Stable id for this outbound notice (demo prefix ntc_demo_)."
    constraints: "ntc_demo_ plus alphanumeric or underscore"
  - field: status
    description: "Whether the notice is ready to send, held, or already sent."
    constraints: "ready | held | sent"
$yaml$,
    '1.0',
    'https://help.shopify.com/en/manual/orders/export-orders',
    '2026-11-14',
    $json$[
  {
    "notice_id": "ntc_demo_1001_001",
    "order_name": "#1001",
    "currency": "USD",
    "total": 50.36,
    "status": "ready",
    "generated_at": "2026-01-15T15:00:00Z"
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
    'Shopify orders pipeline',
    'shopify-orders-pipeline',
    'Shopify orders as they move from the admin CSV export, through a typed order stage, to a warehouse fulfillment notice. Not affiliated with Shopify. Sample order names are fake.'
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
JOIN public_contracts child ON child.name = 'shopify_orders_raw'
WHERE f.slug = 'shopify-orders-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'stage', 1
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'shopify_orders_staged'
JOIN public_contracts parent ON parent.name = 'shopify_orders_raw'
WHERE f.slug = 'shopify-orders-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'egress', 2
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'shopify_fulfillment_notice'
JOIN public_contracts parent ON parent.name = 'shopify_orders_staged'
WHERE f.slug = 'shopify-orders-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;


-- ── ga4-events-pipeline ─────────────────────────────────────────

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'ga4_events_raw',
    'ga4-events-raw',
    'Raw GA4 BigQuery events row at ingest. Compatible with Google Analytics 4''s documented BigQuery export; not affiliated with Google. Root of the GA4 events contract family.',
    'Google Analytics 4',
    'Analytics',
    NULL,
    'json',
    $yaml$# Ingest-gate contract — mirrors catalog/seeds/ga4-bigquery-events (daily events_ export).
# Pipeline stage: Sources → POST /v1/ingest/{contract_id} → Raw
# Chain: ga4_events_raw → ga4_events_staged → ga4_audience_export
# Not affiliated with Google. Sample ids are fake.
version: "1.0"
name: "ga4_events_raw"
description: >
  Raw GA4 BigQuery Export event at ingest (core scalar columns of the daily
  events_ table). Compatible with Google Analytics 4's documented BigQuery
  export; not affiliated with Google. Nested RECORD columns are omitted.
  derived_from: catalog/seeds/ga4-bigquery-events.
  chain: ga4_events_raw (ingest) → ga4_events_staged (stage) →
  ga4_audience_export (egress).

ontology:
  entities:
    - name: event_date
      type: string
      required: true
      pattern: "^[0-9]{8}$"
    - name: event_timestamp
      type: integer
      required: true
      min: 0
    - name: event_name
      type: string
      required: true
    - name: user_pseudo_id
      type: string
      required: false
    - name: platform
      type: string
      required: true
      enum: ["WEB", "ANDROID", "IOS"]
    - name: stream_id
      type: string
      required: false

glossary:
  - field: event_date
    description: "Date the event was logged, YYYYMMDD in the property's reporting time zone."
    constraints: "8-digit date string"
  - field: event_timestamp
    description: "Client time in microseconds UTC."
    constraints: "non-negative integer"
  - field: platform
    description: "Data stream platform from the BigQuery export."
    constraints: "WEB, ANDROID, or IOS"
$yaml$,
    '1.0',
    'https://support.google.com/analytics/answer/7029846',
    '2026-11-14',
    $json$[
  {
    "event_date": "20260115",
    "event_timestamp": 1768491000000000,
    "event_name": "page_view",
    "user_pseudo_id": "1234567890.0987654321",
    "platform": "WEB",
    "stream_id": "1234567"
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

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'ga4_events_staged',
    'ga4-events-staged',
    'Cleaned GA4 event between stages: ISO event_date, lowercase platform, UTC event_at. Not affiliated with Google.',
    'Google Analytics 4',
    'Analytics',
    NULL,
    'json',
    $yaml$# Stage-gate contract — cleaned GA4 event after the raw export landing.
# Pipeline stage: Raw → POST /v1/ingest/{contract_id} → Staging
# Chain: derived_from ga4_events_raw
version: "1.0"
name: "ga4_events_staged"
description: >
  Cleaned GA4 event: event_date as an ISO date, platform in lowercase, and
  event_at as UTC RFC 3339 parsed from the microsecond timestamp.
  derived_from: ga4_events_raw (ingest).
  chain: ga4_events_raw → ga4_events_staged → ga4_audience_export.
  Not affiliated with Google.

ontology:
  entities:
    - name: event_date
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
    - name: event_name
      type: string
      required: true
    - name: platform
      type: string
      required: true
      enum: ["web", "android", "ios"]
    - name: event_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
    - name: user_pseudo_id
      type: string
      required: false

glossary:
  - field: event_date
    description: "Reporting date as an ISO calendar date."
    constraints: "YYYY-MM-DD"
  - field: platform
    description: "Stream platform, lowercased from the export's WEB / ANDROID / IOS."
    constraints: "web, android, or ios"
  - field: event_at
    description: "Event time in UTC, converted from the microsecond timestamp."
    constraints: "YYYY-MM-DDTHH:MM:SSZ"
$yaml$,
    '1.0',
    'https://support.google.com/analytics/answer/7029846',
    '2026-11-14',
    $json$[
  {
    "event_date": "2026-01-15",
    "event_name": "page_view",
    "platform": "web",
    "event_at": "2026-01-15T16:50:00Z",
    "user_pseudo_id": "1234567890.0987654321"
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

INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'ga4_audience_export',
    'ga4-audience-export',
    'Outbound GA4 audience row at egress, for an ads partner or reverse ETL. Not affiliated with Google. Sample ids are fake.',
    'Google Analytics 4',
    'Analytics',
    NULL,
    'json',
    $yaml$# Egress-gate contract — audience row sent to an ads or reverse-ETL destination.
# Pipeline stage: Staging → POST /egress/{contract_id} → Partner / ads API
# Chain: derived_from ga4_events_staged
version: "1.0"
name: "ga4_audience_export"
description: >
  Outbound audience row built from a cleaned GA4 event. Validate with
  POST /egress/{contract_id} before sending it to an ads partner or reverse
  ETL; forward only the conforming payload. derived_from: ga4_events_staged
  (stage). chain: ga4_events_raw → ga4_events_staged → ga4_audience_export.
  Not affiliated with Google. Sample ids are fake.

ontology:
  entities:
    - name: export_id
      type: string
      required: true
      pattern: "^aud_demo_[A-Za-z0-9_]+$"
    - name: event_date
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
    - name: event_name
      type: string
      required: true
    - name: platform
      type: string
      required: true
      enum: ["web", "android", "ios"]
    - name: status
      type: string
      required: true
      enum: ["ready", "held", "sent"]
    - name: generated_at
      type: string
      required: true
      pattern: "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"

glossary:
  - field: export_id
    description: "Stable id for this outbound audience row (demo prefix aud_demo_)."
    constraints: "aud_demo_ plus alphanumeric or underscore"
  - field: status
    description: "Whether the row is ready to send, held, or already sent."
    constraints: "ready | held | sent"
$yaml$,
    '1.0',
    'https://support.google.com/analytics/answer/7029846',
    '2026-11-14',
    $json$[
  {
    "export_id": "aud_demo_page_view_20260115",
    "event_date": "2026-01-15",
    "event_name": "page_view",
    "platform": "web",
    "status": "ready",
    "generated_at": "2026-01-16T07:00:00Z"
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
    'GA4 events pipeline',
    'ga4-events-pipeline',
    'GA4 BigQuery events as they move from the raw daily export, through a cleaned event stage, to an audience export. Not affiliated with Google. Sample ids are fake.'
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
JOIN public_contracts child ON child.name = 'ga4_events_raw'
WHERE f.slug = 'ga4-events-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'stage', 1
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'ga4_events_staged'
JOIN public_contracts parent ON parent.name = 'ga4_events_raw'
WHERE f.slug = 'ga4-events-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;

INSERT INTO public.public_contract_family_links (
    family_id, public_contract_id, derived_from_public_contract_id, boundary, position
)
SELECT f.id, child.id, parent.id, 'egress', 2
FROM public.public_contract_families f
JOIN public_contracts child ON child.name = 'ga4_audience_export'
JOIN public_contracts parent ON parent.name = 'ga4_events_staged'
WHERE f.slug = 'ga4-events-pipeline'
ON CONFLICT (family_id, public_contract_id) DO UPDATE SET
    derived_from_public_contract_id = EXCLUDED.derived_from_public_contract_id,
    boundary = EXCLUDED.boundary,
    position = EXCLUDED.position;
