-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 047: Public Catalog phase 1 expansion
--
-- GENERATED FILE — do not hand-edit. Historical snapshot of the original 12
-- rows. Later seed corrections are upserted by 048_public_catalog_seed_refresh.sql
-- (node catalog/scripts/generate-migration.mjs). Do not regenerate this file.
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

-- ── stripe-charges (Stripe · Payments) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'stripe-charges',
    'stripe-charges',
    'Stripe Charge object — core fields of an exported Stripe charge. Compatible with Stripe''s API Charge object; not affiliated with Stripe.',
    'Stripe',
    'Payments',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "stripe-charges"
description: "Stripe Charge object — core fields of an exported Stripe charge. Compatible with Stripe's API Charge object; not affiliated with Stripe."

ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^ch_[A-Za-z0-9]+$"
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
    description: "Time at which the object was created (Unix epoch seconds)."
    constraints: "non-negative integer"
$yaml$,
    '1.0',
    'https://docs.stripe.com/api/charges/object',
    '2026-11-05'::date,
    $json$[
  {
    "id": "ch_3Oexample0001AbCdEfGh",
    "object": "charge",
    "amount": 4200,
    "amount_captured": 4200,
    "amount_refunded": 0,
    "currency": "usd",
    "customer": "cus_Example0001",
    "description": "Invoice INV-1001",
    "paid": true,
    "refunded": false,
    "captured": true,
    "status": "succeeded",
    "created": 1731000000,
    "livemode": false,
    "receipt_email": "customer@example.com",
    "payment_intent": "pi_3Oexample0001"
  },
  {
    "id": "ch_3Oexample0002IjKlMnOp",
    "object": "charge",
    "amount": 1599,
    "amount_captured": 0,
    "amount_refunded": 0,
    "currency": "eur",
    "description": "Pending subscription charge",
    "paid": false,
    "refunded": false,
    "captured": false,
    "status": "pending",
    "created": 1731003600
  },
  {
    "id": "ch_3Oexample0003QrStUvWx",
    "object": "charge",
    "amount": 9900,
    "amount_captured": 9900,
    "amount_refunded": 9900,
    "currency": "gbp",
    "customer": "cus_Example0003",
    "description": "Refunded order 5582",
    "paid": true,
    "refunded": true,
    "captured": true,
    "status": "succeeded",
    "created": 1731090000,
    "receipt_email": "refunds@example.com"
  }
]$json$::jsonb,
    'MIT',
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

-- ── stripe-customers (Stripe · Payments) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'stripe-customers',
    'stripe-customers',
    'Stripe Customer object — core fields of an exported Stripe customer. Compatible with Stripe''s API Customer object; not affiliated with Stripe.',
    'Stripe',
    'Payments',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "stripe-customers"
description: "Stripe Customer object — core fields of an exported Stripe customer. Compatible with Stripe's API Customer object; not affiliated with Stripe."

ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^cus_[A-Za-z0-9]+$"
    - name: object
      type: string
      required: true
      enum: ["customer"]
    - name: email
      type: string
      required: false
      format: email
    - name: name
      type: string
      required: false
    - name: description
      type: string
      required: false
    - name: phone
      type: string
      required: false
    - name: created
      type: integer
      required: true
      min: 0
    - name: currency
      type: string
      required: false
      pattern: "^[a-z]{3}$"
    - name: balance
      type: integer
      required: false
    - name: delinquent
      type: boolean
      required: false
    - name: livemode
      type: boolean
      required: false

glossary:
  - field: id
    description: "Unique identifier for the customer."
    constraints: "starts with cus_"
  - field: balance
    description: "Current balance, if any, in the smallest currency unit. A negative balance is credit."
    constraints: "integer, may be negative"
  - field: created
    description: "Time at which the object was created (Unix epoch seconds)."
    constraints: "non-negative integer"
