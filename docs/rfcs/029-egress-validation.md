# RFC-029: Egress Validation — Contract Enforcement on the Outbound Path

**Status:** Draft
**Date:** 2026-05-14
**Author:** Alex Suarez

---

## Problem

ContractGate validates data on **ingest**. The patent core is the
inbound semantic validation engine. But the consumers who run
ContractGate also expose *their own* APIs downstream — and today they
have no symmetric guarantee that what *leaves* their API conforms to a
contract.

Bad data still escapes through three gaps:

- Data that slipped past ingest before a contract tightened.
- Internally-assembled payloads (joins, derived fields, transforms) that
  were never validated against anything.
- Partially-degraded upstream providers whose data passed a loose
  ingest contract but is not fit to re-serve.

Concrete case (Findigs): Findigs pulls rental data from many small PMS
(property management system) APIs. Those PMS vendors are small and hate
fielding data-quality complaints. Findigs validates the PMS data on
ingest — but when Findigs serves *its own* customers (lenders,
landlords), there is no last-step gate proving the outbound payload is
clean. The same validation engine, pointed outward, closes the loop.

## Proposed Solution

Add an **egress validation mode**: the existing `validate()` engine,
run against outbound payloads before they leave the API. No fork of the
engine — the contract, the compiled form, and the per-field rule
checks are identical to ingest. Only the *direction* and the
*disposition* differ.

Two deployment shapes, one mechanism:

1. **Last step** — ContractGate sits in front of the response. The
   caller hands the assembled payload to `POST /egress/{contract}`,
   gets back a cleaned payload plus per-record outcomes.
2. **Pre-egress gate** — the same call made *before* the response is
   assembled, so bad records are caught internally and never reach the
   serialization step.

### Egress disposition

Ingest quarantines failing events. Egress needs a choice — what to do
with a failing record *on the way out*:

| Disposition | Behavior |
|-------------|----------|
| `block`     | Drop the failing record from the response. Good records still ship. (Partial-response default.) |
| `fail`      | Any failing record fails the whole response (`422`). Atomic mode. |
| `tag`       | Failing record passes through, flagged in the per-record outcome so the caller can decide. |

### Pipeline

```
outbound payload (assembled by caller's API)
        │
        ▼
POST /egress/{contract}   ── or ──  egress middleware (same code path)
        │
        ▼
┌──────────────────────────┐
│ validate()               │  ← identical engine to ingest. Same
│                          │    compiled contract, same rule checks.
└────────────┬─────────────┘
       pass  │  fail
             │   │
             │   └──► disposition: block | fail | tag
             │
             ▼
   include in response, write audit_log row (direction = 'egress')
        │
        ▼
return { payload: cleaned, outcomes: [...] }
```

Key invariant: **the egress path reuses `validate()` verbatim.** It does
not re-implement any rule logic. This keeps the patent-core engine the
single source of truth and keeps the `<15ms p99` budget measurable on
one code path.

### Schema (proposed)

Egress events belong in the same queryable store as ingest events, so
they can be joined and reported on together (see RFC-031). Add a
direction discriminator rather than a parallel table:

```sql
-- migration 017_egress_validation.sql
ALTER TABLE audit_log
    ADD COLUMN direction text NOT NULL DEFAULT 'ingress'
    CHECK (direction IN ('ingress', 'egress'));

ALTER TABLE quarantine_events
    ADD COLUMN direction text NOT NULL DEFAULT 'ingress'
    CHECK (direction IN ('ingress', 'egress'));
```

Existing rows default to `ingress` — no behavior change for any current
query. New egress writes set `direction = 'egress'`.

### What this unlocks

- **Last-step API gate.** A consumer can guarantee — and advertise —
  that every payload its API emits passed a named contract.
- **Pre-egress internal gate.** Bad records are stopped before
  serialization, not after a customer complaint.
- **Symmetric contract reuse.** One contract file governs both
  directions. A provider's ingest contract *is* the consumer's egress
  contract (see RFC-032).
- **Unified audit trail.** `direction` makes "what did we accept" and
  "what did we emit" the same queryable surface.

### Integration points

- **Engine:** `src/validation.rs::validate()` reused unchanged.
- **New module:** `src/egress.rs` — the `POST /egress/{contract}`
  handler, disposition logic, response shaping.
- **Routing:** new protected route `POST /egress/{contract}` alongside
  the existing `/ingest/{raw_id}` route in `src/main.rs`.
- **Storage:** `src/storage.rs` audit/quarantine inserts take a
  `direction` field; ingest call sites pass `'ingress'` explicitly.
- **CLI:** optional `cargo run -- validate-egress <file>` for local
  contract testing, mirroring the ingest CLI.

## Out of Scope (this RFC)

- Egress PII / leakage transforms — RFC-030.
- Per-provider data-quality scorecard / drift reporting — RFC-031.
- Contract sharing between provider and consumer — RFC-032.
- Streaming egress (Kafka/Kinesis outbound). HTTP egress first; the
  stream consumers can adopt the same `src/egress.rs` core later.

## Open Questions

1. **Contract format.** The semantic contract format is locked. Does
   egress need an optional `egress:` block (default disposition,
   default mode) co-located in the contract, or does disposition stay a
   call-time parameter only? Recommendation: call-time parameter in v1,
   keep the locked format untouched; revisit an `egress:` block if a
   real use case needs per-contract defaults.
2. **Endpoint vs middleware.** Ship `POST /egress/{contract}` first, or
   ship a middleware/library form at the same time? Recommendation:
   endpoint first — middleware is a thin wrapper over the same handler
   and can follow.
3. **Default mode.** Partial-response (`block`) or atomic (`fail`) as
   the default when the caller does not specify? Recommendation:
   `block` — graceful degradation matches the "one bad row, not a whole
   500" goal.
4. **Latency budget.** Does egress validation hold the same `<15ms p99`
   bar as ingest? Recommendation: yes — it is the same engine; the bar
   is the engine's, not the direction's.
5. **Audit volume.** Egress validation could roughly double `audit_log`
   write volume. Do egress passes get a sampled / summary-only audit
   mode? Recommendation: full audit in v1, revisit with RFC-031 data.

## Acceptance Criteria

- [ ] Migration `017_egress_validation.sql` adds `direction` to
      `audit_log` and `quarantine_events`, defaulting to `ingress`
- [ ] `POST /egress/{contract}` validates an outbound payload and
      returns the cleaned payload plus per-record outcomes
- [ ] `block`, `fail`, and `tag` dispositions all behave per the table
      above
- [ ] Egress writes are tagged `direction = 'egress'`; all existing
      ingest writes are tagged `direction = 'ingress'`
- [ ] Egress validation calls `validate()` with no duplicated rule
      logic
- [ ] `<15ms p99` validation budget preserved (bench unchanged)
- [ ] `cargo test` / `cargo check` pass; no existing ingest behavior
      changed
- [ ] `docs/egress-validation-reference.md` added — new user-facing
      endpoint and disposition options

---

## Dependency Chain

RFC-029 is the foundation of the egress series. RFC-030 (egress PII
guard) and RFC-031 (provider scorecard) both build on the
`direction = 'egress'` data this RFC introduces.
