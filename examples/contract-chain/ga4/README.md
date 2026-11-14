# GA4 events contract chain (example)

**Linked contracts that walk a GA4 BigQuery events export from ingest → stage → egress.**

Public brand: **DataContractGate**. Not affiliated with Google. Sample export ids (`aud_demo_…`) are fake.

This folder is the seed for the public catalog family `ga4-events-pipeline` (migration `052_shopify_and_ga4_contract_families.sql`). The raw contract mirrors [`catalog/seeds/ga4-bigquery-events`](../../../catalog/seeds/ga4-bigquery-events/). See [`docs/contract-families-reference.md`](../../../docs/contract-families-reference.md).

## Contracts

| Stage | Contract | Gate | Endpoint |
|---|---|---|---|
| Raw ingest | `ga4_events_raw` | Ingest | `POST /v1/ingest/{contract_id}` |
| Cleaned event | `ga4_events_staged` | Stage | `POST /v1/ingest/{contract_id}` |
| Audience export | `ga4_audience_export` | Egress | `POST /egress/{contract_id}` |

`derived_from` inside each description is prose. The relation the API walks is the family link.

## Files

```
ga4_events_raw.yaml           + .pass.json / .fail.json
ga4_events_staged.yaml        + .pass.json / .fail.json
ga4_audience_export.yaml      + .pass.json / .fail.json
```

Each `.pass.json` validates with 0 violations. Each `.fail.json` is an intentional violation (enum, pattern, or range).