$yaml$,
    '1.0',
    'https://docs.stripe.com/api/customers/object',
    '2026-11-05'::date,
    $json$[
  {
    "id": "cus_Example0001",
    "object": "customer",
    "email": "customer@example.com",
    "name": "Jordan Example",
    "description": "Imported from client CRM",
    "phone": "555-0100",
    "created": 1731000000,
    "currency": "usd",
    "balance": 0,
    "delinquent": false,
    "livemode": false
  },
  {
    "id": "cus_Example0002",
    "object": "customer",
    "email": "billing@example.org",
    "name": "Casey Sample",
    "created": 1731003600,
    "balance": -500,
    "delinquent": false
  },
  {
    "id": "cus_Example0003",
    "object": "customer",
    "name": "Acme Test LLC",
    "phone": "555-0133",
    "created": 1731090000,
    "currency": "eur",
    "delinquent": true,
    "livemode": false
  }
]$json$::jsonb,
    'MIT',
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

-- ── shopify-orders (Shopify · Commerce) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'shopify-orders',
    'shopify-orders',
    'Shopify orders CSV export — core columns from the Store admin Orders export. Compatible with Shopify''s documented orders CSV; not affiliated with Shopify. Values are CSV strings; money columns are validated as decimal strings.',
    'Shopify',
    'Commerce',
    NULL,
    'csv',
    $yaml$
version: "1.0"
name: "shopify-orders"
description: "Shopify orders CSV export — core columns from the Store admin Orders export. Compatible with Shopify's documented orders CSV; not affiliated with Shopify. Values are CSV strings; money columns are validated as decimal strings."

ontology:
  entities:
    - name: Name
      type: string
      required: true
      pattern: "^#?[0-9]+$"
    - name: Email
      type: string
      required: false
      format: email
    - name: Financial Status
      type: string
      required: false
      enum:
        - "pending"
        - "authorized"
        - "paid"
        - "partially_paid"
        - "refunded"
        - "voided"
        - "partially_refunded"
        - "unpaid"
    - name: Fulfillment Status
      type: string
      required: false
      enum: ["fulfilled", "unfulfilled", "partial", "restocked"]
    - name: Currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: Subtotal
      type: string
      required: false
      pattern: "^-?[0-9]+(\\.[0-9]{1,2})?$"
    - name: Shipping
      type: string
      required: false
      pattern: "^-?[0-9]+(\\.[0-9]{1,2})?$"
    - name: Taxes
      type: string
      required: false
      pattern: "^-?[0-9]+(\\.[0-9]{1,2})?$"
    - name: Total
      type: string
      required: true
      pattern: "^-?[0-9]+(\\.[0-9]{1,2})?$"
    - name: Created at
      type: string
      required: false

glossary:
  - field: Name
    description: "Order name as shown in the admin (e.g. #1001)."
    constraints: "order number, optional leading #"
  - field: Financial Status
    description: "Payment status of the order."
    constraints: "Shopify financial status value"
  - field: Total
    description: "Order total in the order currency."
    constraints: "decimal string"
$yaml$,
    '1.0',
    'https://help.shopify.com/en/manual/orders/export-orders',
    '2026-11-05'::date,
    $json$[
  {
    "Name": "#1001",
    "Email": "shopper@example.com",
    "Financial Status": "paid",
    "Fulfillment Status": "fulfilled",
    "Currency": "USD",
    "Subtotal": "42.00",
    "Shipping": "5.00",
    "Taxes": "3.36",
    "Total": "50.36",
    "Created at": "2026-01-15 09:30:00 -0500"
  },
  {
    "Name": "#1002",
    "Email": "buyer@example.org",
    "Financial Status": "pending",
    "Fulfillment Status": "unfulfilled",
    "Currency": "USD",
    "Subtotal": "19.99",
    "Shipping": "0.00",
    "Taxes": "1.60",
    "Total": "21.59",
    "Created at": "2026-01-16 14:05:00 -0500"
  },
  {
    "Name": "#1003",
    "Email": "returns@example.com",
    "Financial Status": "refunded",
    "Fulfillment Status": "restocked",
    "Currency": "CAD",
    "Subtotal": "120.00",
    "Shipping": "12.50",
    "Taxes": "17.16",
    "Total": "149.66",
    "Created at": "2026-01-17 11:45:00 -0500"
  }
]$json$::jsonb,
    'MIT',
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

-- ── quickbooks-invoices (Intuit QuickBooks Online · Accounting) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'quickbooks-invoices',
    'quickbooks-invoices',
    'QuickBooks Online Invoice entity — core fields from the Accounting API. Compatible with Intuit''s documented Invoice entity; not affiliated with Intuit or QuickBooks.',
    'Intuit QuickBooks Online',
    'Accounting',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "quickbooks-invoices"
