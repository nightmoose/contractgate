# RFC-067 — Harden request-path panics (collaboration `expect`, replay `unwrap`)

**Status:** Implemented (2026-05-28)
**Date:** 2026-05-28
**Branch:** `nightly-maintenance-2026-05-28-rfc067-panic-hardening`
**Follows:** RFC-066 (legacy master-key removal)
**Addresses:** docs/reviews/cheap-findings-2026-05-28.md — Pattern 1, items #1 and the replay `unwrap` pair
**Severity:** P2 — latent denial-of-service (worker panic on an authenticated request)

> **Source of this work:** an external read-only "cheap findings" scan
> (`docs/reviews/cheap-findings-2026-05-28.md`). Of ~48 flagged items, only the
> request-path panics below were judged worth acting on now; the HMAC/UTF-8
> `expect`s in `transform.rs` and the `len() as i32` casts are provably safe
> under current input bounds and were intentionally left in place.

---

## Problem

Six call sites turn a violated runtime invariant into a **panic** instead of an
HTTP error. None is reachable today (each is guarded by a preceding check), but
each depends on an invariant enforced by *convention*, not by the type system —
so a future refactor can silently re-arm it. A panic on an Axum handler aborts
the worker task and returns a connection reset rather than a clean status code.

### Group A — `collaboration.rs`: `org_id.expect(...)` after `require_role`

Four mutation handlers resolve `org_id` (an `Option<Uuid>`), call
`require_role(...).await?`, then immediately `org_id.expect("… check guarantees
org_id is set")`. The assumption is that a successful `require_role` implies
`org_id.is_some()`. That holds now, but it is a cross-function coupling between
`require_role` and the handler — and RFC-066 just reworked the `dev_no_auth` /
`org_id_from_req` path that feeds it. A change there that let `require_role`
succeed with `org_id == None` would convert an authenticated request into a
worker panic.

| Handler | Site |
|---|---|
| `grant_collaborator_handler` (POST `/contracts/{name}/collaborators`) | `:186` |
| `add_comment_handler` (POST `/contracts/{name}/comments`) | `:262` |
| `create_proposal_handler` (POST `/contracts/{name}/proposals`) | `:329` |
| `decide_proposal_handler` (POST `…/proposals/{id}/decide`) | `:356` |

### Group B — `replay.rs`: `ordinal_for_id.get(&source_id).unwrap()`

In `replay_handler`'s outcome-folding loop (`:419`, `:436`), every `pending`
item's `source_id` is looked up in `ordinal_for_id` to find its response slot.
The map is built from the same id set `pending` derives from, so the key is
always present today. But the very next statement that consumes a slot
(`slot[idx].take().unwrap_or(ReplayItemOutcome::NotFound)`, `:449`) already
tolerates a missing entry gracefully — so a hard `unwrap()` here is both
inconsistent and an unnecessary panic surface if the map-construction invariant
ever drifts.

## Fix

**Group A** — replace each `org_id.expect(...)` with error propagation:

```rust
let caller_org = org_id.ok_or(AppError::Unauthorized)?;
```

A missing org now returns **401**, the same as any other unresolved-identity
case, instead of panicking. Behaviour is unchanged for every request that
reaches these handlers today (org is always set), so no test changes are needed.

**Group B** — skip the item gracefully instead of unwrapping:

```rust
let Some(&idx) = ordinal_for_id.get(&source_id) else { continue };
```

A `source_id` with no slot is dropped from the response (its slot stays `None`
and is later materialised as `NotFound` at `:449`), matching the existing
tolerance at the slot-take site. Unreachable today; no behaviour change.

## Changes made

| File | Change |
|---|---|
| `src/collaboration.rs` | 4× `org_id.expect("…")` → `org_id.ok_or(AppError::Unauthorized)?` (lines 186, 262, 329, 356). |
| `src/replay.rs` | 2× `let idx = *ordinal_for_id.get(&source_id).unwrap();` → `let Some(&idx) = ordinal_for_id.get(&source_id) else { continue };` (lines 419, 436). |

## Rollout / migration

No API change, no DB migration, no config change. Pure internal hardening:
every code path observable to a client is identical to before for all inputs
that are reachable today. The only difference is the failure *mode* of a
should-be-impossible state: clean 401 / dropped-slot instead of a worker panic.

## Verification

`cargo check` + `cargo test`. The existing `collaboration` and `replay` unit
tests exercise the success paths unchanged. No new test is added because the
changed branches are unreachable under current invariants — adding a test would
require deliberately breaking `require_role` or the ordinal map, which is out of
scope. (Tracked alongside the broader test-hardening handoff,
docs/reviews/test-hardening-handoff-2026-05-28.md.)
