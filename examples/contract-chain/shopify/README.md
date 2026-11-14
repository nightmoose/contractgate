# Shopify orders contract chain (example)

**Linked contracts that walk a Shopify orders export from ingest → stage → egress.**

Public brand: **DataContractGate**. Not affiliated with Shopify. Sample order names (`#1001`) and notice ids (`ntc_demo_…`) are fake.

This folder is the seed for the public catalog family `shopify-orders-pipeline` (migration `052_shopify_and_ga4_contract_families.sql`). The raw contract mirrors [`catalog/seeds/shopify-orders`](../../../catalog/seeds/shopify-orders/). See [`docs/contract-families-reference.md`](../../../docs/contract-families-reference.md).

## Contracts

| Stage | Contract | Gate | Endpoint |
|---|---|---|---|
| Raw ingest | `shopify_orders_raw` | Ingest | `POST /v1/ingest/{contract_id}` |
| Typed order | `shopify_orders_staged` | Stage | `POST /v1/ingest/{contract_id}` |
| Fulfillment notice | `shopify_fulfillment_notice` | Egress | `POST /egress/{contract_id}` |

`derived_from` inside each description is prose. The relation the API walks is the family link.

## Files

```
shopify_orders_raw.yaml              + .pass.json / .fail.json
shopify_orders_staged.yaml           + .pass.json / .fail.json
shopify_fulfillment_notice.yaml      + .pass.json / .fail.json
```

Each `.pass.json` validates with 0 violations. Each `.fail.json` is an intentional violation (enum, case, pattern, or range).
