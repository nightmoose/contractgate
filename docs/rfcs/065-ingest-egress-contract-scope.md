# RFC-065 — Enforce per-key `allowed_contract_ids` on ingest & egress hot paths

**Status:** Accepted
**Date:** 2026-05-28
**Branch:** nightly-maintenance-2026-05-28-rfc065-ingest-egress-scope
**Addresses:** docs/reviews/sale-readiness-review-2026-05-28.md (new high-severity finding)
**Severity:** P0 — cross-tenant authorization gap

---

## Problem

RFC-047/048 closed Broken-Object-Level-Authorization on the management
(by-ID) routes. The **ingest and egress hot paths were not covered** and still
have the gap.

`v1_ingest.rs` enforces per-key contract scoping:

```rust
if let Some(ref allowed) = k.allowed_contract_ids {
    if !allowed.contains(&contract_id) {
        return Err(AppError::Unauthorized);
    }
}
```

But the current `ingest.rs` and `egress.rs` handlers carry only a **comment**
claiming the path is "scoped by key.allowed_contract_ids" — there is no
enforcement code:

| Handler | File:line | `allowed_contract_ids` checked? |
|---|---|---|
| `ingest_handler` | `src/ingest.rs:240` | No — comment only |
| `egress_handler` | `src/egress.rs:374` | No — comment only |
| `v1_ingest` | `src/v1_ingest.rs:489` | Yes |

**Impact:** a holder of any valid API key whose `allowed_contract_ids` is a
restricted subset can still POST events to (`/ingest`) or read validated data
from (`/egress`) **any** `contract_id` in the system by enumerating UUIDs —
across org boundaries. This is the same BOLA class RFC-047 fixed, on the two
highest-volume surfaces. A buyer's diligence pen-test will hit this first.

## Non-goals

Retiring/strictly scoping the legacy env-var `API_KEY` master key (which is
issued with `allowed_contract_ids: None`, i.e. unrestricted) is a behavior
change requiring its own decision. Tracked as a follow-up, not in this RFC.

## Fix

Add one testable helper on `ValidatedKey` and call it from all three hot paths,
removing the duplicated logic and the false comments.

```rust
impl ValidatedKey {
    /// True if this key may act on `contract_id`.
    /// A key with `allowed_contract_ids == None` is unrestricted
    /// (e.g. JWT-derived keys and the legacy master key — see Non-goals).
    pub fn permits_contract(&self, contract_id: Uuid) -> bool {
        match &self.allowed_contract_ids {
            Some(allowed) => allowed.contains(&contract_id),
            None => true,
        }
    }
}
```

Call site (ingest, egress, v1_ingest):

```rust
if let Some(Extension(ref k)) = key_ext {
    if !k.permits_contract(contract_id) {
        return Err(AppError::Unauthorized);
    }
}
```

`key_ext` is consumed by the existing `org_id` resolution in `ingest.rs` /
`egress.rs`; that line changes to `key_ext.as_ref().map(...)` so the extension
remains available for the scope check.

Enforcement runs **before** loading contract identity, so a wrong-scope key
gets `401 Unauthorized` and never observes whether the contract exists.

## Testing

- Unit tests for `permits_contract`: unrestricted (`None`), allowed match,
  denied (UUID not in list).
- Existing `v1_ingest` behavior preserved (refactored to the shared helper).
- Full path validated via `cargo test` + `cargo check` (run by maintainer).

## Rollout

No migration, no config change, no API surface change. Keys already issued with
`allowed_contract_ids: None` are unaffected (unrestricted by design).