description: "QuickBooks Online Invoice entity — core fields from the Accounting API. Compatible with Intuit's documented Invoice entity; not affiliated with Intuit or QuickBooks."

ontology:
  entities:
    - name: Id
      type: string
      required: true
      pattern: "^[0-9]+$"
    - name: SyncToken
      type: string
      required: false
      pattern: "^[0-9]+$"
    - name: DocNumber
      type: string
      required: false
    - name: TxnDate
      type: date
      required: false
    - name: DueDate
      type: date
      required: false
    - name: CustomerRef
      type: object
      required: true
      properties:
        - name: value
          type: string
          required: true
          pattern: "^[0-9]+$"
        - name: name
          type: string
          required: false
    - name: CurrencyRef
      type: object
      required: false
      properties:
        - name: value
          type: string
          required: true
          pattern: "^[A-Z]{3}$"
        - name: name
          type: string
          required: false
    - name: TotalAmt
      type: number
      required: true
      min: 0
    - name: Balance
      type: number
      required: false
      min: 0

glossary:
  - field: Id
    description: "Unique identifier for the invoice within the QuickBooks company."
    constraints: "numeric string"
  - field: CustomerRef
    description: "Reference to the customer the invoice is for."
    constraints: "object with value (customer id) and optional name"
  - field: TotalAmt
    description: "Total amount of the transaction, including tax."
    constraints: "non-negative number"
$yaml$,
    '1.0',
    'https://developer.intuit.com/app/developer/qbo/docs/api/accounting/all-entities/invoice',
    '2026-11-05'::date,
    $json$[
  {
    "Id": "130",
    "SyncToken": "0",
    "DocNumber": "1070",
    "TxnDate": "2026-01-10",
    "DueDate": "2026-02-09",
    "CustomerRef": { "value": "24", "name": "Sample Client Co" },
    "CurrencyRef": { "value": "USD", "name": "United States Dollar" },
    "TotalAmt": 1250.00,
    "Balance": 1250.00
  },
  {
    "Id": "131",
    "SyncToken": "2",
    "DocNumber": "1071",
    "TxnDate": "2026-01-12",
    "DueDate": "2026-01-26",
    "CustomerRef": { "value": "25", "name": "Example Retail LLC" },
    "TotalAmt": 480.50,
    "Balance": 0
  },
  {
    "Id": "132",
    "DocNumber": "1072",
    "TxnDate": "2026-01-15",
    "CustomerRef": { "value": "26" },
    "CurrencyRef": { "value": "CAD" },
    "TotalAmt": 99.99,
    "Balance": 99.99
  }
]$json$::jsonb,
    'MIT',
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

-- ── hubspot-contacts (HubSpot · CRM) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'hubspot-contacts',
    'hubspot-contacts',
    'HubSpot CRM contact — default properties from the Contacts API / export. Compatible with HubSpot''s documented default contact properties; not affiliated with HubSpot.',
    'HubSpot',
    'CRM',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "hubspot-contacts"
description: "HubSpot CRM contact — default properties from the Contacts API / export. Compatible with HubSpot's documented default contact properties; not affiliated with HubSpot."

ontology:
  entities:
    - name: id
      type: string
      required: true
      pattern: "^[0-9]+$"
    - name: email
      type: string
      required: false
      format: email
    - name: firstname
      type: string
      required: false
    - name: lastname
      type: string
      required: false
    - name: phone
      type: string
      required: false
    - name: company
      type: string
      required: false
    - name: lifecyclestage
      type: string
      required: false
      enum:
        - "subscriber"
        - "lead"
        - "marketingqualifiedlead"
        - "salesqualifiedlead"
        - "opportunity"
        - "customer"
        - "evangelist"
        - "other"
    - name: hs_object_id
      type: string
      required: false
      pattern: "^[0-9]+$"
    - name: createdate
      type: timestamp
      required: false
    - name: lastmodifieddate
      type: timestamp
      required: false

glossary:
  - field: id
    description: "HubSpot record id for the contact."
    constraints: "numeric string"
  - field: lifecyclestage
    description: "The contact's stage in the marketing/sales lifecycle."
    constraints: "HubSpot default lifecycle stage value"
  - field: createdate
    description: "Timestamp when the contact was created."
    constraints: "RFC 3339 timestamp"
