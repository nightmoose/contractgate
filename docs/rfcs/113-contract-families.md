# RFC-113 — Contract families

**Status:** Accepted
**Date:** 2026-11-09

## Problem

The same dataset is checked at more than one pipeline boundary: raw ingest, a cleaned stage, a mart, and an egress feed. Each shape is its own contract. Nothing in the product records that those contracts are one chain, so the catalog cannot show a neighbor, and an org cannot say which contract a contract was derived from.

`examples/contract-chain/stripe/` already writes that chain in prose (`derived_from` inside the description). That is documentation. It is not a relation the API or the catalog can walk.

## Decision

A family is a named group. A link is one contract in that group:

- `derived_from` — the previous contract. Null on the root.
- `boundary` — `ingest`, `stage` (between stages), or `egress`. Writers may send `between_stages`; it is stored as `stage`.

The links must be a single chain: one root, each contract the parent of at most one other, every contract reachable from the root. The server assigns `position` by walking that chain.

Private families live on `contract_families` and `contract_family_links`, scoped to the owning org the same way contract identity is. A contract belongs to at most one family. Public catalog families use the same shape on `public_contract_families` and `public_contract_family_links`. The Stripe charges example is the first seeded public family (`stripe-charges-pipeline`).

The contract YAML schema does not gain a field. `derived_from` in a description stays prose.

## Non-goals

- Community publishing and author rewards. Seeded catalog rows keep `author_org_id` null.
- A per-run `batch_id` on quarantine.
- Replacing dbt, Great Expectations, or Soda. A mart contract is still a between-stages (or egress) check alongside those tools.

## API

See [`docs/contract-families-reference.md`](../contract-families-reference.md).
