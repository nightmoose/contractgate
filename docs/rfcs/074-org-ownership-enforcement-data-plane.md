# RFC-074 — Enforce org-ownership on the data plane (P0 cross-tenant write)

**Status:** Implemented (2026-05-29)
**Date:** 2026-05-29
**Branch:** `nightly-maintenance-2026-05-28-rfc069-pure-fn-coverage`
**Severity:** **Latent P0-class bug (cross-tenant write, BOLA/IDOR) found by
code inspection.** Under production auth this would let any unrestricted key
write to another org's contract on ingest/egress/v1-ingest. **Not reproduced
under auth** — see "What is and isn't proven" below.
**Surfaced by:** triaging why RFC-073's `cross_org_ingest_is_rejected` returned
200 in the compose-smoke lane. The 200 itself turned out to be an
auth-disabled-stack artifact (see below), but tracing it led to the real
`None`-org_id bug in the handlers.

---

## Problem

`cross_org_ingest_is_rejected` POSTs org B's API key to org A's contract and
asserts the request is rejected (403/404). Against the compose stack it returned
**200**.

### What is and isn't proven

The compose stack runs with `CONTRACTGATE_DEV_NO_AUTH=1` (`docker-compose.yml`),
which makes `require_api_key` (`src/main.rs`) pass every request through
**without validating the key or attaching a `ValidatedKey`**. So in that stack
`org_id` is always `None` regardless of which key is sent — the 200 means "auth
is off," **not** "a cross-tenant breach was demonstrated." The cross-tenant
write was **not** reproduced end-to-end.

What *is* established, by reading the code:

```
 key_prefix  |                org_id                | allowed_contract_ids
 cg_live_orgA | aaaaaaaa-…-aaaaaaaaaaaa               | (null)
 cg_live_orgB | bbbbbbbb-…-bbbbbbbbbbbb               | (null)

 contract a0000000-…-000000000001 | org_id aaaaaaaa-…-aaaaaaaaaaaa
```

Two distinct orgs, both keys unrestricted. With auth **on**, the handlers
resolve `org_id` from the validated key but then discard it (pass `None` to the
storage lookup), so B's key would reach A's contract. That is the bug this RFC
fixes. It is a real, reachable code path in production — it is just not
exercised by the current (auth-disabled) smoke lane.

## Root cause

The data-plane handlers resolve the caller's `org_id` from the validated key,
then **throw it away** — every contract lookup passes `None` for the org filter:

| File | Handler | Line | Call |
|---|---|---|---|
| `src/ingest.rs` | `ingest_handler` | 251 | `get_contract_identity(.., None)` |
| `src/ingest.rs` | `ingest_handler` | 258 | `get_version(.., None)` |
| `src/egress.rs` | `egress_handler` | 382 | `get_contract_identity(.., None)` |
| `src/egress.rs` | `egress_handler` | 389 | `get_version(.., None)` |
| `src/v1_ingest.rs` | `v1_ingest_handler` | 485 | `get_contract_identity(.., None)` |
| `src/v1_ingest.rs` | `v1_ingest_handler` | 498 | `get_version(.., None)` |

The *only* per-request scope check on these paths is RFC-065's
`key.permits_contract(contract_id)`, which consults the key's
`allowed_contract_ids` allow-list. That list is `None` for **all JWT-derived
dashboard keys and the legacy master key** (per the doc comment on
`ValidatedKey::permits_contract`), and `None` → unrestricted → `true`. So the
common-case key passes the only gate and reaches a contract it does not own.

RFC-065 wired per-**key** contract scoping across the data plane but never
wired per-**org** scoping. The storage layer was already org-aware
(`get_contract_identity` / `get_version` both take `org_id` and filter
`($N IS NULL OR org_id = $N)`); the handlers simply never passed it.

## Fix

Thread the resolved `org_id` into the identity + version lookups on all three
handlers. Scoping the **identity load** is the gatekeeper: a cross-org request
404s there, before version resolution, compile, validation, or any write. The
`get_version` call is scoped too (the param already exists) for defense in
depth. No storage, schema, or signature change — only the argument passed.

| File | Change |
|---|---|
| `src/ingest.rs` | `get_contract_identity(.., org_id)`, `get_version(.., org_id)` in `ingest_handler`. |
| `src/egress.rs` | same two calls in `egress_handler`. |
| `src/v1_ingest.rs` | same two calls in `v1_ingest_handler`. |

A cross-org caller now gets `404 ContractNotFound` (we 404 rather than 403 so a
wrong-org key never learns the contract exists — consistent with the RFC-073
test's accepted `403 || 404`).

## Deliberately out of scope (tracked, not fixed here)

- **`get_latest_stable_version` / `resolve_version` take no `org_id`.** They are
  only reachable *after* the now-scoped identity check, so they cannot be used
  to cross orgs once this fix lands. Scoping them is defense-in-depth, not a
  live hole — deferred to keep tonight's change minimal.
- **`ingest_stats_handler` (`src/ingest.rs:912`) has no auth extension at all** —
  it takes only `State` + `Path`, calls `get_contract_identity(.., None)`, and
  returns ingestion stats for any contract id with no key required. This is a
  separate read-side exposure that needs an auth signature change to fix.
  **Flagged as a related P1 finding** — not addressed in this RFC.

## Verification (maintainer runs — no docker/cargo in agent env)

```bash
cargo test                      # confirm no existing test regressed
```

**The compose-smoke lane does not verify this fix.** With
`CONTRACTGATE_DEV_NO_AUTH=1`, `org_id` is `None` whether or not the fix is
present, so `cross_org_ingest_is_rejected` returns 200 either way — the test is
a false signal in that stack. To actually exercise org isolation the test must
run against a gateway instance with auth **on** (`DEV_NO_AUTH=0`) and real keys.
That test-lane fix is tracked separately (see RFC-073 follow-up); until it
lands, this fix is verified by code inspection + unit tests only, not by the
smoke lane.