$yaml$,
    '1.0',
    'https://developers.hubspot.com/docs/api/crm/contacts',
    '2026-11-05'::date,
    $json$[
  {
    "id": "101",
    "email": "contact@example.com",
    "firstname": "Alex",
    "lastname": "Sample",
    "phone": "555-0100",
    "company": "Example Co",
    "lifecyclestage": "lead",
    "hs_object_id": "101",
    "createdate": "2026-01-05T10:00:00Z",
    "lastmodifieddate": "2026-01-20T14:30:00Z"
  },
  {
    "id": "102",
    "email": "jordan@example.org",
    "firstname": "Jordan",
    "lastname": "Test",
    "company": "Acme Test LLC",
    "lifecyclestage": "customer",
    "hs_object_id": "102",
    "createdate": "2026-01-06T08:15:00Z",
    "lastmodifieddate": "2026-01-21T09:00:00Z"
  },
  {
    "id": "103",
    "email": "casey@example.net",
    "firstname": "Casey",
    "lastname": "Demo",
    "phone": "555-0177",
    "lifecyclestage": "subscriber",
    "hs_object_id": "103",
    "createdate": "2026-01-07T12:45:00Z"
  }
]$json$::jsonb,
    'MIT',
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

-- ── salesforce-account (Salesforce · CRM) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'salesforce-account',
    'salesforce-account',
    'Salesforce Account object — core standard fields from the Object Reference. Compatible with Salesforce''s documented Account object; not affiliated with Salesforce.',
    'Salesforce',
    'CRM',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "salesforce-account"
description: "Salesforce Account object — core standard fields from the Object Reference. Compatible with Salesforce's documented Account object; not affiliated with Salesforce."

ontology:
  entities:
    - name: Id
      type: string
      required: true
      pattern: "^[A-Za-z0-9]{15,18}$"
    - name: Name
      type: string
      required: true
    - name: Type
      type: string
      required: false
      enum:
        - "Prospect"
        - "Customer - Direct"
        - "Customer - Channel"
        - "Channel Partner / Reseller"
        - "Installation Partner"
        - "Technology Partner"
        - "Other"
    - name: Industry
      type: string
      required: false
    - name: AnnualRevenue
      type: number
      required: false
      min: 0
    - name: NumberOfEmployees
      type: integer
      required: false
      min: 0
    - name: Phone
      type: string
      required: false
    - name: Website
      type: string
      required: false
      format: uri
    - name: BillingCity
      type: string
      required: false
    - name: BillingCountry
      type: string
      required: false
    - name: OwnerId
      type: string
      required: false
      pattern: "^[A-Za-z0-9]{15,18}$"
    - name: CreatedDate
      type: timestamp
      required: false

glossary:
  - field: Id
    description: "Salesforce 15- or 18-character record id."
    constraints: "15 or 18 alphanumeric characters"
  - field: Type
    description: "Type of account (standard picklist; may be customised per org)."
    constraints: "standard Account.Type value"
  - field: AnnualRevenue
    description: "Estimated annual revenue of the account."
    constraints: "non-negative number"
$yaml$,
    '1.0',
    'https://developer.salesforce.com/docs/atlas.en-us.object_reference.meta/object_reference/sforce_api_objects_account.htm',
    '2026-11-05'::date,
    $json$[
  {
    "Id": "001ExampleAcct0001",
    "Name": "Example Client Co",
    "Type": "Customer - Direct",
    "Industry": "Technology",
    "AnnualRevenue": 5000000,
    "NumberOfEmployees": 120,
    "Phone": "555-0100",
    "Website": "https://example.com",
    "BillingCity": "Austin",
    "BillingCountry": "USA",
    "OwnerId": "005ExampleUser0001",
    "CreatedDate": "2026-01-10T16:00:00Z"
  },
  {
    "Id": "001ExampleAcct0002",
    "Name": "Acme Test LLC",
    "Type": "Prospect",
    "Industry": "Manufacturing",
    "NumberOfEmployees": 40,
    "BillingCity": "Denver",
    "BillingCountry": "USA",
    "CreatedDate": "2026-01-11T09:30:00Z"
  },
  {
    "Id": "001ExampleAcct0003",
    "Name": "Sample Partner GmbH",
    "Type": "Channel Partner / Reseller",
    "AnnualRevenue": 0,
    "Website": "https://partner.example.org",
    "BillingCountry": "Germany"
  }
]$json$::jsonb,
    'MIT',
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

