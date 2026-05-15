# RFC-030: Egress PII & Leakage Guard

**Status:** Draft
**Date:** 2026-05-14
**Author:** Alex Suarez

---

## Problem

RFC-004 added per-field PII transforms at **ingest**: `mask`, `hash`,
`drop`, `redact`, plus a `compliance_mode` flag that rejects undeclared
fields. The invariant RFC-004 guarantees is *"raw PII never lands in
durable storage."*

Egress has the mirror-image problem with a sharper edge: **raw PII /
internal fields never leave the API.** When ContractGate runs as a
last-step gate (RFC-029), the outbound payload is assembled by the
caller's own code — joins, derived fields, internal bookkeeping
columns. That is exactly where leakage happens:

- An internal `cost_basis` or `risk_score` field accidentally serialized
  into a customer-facing response.
- A `user_email` that should be hashed for a downstream partner but is
  shipped raw.
- A debug field that a contract author forgot to mark, riding along in
  every response.

Ingest validation cannot catch any of this — the data was assembled
*after* ingest. Only an egress guard can.

## Proposed Solution

Run the **RFC-004 transform engine on the egress path.** No new
transform kinds, no new engine — `src/transform.rs::apply_transforms`
is reused verbatim. A field declared `drop` is dropped on the way out;
a field declared `hash` ships hashed; `redact` ships redacted.

Two additions specific to egress:

### 1. Egress leakage mode

Ingest `compliance_mode` strips undeclared fields so raw values do not
*land on disk*. On egress, silently stripping an undeclared field can
*hide a real bug* — you were about to leak something and never found
out. So egress gets a three-way mode instead of a boolean:

| `egress_leakage_mode` | Behavior on an undeclared field in the outbound payload |
|-----------------------|----------------------------------------------------------|
| `off`                 | Pass through untouched (backwards compatible default). |
| `strip`               | Remove the field from the response. Record it in the egress outcome. |
| `fail`                | Treat as a violation — the record fails under the RFC-029 disposition (`block` / `fail` / `tag`). |

### 2. Salt continuity

Egress `hash` transforms reuse the **same** `contracts.pii_salt`
introduced by RFC-004. A value hashed on ingest and the same value
hashed on egress produce identical output — so downstream joins on
hashed keys stay consistent across both directions. The salt is still
never serialized, never returned by any API.

### Pipeline (extends RFC-029)

```
outbound payload
        │
        ▼
┌──────────────────────────┐
│ validate()  (RFC-029)    │  ← sees raw values; rule checks run on
│                          │    original data, exactly as ingest
└────────────┬─────────────┘
             │
             ▼
┌──────────────────────────┐
│ apply_transforms()       │  ← RFC-004 engine, reused verbatim.
│  + egress_leakage_mode   │    drop/hash/mask/redact on declared
│                          │    fields; strip|fail undeclared fields
└────────────┬─────────────┘
             │
             ▼
   cleaned payload returned to caller + audit_log (direction='egress')
```

Key invariant: **the cleaned payload returned by `POST /egress/...` is
the post-transform payload, never the raw one.** Same end-to-end
honesty as RFC-004's "raw PII never leaves the validator" — extended to
"raw PII never leaves the API."

### Schema (proposed)

```sql
-- migration 018_egress_leakage_guard.sql
ALTER TABLE contract_versions
    ADD COLUMN egress_leakage_mode text NOT NULL DEFAULT 'off'
    CHECK (egress_leakage_mode IN ('off', 'strip', 'fail'));
```

No new salt column — RFC-004's `contracts.pii_salt` is reused. No new
transform columns — RFC-004's inline `transform:` blocks are reused.

### Integration points

- **Transform engine:** `src/transform.rs::apply_transforms` reused
  with zero changes.
- **Egress handler:** `src/egress.rs` (RFC-029) calls `apply_transforms`
  after `validate()`, then applies `egress_leakage_mode`.
- **Contract schema:** `src/contract.rs` version-level struct gains
  `egress_leakage_mode` (parsed from the version, not the locked
  ontology block).
- **Dashboard:** the existing RFC-004 transform dropdowns already cover
  egress — a field marked `drop` is dropped both ways. Add an
  "Egress leakage" mode selector at the version level next to the
  existing "Compliance mode" toggle.

## Out of Scope (this RFC)

- **Per-direction transforms.** In v1 a field has *one* `transform:`
  block applied in both directions. "Stored hashed, but redacted
  entirely on egress" is not expressible. If a real use case appears it
  becomes a future RFC — RFC-004 already ruled out transform stacking
  for the same reasons.
- **Static deny-list.** A global "these field names never leave,
  contract or not" list is appealing but is a separate policy surface;
  egress `compliance`/`fail` mode covers the declared-contract case
  first.
- **Salt rotation.** Still deferred, same as RFC-004.
- **Egress validation core** — that is RFC-029, which this RFC depends
  on.

## Open Questions

1. **One flag or two.** Reuse RFC-004's `compliance_mode` for egress
   too, or keep `egress_leakage_mode` separate? Recommendation:
   separate — ingest wants a boolean (strip-for-storage), egress wants
   three-way (strip vs fail), and a contract may legitimately want
   strict ingest but lenient egress or vice versa.
2. **Default.** `egress_leakage_mode` default — `off` or `strip`?
   Recommendation: `off`, for strict backwards compatibility; a
   contract opts in.
3. **Echo on `fail`.** When `egress_leakage_mode = 'fail'` and a record
   fails, does the failure outcome carry the offending field *name*
   only, or name + transformed value? Recommendation: name only — the
   value may itself be the thing you must not leak.
4. **Interaction with RFC-029 `tag` disposition.** If leakage mode is
   `fail` but the RFC-029 disposition is `tag`, the record passes
   through tagged — does the undeclared field still get stripped?
   Recommendation: yes, strip the field even on `tag`; `tag` flags the
   record, it does not waive the leakage guarantee.

## Acceptance Criteria

- [ ] Migration `018_egress_leakage_guard.sql` adds
      `contract_versions.egress_leakage_mode`
- [ ] Egress path applies RFC-004 `transform:` blocks
      (`drop`/`hash`/`mask`/`redact`) to the outbound payload
- [ ] `egress_leakage_mode` of `off` / `strip` / `fail` all behave per
      the table above
- [ ] Egress `hash` output equals ingest `hash` output for the same
      value on the same contract (salt continuity)
- [ ] The payload returned by `POST /egress/...` is always the
      post-transform payload
- [ ] `contracts.pii_salt` still never appears in any API response
- [ ] `cargo test` / `cargo check` pass; RFC-004 ingest behavior
      unchanged
- [ ] `docs/pii-masking-reference.md` updated (or created) to document
      egress behavior — this changes user-facing transform semantics

---

## Dependency Chain

Depends on **RFC-029** (egress path + `direction` column) and reuses
**RFC-004** (transform engine, `pii_salt`). Should ship after RFC-029.
