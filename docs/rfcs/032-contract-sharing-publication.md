# RFC-032: Contract Sharing & Publication

**Status:** Draft
**Date:** 2026-05-14
**Author:** Alex Suarez

---

## Problem

A contract is the single best onboarding artifact between a provider and
a consumer — it spells out every field, type, enum, and rule. Today
there is no way to *share* one. A provider who wants a consumer to send
(or receive) conformant data has to describe the shape informally —
email, a PDF, a Slack thread — and the consumer reconstructs the
contract by hand and hopes it matches.

This is backwards. The contract already exists, it is already
queryable (RFC-028 mirrored it into Supabase), and it is the exact spec
the consumer needs. The only thing missing is a distribution mechanism:
a provider should be able to **publish** a contract and a consumer
should be able to **import** it directly, so the consumer's side is
configured from the authoritative definition instead of guesswork.

Findigs case: a PMS vendor that publishes its egress contract lets
Findigs import it and stand up a matching ingest contract in one step —
no back-and-forth, no drift between "what the PMS says it sends" and
"what Findigs configured."

## Proposed Solution

A **publish → import** flow built on the RFC-028 contract store.

### Publish

A provider marks a specific contract *version* as published. This
generates a stable **publication reference** — an opaque, shareable id
— and a visibility setting:

| Visibility    | Who can fetch the published contract |
|---------------|--------------------------------------|
| `public`      | Anyone with the publication ref. |
| `link`        | Anyone with the ref *and* an unguessable token. |
| `org`         | Only orgs explicitly granted access (sets up RFC-033). |

Publishing pins a version — the published artifact is immutable. A new
provider version is a new publication (or an update to a `link`/`org`
publication that subscribers can pull; see Open Questions).

### Import

A consumer imports a published contract by reference. Two import modes:

| Mode         | Behavior |
|--------------|----------|
| `snapshot`   | One-time copy into the consumer's org. Records provenance, never auto-changes. |
| `subscribe`  | Copy plus a live link — when the provider publishes a newer version, the consumer sees an *available update* and can pull it (explicit, never silent). |

Either way the imported contract carries **provenance** — it knows it
came from publication ref X at version V — so "is our copy current?"
is a query, not an investigation.

### Schema (proposed)

```sql
-- migration 020_contract_publication.sql

CREATE TABLE contract_publications (
  ref            text PRIMARY KEY DEFAULT encode(gen_random_bytes(12), 'hex'),
  contract_name  text NOT NULL,
  version        text NOT NULL,
  visibility     text NOT NULL DEFAULT 'link'
                 CHECK (visibility IN ('public', 'link', 'org')),
  link_token     text,                       -- non-null when visibility = 'link'
  published_by   text NOT NULL,
  published_at   timestamptz NOT NULL DEFAULT now(),
  revoked        boolean NOT NULL DEFAULT false,
  FOREIGN KEY (contract_name, version) REFERENCES contracts (name, version)
);

-- provenance on imported contracts
ALTER TABLE contracts
    ADD COLUMN imported_from_ref     text,    -- contract_publications.ref
    ADD COLUMN imported_from_version text,    -- version pinned at import time
    ADD COLUMN import_mode           text     -- 'snapshot' | 'subscribe' | NULL
        CHECK (import_mode IN ('snapshot', 'subscribe'));
```

`contracts` already has an `ImportSource` enum (RFC-010, ODCS import) —
publication import is a new `ImportSource` variant, reusing that
machinery rather than inventing a parallel path.

### API

- `POST /contracts/{name}/versions/{v}/publish` — publish a version,
  returns `{ ref, visibility, link_token? }`.
- `DELETE /contracts/publications/{ref}` — revoke a publication.
- `GET /published/{ref}` — fetch a published contract (read-only;
  honors visibility; `link` requires `?token=`). Returns the locked
  YAML plus metadata.
- `POST /contracts/import-published` — import by `{ ref, token?, mode }`
  into the caller's org.
- `GET /contracts/{name}/import-status` — for `subscribe` imports, is a
  newer published version available?