-- ── salesforce-opportunity (Salesforce · CRM) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'salesforce-opportunity',
    'salesforce-opportunity',
    'Salesforce Opportunity object — core standard fields from the Object Reference. Compatible with Salesforce''s documented Opportunity object; not affiliated with Salesforce.',
    'Salesforce',
    'CRM',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "salesforce-opportunity"
description: "Salesforce Opportunity object — core standard fields from the Object Reference. Compatible with Salesforce's documented Opportunity object; not affiliated with Salesforce."

ontology:
  entities:
    - name: Id
      type: string
      required: true
      pattern: "^[A-Za-z0-9]{15,18}$"
    - name: Name
      type: string
      required: true
    - name: AccountId
      type: string
      required: false
      pattern: "^[A-Za-z0-9]{15,18}$"
    - name: StageName
      type: string
      required: true
      enum:
        - "Prospecting"
        - "Qualification"
        - "Needs Analysis"
        - "Value Proposition"
        - "Id. Decision Makers"
        - "Perception Analysis"
        - "Proposal/Price Quote"
        - "Negotiation/Review"
        - "Closed Won"
        - "Closed Lost"
    - name: Amount
      type: number
      required: false
      min: 0
    - name: Probability
      type: number
      required: false
      min: 0
      max: 100
    - name: CloseDate
      type: date
      required: true
    - name: Type
      type: string
      required: false
      enum:
        - "Existing Customer - Upgrade"
        - "Existing Customer - Replacement"
        - "Existing Customer - Downgrade"
        - "New Customer"
    - name: IsClosed
      type: boolean
      required: false
    - name: IsWon
      type: boolean
      required: false
    - name: CreatedDate
      type: timestamp
      required: false

glossary:
  - field: StageName
    description: "Current stage of the opportunity (standard sales process picklist)."
    constraints: "standard Opportunity.StageName value"
  - field: CloseDate
    description: "Date when the opportunity is expected to close."
    constraints: "YYYY-MM-DD calendar date"
  - field: Probability
    description: "Percentage likelihood the opportunity will close."
    constraints: "0–100"
$yaml$,
    '1.0',
    'https://developer.salesforce.com/docs/atlas.en-us.object_reference.meta/object_reference/sforce_api_objects_opportunity.htm',
    '2026-11-05'::date,
    $json$[
  {
    "Id": "006ExampleOppy0001",
    "Name": "Example Co - New Platform",
    "AccountId": "001ExampleAcct0001",
    "StageName": "Proposal/Price Quote",
    "Amount": 48000,
    "Probability": 60,
    "CloseDate": "2026-03-31",
    "Type": "New Customer",
    "IsClosed": false,
    "IsWon": false,
    "CreatedDate": "2026-01-10T16:05:00Z"
  },
  {
    "Id": "006ExampleOppy0002",
    "Name": "Acme Test - Renewal",
    "AccountId": "001ExampleAcct0002",
    "StageName": "Closed Won",
    "Amount": 12000,
    "Probability": 100,
    "CloseDate": "2026-02-15",
    "Type": "Existing Customer - Upgrade",
    "IsClosed": true,
    "IsWon": true,
    "CreatedDate": "2026-01-11T09:40:00Z"
  },
  {
    "Id": "006ExampleOppy0003",
    "Name": "Sample Partner - Pilot",
    "StageName": "Qualification",
    "Probability": 20,
    "CloseDate": "2026-04-30",
    "IsClosed": false,
    "IsWon": false
  }
]$json$::jsonb,
    'MIT',
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

-- ── ga4-bigquery-events (Google Analytics 4 · Analytics) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'ga4-bigquery-events',
    'ga4-bigquery-events',
    'GA4 BigQuery Export events — core scalar columns of the daily events_ table. Compatible with Google Analytics 4''s documented BigQuery export schema; not affiliated with Google. Nested RECORD columns (event_params, user_properties, items) are omitted from this core contract.',
    'Google Analytics 4',
    'Analytics',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "ga4-bigquery-events"
