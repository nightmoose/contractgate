# Contract families — API reference

**RFC:** 113
**Migration:** `supabase/migrations/050_contract_families.sql`

A family relates contracts that describe the same data as it changes shape through a pipeline. Each link says which contract this one is derived from and which boundary it sits at.

| `boundary` | Where the contract sits | Gateway |
|---|---|---|
| `ingest` | Source → raw | `POST /v1/ingest/{contract_id}` |
| `stage` | Between stages (raw → staging, staging → mart) | `POST /v1/ingest/{contract_id}` |
| `egress` | Mart → reverse ETL / partner / API | `POST /egress/{contract_id}` |

`between_stages` is accepted on write and stored as `stage`. Responses always use `ingest`, `stage`, or `egress`.

A family is one chain. Exactly one contract has no `derived_from` (the root). Every other contract names a member of the same family. Each contract is the parent of at most one other. The server numbers `position` from 0 at the root.

Auth matches the contract routes: `X-Api-Key`. The caller's org owns the family and every contract in it. Another org's id is `404`. When auth is off (local dev), create still needs an `x-org-id` header, the same as `POST /contracts`.

The contract YAML document does not grow a new field. A `derived_from` line inside `description` is prose. The relation the API walks is the family link.

## Private contracts

### `POST /contract-families`

Create a family. `links` may be empty, or the full chain.

```json
{
  "name": "Stripe charges pipeline",
  "description": "Raw ingest through the partner payout feed.",
  "links": [
    { "contract_id": "…raw…", "boundary": "ingest" },
    {
      "contract_id": "…staged…",
      "derived_from_contract_id": "…raw…",
      "boundary": "stage"
    }
  ]
}
```

`201` returns the family with `links` ordered root to leaf. `400` when the links are not one chain. `404` when a contract is not in this org. `409` when the name is already used in the org, or a contract is already in a family.

### `GET /contract-families`

List the caller's families. Each row includes `link_count`.

### `GET /contract-families/{id}`

One family, including `links` in chain order.

### `POST /contract-families/{id}/links`

Add one contract. The set of links must still be one chain. Extending the current tail is the usual call. `201` returns the updated family.

The first link on an empty family must be the root (`derived_from_contract_id` omitted).

### `GET /contracts/{id}/family`

The family that contains this contract. `404` when the contract is not in the caller's org, or is not in a family.

## Public catalog

No API key.

| Method | Path |
|---|---|
| `GET` | `/public-contract-families` |
| `GET` | `/public-contract-families/{id}` |
| `GET` | `/public-contracts/{id}/family` |

`{id}` is a UUID or a slug. Each family includes `links` ordered from the root. `GET /public-contracts/{id}` includes the same object as `family` when the contract is in a chain, and omits `family` otherwise.

The seeded family is `stripe-charges-pipeline`:

| Position | Contract | Slug | Boundary | Derived from |
|---|---|---|---|---|
| 0 | `stripe_charges_raw` | `stripe-charges-raw` | ingest | — |
| 1 | `stripe_charges_staged` | `stripe-charges-staged` | stage | `stripe_charges_raw` |
| 2 | `revenue_daily_mart` | `revenue-daily-mart` | stage | `stripe_charges_staged` |
| 3 | `partner_payout_feed` | `partner-payout-feed` | egress | `revenue_daily_mart` |

The YAML and the one synthetic sample on each row come from `examples/contract-chain/stripe/`. Not affiliated with Stripe. Sample ids are fake. These rows are in addition to the existing `stripe-charges` catalog contract.

The catalog contract page shows the chain and links to the neighbors.

## Database

| Table | Scope |
|---|---|
| `contract_families` | Org-owned group |
| `contract_family_links` | `derived_from_contract_id`, `boundary`, `position` |
| `public_contract_families` | Catalog group |
| `public_contract_family_links` | Same link shape for `public_contracts` |

Row level security is on and there is no `anon` or `authenticated` policy. The gateway connects as a role that bypasses RLS, the same as `public_contracts`. A contract id appears in at most one family.

## Applying migration 050

Apply `supabase/migrations/050_contract_families.sql` **before** deploying a gateway that reads these tables. The file is additive: four new tables, four catalog contracts, and the Stripe family. Existing catalog slugs are unchanged. Apply it after `049_quarantine_batch_id.sql`.

Hosted production is applied by the product owner, not by this repository's deploy. After the SQL succeeds, record the file in `supabase_migrations.schema_migrations` so the drift check matches. `name` is `050_contract_families`. `version` is a timestamp that sorts after the `049_quarantine_batch_id` row (see `scripts/backfill_schema_migrations.sql` for the column shape).

Self-host applies the file through `ops/postgres/selfhost-apply.sh`, which records the filename itself.

A gateway deployed before the migration will fail `GET /public-contracts/{slug}` with a missing-relation error, because that response loads the family. Apply the file first.