### What this unlocks

- **One-step consumer onboarding.** Import the provider's contract,
  done — no manual reconstruction.
- **No spec drift.** The consumer's config *is* the provider's
  definition, with provenance proving it.
- **A public contract directory becomes possible.** `public`
  visibility + the RFC-028 store is the seed of a browsable catalog
  (future RFC).
- **Sets up collaboration.** `org` visibility is the hook RFC-033 hangs
  the joint editing / review model on.

### Integration points

- **Contract store:** publications reference the RFC-028 `contracts`
  table directly.
- **Import path:** reuse `src/odcs.rs` / `ImportSource` machinery; add a
  `Publication` variant.
- **New module:** `src/publication.rs` — publish, revoke, fetch,
  import-published, import-status handlers.
- **Routing:** new routes under `/contracts/.../publish`, `/published`,
  `/contracts/import-published`.
- **Dashboard:** a "Publish" action on a contract version; an "Import
  from reference" entry point next to the existing ODCS import.

## Out of Scope (this RFC)

- **Joint editing / review / comments** between provider and consumer —
  RFC-033. This RFC is one-directional distribution only.
- **A browsable public contract catalog UI** — `public` visibility
  makes it possible; the catalog itself is a later RFC.
- **Auto-pull of subscribed updates.** `subscribe` mode *surfaces* an
  available update; it never applies one silently. Auto-apply is
  explicitly not in scope.
- **Cross-instance federation** (publishing across separate
  ContractGate deployments). v1 assumes one deployment; the `ref` is
  designed to not preclude federation later.

## Open Questions

1. **Default import mode.** `snapshot` or `subscribe`? Recommendation:
   `snapshot` — a copy with no live coupling is the least surprising
   default; `subscribe` is opt-in.
2. **What `subscribe` pulls.** When a provider publishes a new version,
   does a `subscribe` consumer see it as "update available" against the
   *same* publication ref, or is every version a fresh ref?
   Recommendation: the ref is stable per (contract, visibility);
   publishing a new version updates what the ref resolves to, and
   `import-status` reports the delta.
3. **Format on the wire.** Publish/fetch the locked ContractGate YAML,
   or the ODCS export form (RFC-010 already has ODCS export)?
   Recommendation: locked YAML as the canonical form; offer ODCS as an
   alternate representation on `GET /published/{ref}?format=odcs`.
4. **Revocation semantics.** When a publication is revoked, what
   happens to consumers who already imported it — nothing (they keep
   their copy), or `import-status` flags it revoked? Recommendation:
   they keep their copy; `import-status` reports `source_revoked`.
5. **Org scoping.** `org` visibility needs an "org granted access"
   list. Is that list managed here, or deferred entirely to RFC-033?
   Recommendation: RFC-032 ships `public` + `link` only; `org`
   visibility lands with RFC-033 so the access model is designed once.

## Acceptance Criteria

- [ ] Migration `020_contract_publication.sql` adds
      `contract_publications` and the `imported_from_*` provenance
      columns on `contracts`
- [ ] `POST /contracts/{name}/versions/{v}/publish` returns a stable
      publication ref with `public` or `link` visibility
- [ ] `GET /published/{ref}` returns the locked YAML + metadata,
      honoring visibility and `link` tokens
- [ ] `POST /contracts/import-published` imports a published contract
      in `snapshot` or `subscribe` mode with provenance recorded
- [ ] `GET /contracts/{name}/import-status` reports whether a newer
      published version exists for a `subscribe` import
- [ ] Revoking a publication is reflected in `import-status`
- [ ] `cargo test` / `cargo check` pass; existing ODCS import behavior
      unchanged
- [ ] `docs/contract-sharing-reference.md` added — new user-facing
      publish/import endpoints

---

## Dependency Chain

Depends on **RFC-028** (queryable contract store) and reuses
**RFC-010** import machinery. `org` visibility is intentionally
deferred to **RFC-033**, which builds the full provider-consumer
collaboration model on top of this distribution layer.