description: "GA4 BigQuery Export events — core scalar columns of the daily events_ table. Compatible with Google Analytics 4's documented BigQuery export schema; not affiliated with Google. Nested RECORD columns (event_params, user_properties, items) are omitted from this core contract."

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
    - name: event_previous_timestamp
      type: integer
      required: false
      min: 0
    - name: event_bundle_sequence_id
      type: integer
      required: false
    - name: user_pseudo_id
      type: string
      required: false
    - name: user_id
      type: string
      required: false
    - name: platform
      type: string
      required: false
      enum: ["WEB", "ANDROID", "IOS"]
    - name: stream_id
      type: string
      required: false

glossary:
  - field: event_date
    description: "The date on which the event was logged (YYYYMMDD, in the property's reporting time zone)."
    constraints: "8-digit date string"
  - field: event_timestamp
    description: "The time (in microseconds, UTC) at which the event was logged on the client."
    constraints: "non-negative integer (microseconds)"
  - field: platform
    description: "The data stream platform the event came from."
    constraints: "WEB, ANDROID, or IOS"
$yaml$,
    '1.0',
    'https://support.google.com/analytics/answer/7029846',
    '2026-11-05'::date,
    $json$[
  {
    "event_date": "20260115",
    "event_timestamp": 1736935800000000,
    "event_name": "page_view",
    "user_pseudo_id": "1234567890.0987654321",
    "platform": "WEB",
    "stream_id": "1234567",
    "event_bundle_sequence_id": 1
  },
  {
    "event_date": "20260115",
    "event_timestamp": 1736935860000000,
    "event_name": "session_start",
    "user_pseudo_id": "1234567890.0987654321",
    "user_id": "user_0001",
    "platform": "ANDROID",
    "stream_id": "7654321",
    "event_previous_timestamp": 1736935800000000
  },
  {
    "event_date": "20260116",
    "event_timestamp": 1737022200000000,
    "event_name": "purchase",
    "user_pseudo_id": "2233445566.1122334455",
    "platform": "IOS",
    "stream_id": "7654321"
  }
]$json$::jsonb,
    'MIT',
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

-- ── square-payments (Square · Payments) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'square-payments',
    'square-payments',
    'Square Payment object — core fields from the Payments API. Compatible with Square''s documented Payment object; not affiliated with Square.',
    'Square',
    'Payments',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "square-payments"
description: "Square Payment object — core fields from the Payments API. Compatible with Square's documented Payment object; not affiliated with Square."

ontology:
  entities:
    - name: id
      type: string
      required: true
    - name: status
      type: string
      required: false
      enum: ["APPROVED", "PENDING", "COMPLETED", "CANCELED", "FAILED"]
    - name: amount_money
      type: object
      required: true
      properties:
        - name: amount
          type: integer
          required: true
          min: 0
        - name: currency
          type: string
          required: true
          pattern: "^[A-Z]{3}$"
    - name: source_type
      type: string
      required: false
      enum: ["CARD", "BANK_ACCOUNT", "WALLET", "CASH", "EXTERNAL"]
    - name: location_id
      type: string
      required: false
    - name: order_id
      type: string
      required: false
    - name: reference_id
      type: string
      required: false
    - name: receipt_number
      type: string
      required: false
    - name: created_at
      type: timestamp
      required: false
    - name: updated_at
      type: timestamp
      required: false

glossary:
  - field: amount_money
    description: "The amount of money processed for the payment."
    constraints: "object with integer amount (smallest currency unit) and ISO currency"
  - field: status
    description: "Status of the payment."
    constraints: "APPROVED, PENDING, COMPLETED, CANCELED, or FAILED"
  - field: source_type
    description: "The source type for the payment."
    constraints: "CARD, BANK_ACCOUNT, WALLET, CASH, or EXTERNAL"
$yaml$,
    '1.0',
    'https://developer.squareup.com/reference/square/payments-api',
    '2026-11-05'::date,
    $json$[
  {
    "id": "pmt_Example00000000000001",
    "status": "COMPLETED",
    "amount_money": { "amount": 4200, "currency": "USD" },
    "source_type": "CARD",
    "location_id": "LOC_EXAMPLE0001",
    "order_id": "ord_Example0001",
    "reference_id": "client-ref-1001",
    "receipt_number": "ABCD",
    "created_at": "2026-01-15T09:30:00Z",
    "updated_at": "2026-01-15T09:30:05Z"
  },
  {
    "id": "pmt_Example00000000000002",
    "status": "APPROVED",
    "amount_money": { "amount": 1599, "currency": "USD" },
    "source_type": "WALLET",
    "location_id": "LOC_EXAMPLE0001",
    "created_at": "2026-01-16T14:05:00Z"
  },
  {
    "id": "pmt_Example00000000000003",
    "status": "FAILED",
    "amount_money": { "amount": 0, "currency": "CAD" },
    "source_type": "CARD",
    "location_id": "LOC_EXAMPLE0002",
    "created_at": "2026-01-17T11:45:00Z"
  }
]$json$::jsonb,
    'MIT',
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

-- ── xero-invoices (Xero · Accounting) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'xero-invoices',
    'xero-invoices',
    'Xero Invoice — core fields from the Accounting API Invoices endpoint. Compatible with Xero''s documented Invoice object; not affiliated with Xero.',
    'Xero',
    'Accounting',
    NULL,
    'json',
    $yaml$
version: "1.0"
name: "xero-invoices"
description: "Xero Invoice — core fields from the Accounting API Invoices endpoint. Compatible with Xero's documented Invoice object; not affiliated with Xero."

ontology:
  entities:
    - name: InvoiceID
      type: string
      required: false
      pattern: "^[0-9a-fA-F-]{36}$"
    - name: InvoiceNumber
      type: string
      required: false
    - name: Type
      type: string
      required: true
      enum: ["ACCREC", "ACCPAY"]
    - name: Contact
      type: object
      required: true
      properties:
        - name: ContactID
          type: string
          required: false
          pattern: "^[0-9a-fA-F-]{36}$"
        - name: Name
          type: string
          required: true
    - name: Date
      type: date
      required: false
    - name: DueDate
      type: date
      required: false
    - name: Status
      type: string
      required: false
      enum: ["DRAFT", "SUBMITTED", "AUTHORISED", "PAID", "VOIDED", "DELETED"]
    - name: LineAmountTypes
      type: string
      required: false
      enum: ["Exclusive", "Inclusive", "NoTax"]
    - name: CurrencyCode
      type: string
      required: false
      pattern: "^[A-Z]{3}$"
    - name: SubTotal
      type: number
      required: false
      min: 0
    - name: TotalTax
      type: number
      required: false
      min: 0
    - name: Total
      type: number
      required: false
      min: 0
    - name: AmountDue
      type: number
      required: false
      min: 0
    - name: AmountPaid
      type: number
      required: false
      min: 0

glossary:
  - field: Type
    description: "Invoice type: ACCREC (accounts receivable / sales) or ACCPAY (accounts payable / bills)."
    constraints: "ACCREC or ACCPAY"
  - field: Contact
    description: "The contact the invoice is issued to or from."
    constraints: "object with Name and optional ContactID"
  - field: Status
    description: "Invoice status in Xero."
    constraints: "Xero invoice status value"
$yaml$,
    '1.0',
    'https://developer.xero.com/documentation/api/accounting/invoices',
    '2026-11-05'::date,
    $json$[
  {
    "InvoiceID": "00000000-0000-4000-8000-000000000001",
    "InvoiceNumber": "INV-1001",
    "Type": "ACCREC",
    "Contact": { "ContactID": "00000000-0000-4000-8000-0000000000c1", "Name": "Example Client Co" },
    "Date": "2026-01-10",
    "DueDate": "2026-02-09",
    "Status": "AUTHORISED",
    "LineAmountTypes": "Exclusive",
    "CurrencyCode": "USD",
    "SubTotal": 1000.00,
    "TotalTax": 80.00,
    "Total": 1080.00,
    "AmountDue": 1080.00,
    "AmountPaid": 0
  },
  {
    "InvoiceID": "00000000-0000-4000-8000-000000000002",
    "InvoiceNumber": "INV-1002",
    "Type": "ACCREC",
    "Contact": { "Name": "Acme Test LLC" },
    "Date": "2026-01-12",
    "DueDate": "2026-01-26",
    "Status": "PAID",
    "CurrencyCode": "USD",
    "SubTotal": 450.00,
    "TotalTax": 36.00,
    "Total": 486.00,
    "AmountDue": 0,
    "AmountPaid": 486.00
  },
  {
    "InvoiceNumber": "BILL-5001",
    "Type": "ACCPAY",
    "Contact": { "Name": "Sample Supplier Ltd" },
    "Date": "2026-01-15",
    "Status": "SUBMITTED",
    "CurrencyCode": "GBP",
    "Total": 220.00,
    "AmountDue": 220.00
  }
]$json$::jsonb,
    'MIT',
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

-- ── generic-sheets-transactions-csv (DataContractGate · Spreadsheets) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'generic-sheets-transactions-csv',
    'generic-sheets-transactions-csv',
    'Generic transactions / ledger CSV — a practical spreadsheet format for bank, ledger, and bookkeeping exports. Authored by DataContractGate as an open spec; not tied to any vendor.',
    'DataContractGate',
    'Spreadsheets',
    NULL,
    'csv',
    $yaml$
version: "1.0"
name: "generic-sheets-transactions-csv"
description: "Generic transactions / ledger CSV — a practical spreadsheet format for bank, ledger, and bookkeeping exports. Authored by DataContractGate as an open spec; not tied to any vendor."

ontology:
  entities:
    - name: date
      type: date
      required: true
    - name: description
      type: string
      required: true
      min_length: 1
    - name: amount
      type: number
      required: true
    - name: currency
      type: string
      required: true
      pattern: "^[A-Z]{3}$"
    - name: category
      type: string
      required: false
    - name: account
      type: string
      required: false
    - name: reference
      type: string
      required: false
    - name: balance
      type: number
      required: false

glossary:
  - field: date
    description: "Transaction date (YYYY-MM-DD)."
    constraints: "calendar date"
  - field: amount
    description: "Signed transaction amount; negative for debits, positive for credits."
    constraints: "number, may be negative"
  - field: currency
    description: "Three-letter ISO currency code, uppercase."
    constraints: "ISO 4217, uppercase"
  - field: balance
    description: "Running account balance after this transaction, if provided."
    constraints: "number"
$yaml$,
    '1.0',
    'https://app.datacontractgate.com/catalog/generic-sheets-transactions-csv',
    '2026-11-05'::date,
    $json$[
  {
    "date": "2026-01-03",
    "description": "Client retainer - January",
    "amount": 2500.00,
    "currency": "USD",
    "category": "Income",
    "account": "Operating",
    "reference": "INV-1001",
    "balance": 2500.00
  },
  {
    "date": "2026-01-05",
    "description": "SaaS subscription",
    "amount": -49.00,
    "currency": "USD",
    "category": "Software",
    "account": "Operating",
    "reference": "sub_0001",
    "balance": 2451.00
  },
  {
    "date": "2026-01-08",
    "description": "Bank fee",
    "amount": -12.50,
    "currency": "USD",
    "category": "Fees",
    "account": "Operating",
    "balance": 2438.50
  }
]$json$::jsonb,
    'MIT',
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

-- ── acs-5year-2023-county (US Census Bureau · Public data) ───────────────────────────
INSERT INTO public_contracts (
    name, slug, description, vendor, category,
    source_url, source_format, contract_yaml, version,
    verified_source_url, verified_at, sample_json, license_note, author_org_id
) VALUES (
    'acs_5year_2023_county',
    'acs-5year-2023-county',
    'ACS 5-Year Estimates 2023 — county-level population and income. Source: US Census Bureau (data.census.gov). No API key required for < 500 requests/day.',
    'US Census Bureau',
    'Public data',
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
    '1.0',
    'https://www.census.gov/data/developers/data-sets/acs/acs5.html',
    '2026-11-05'::date,
    $json$[
  {
    "NAME": "Example County, Teststate",
    "B01003_001E": 55200,
    "B19013_001E": 62000,
    "B19013_001MA": 2500,
    "state": "99",
    "county": "001"
  },
  {
    "NAME": "Sample County, Teststate",
    "B01003_001E": 12840,
    "B19013_001E": 48750,
    "B19013_001MA": 3100,
    "state": "99",
    "county": "002"
  },
  {
    "NAME": "Demo County, Teststate",
    "B01003_001E": 304100,
    "B19013_001E": 81250,
    "B19013_001MA": 1800,
    "state": "99",
    "county": "003"
  }
]$json$::jsonb,
    'MIT',
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
