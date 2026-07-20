# RFC-086 — Gated event-payload storage (paid + opt-in)

**Status:** Implemented (backend + all ingest/egress/consumer paths + dashboard + docs) on branch; not yet shipped
**Date:** 2026-07-20
**Branch:** nightly-maintenance-2026-07-20-rfc086-gated-payload-storage
**Depends on:** RFC-004 (PII transforms), RFC-045 (plan gating), RFC-083 (metering / `get_org_plan_and_usage`), RFC-003/RFC-081 (quarantine + replay)

---

## Problem

We durably store customer event bodies for **every** ingested event, for all
orgs including Free, with no opt-out:

- `audit_log.raw_event` — the post-transform body of **every** event, pass or
  fail (`src/storage/audit.rs::AuditEntryInsert.raw_event`, insert at
  `log_audit_entries_batch`). This is the large surface: one row per event.
- `quarantine_events.payload` — the post-transform body of every **failed**
  event (three write sites in `src/ingest.rs`: normal, deprecated-pin, envelope).
  Smaller surface (failures only) but it is what replay re-validates.

Two problems with storing this unconditionally:

1. **Storage balloon on garbage data.** Free orgs can push up to 1M events/mo
   (RFC-083). Persisting every body — much of it malformed junk that failed
   validation — is unbounded cost we aren't paid for.
2. **Liability.** We hold copies of customer data at rest for accounts that pay
   us nothing. Even post-transform (PII-masked, RFC-004), it is still their
   data sitting in our database.

The bodies are only *functionally* required for one paid feature — **quarantine
replay** (already Growth+ gated, RFC-081/plan-gating-reference). Nothing on the
Free tier needs the stored body: the audit log, metrics, violation reporting,
and pilot report all work from metadata (contract, version, pass/fail,
`violation_details`, counts, `source_ip`, timing).

## Goal

Store event payloads **only** when the owning org is on a paid plan **and** has
explicitly opted in. Otherwise persist the metadata row with an empty body and a
`payload_redacted` marker — "we recorded that it failed and why, not the source
data."

Non-goals: changing forwarding (forward uses the in-memory transformed payload,
never storage — unaffected), changing validation semantics, or touching the
hot-path p99 budget (< 15 ms).

## Policy

Two levels. An **org-level** master switch (`orgs.store_event_payloads`,
default **false**) gates the whole org. A **per-contract** override
(`contracts.store_event_payloads`, default **true**) is only consulted when the
org switch is on, letting a paid org keep bodies on some pipelines and off on
others.

```
payloads_stored(org, contract) =
    org is None                          -> true    # self-host / dev: not plan-gated (RFC-045)
    plan not in {growth, enterprise}     -> false   # Free/unknown: never store; flags inert
    not org.store_event_payloads         -> false   # org master switch off
    not contract.store_event_payloads    -> false   # per-contract opt-out
    else                                 -> true
```

- **Free:** always metadata-only. Both flags are inert until they upgrade.
- **Paid, org switch off (default):** metadata-only across all contracts. Replay
  unavailable until enabled.
- **Paid, org switch on:** bodies stored for every contract *except* those whose
  per-contract override is off.
- **Self-hosted / dev (`org_id = None`):** unchanged — bodies stored. Self-host
  owns its own storage and data; plan gating never applied to it.

There are no existing paying customers and no data to grandfather (confirmed
2026-07-20), so default-false at the org level is safe — no silent breakage.

### Transitions and purge

Turning storage **off** does more than stop new writes; it removes what's already
stored. The scope differs by level:

- **Org switch on → off:** purge **all** stored bodies for the org (every
  contract). The org is choosing to hold no source data.
- **Per-contract override on → off:** **write-forward only** — stop writing new
  bodies for that contract; **no** historical purge. (A separate explicit action
  purges history — below.)
- **"Purge body history" (per-contract action):** available on any contract,
  independent of the toggles. Strips stored body values for that contract but
  retains every audit and quarantine **row** and all its metadata.

**Hard invariant: purge only ever nulls the body column. We never delete an
`audit_log` (or `quarantine_events`) row.** "Purge" = redact in place:

- `quarantine_events`: `payload = NULL`, `payload_redacted = true`
- `audit_log`: `raw_event = '{}'::jsonb`, `raw_event_redacted = true`

Everything else on the row — contract, version, `passed`, `violation_details`,
counts, `source_ip`, timing, replay linkage — is untouched. History stays; only
the source body leaves. After a purge, affected quarantine rows become
non-replayable (§3), same as any redacted row.

## Data model

Migration `034_gated_payload_storage.sql`:

```sql
-- Org-level master switch. Default off: nobody stores bodies until they choose to.
-- (Actual table is `orgs`, plan lives here as a text column — migration 007/026.)
ALTER TABLE orgs
  ADD COLUMN store_event_payloads BOOLEAN NOT NULL DEFAULT false;

-- Per-contract override, only consulted when the org switch is on.
-- Default true = "inherit the org switch"; set false to opt one pipeline out.
ALTER TABLE contracts
  ADD COLUMN store_event_payloads BOOLEAN NOT NULL DEFAULT true;

-- quarantine_events.payload is currently NOT NULL with no default.
-- Allow metadata-only rows (and purge-to-NULL).
ALTER TABLE quarantine_events
  ALTER COLUMN payload DROP NOT NULL;

-- Explicit "body intentionally not stored" marker, so the API and dashboard can
-- distinguish redacted-by-policy from a genuinely empty/`{}` payload. Audit
-- honesty: never let a NULL/`{}` be mistaken for real captured data.
ALTER TABLE quarantine_events
  ADD COLUMN payload_redacted BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE audit_log
  ADD COLUMN raw_event_redacted BOOLEAN NOT NULL DEFAULT false;
```

`audit_log.raw_event` already carries `NOT NULL DEFAULT '{}'::jsonb`
(001_initial_schema.sql:68), so redacted rows write `'{}'` + `raw_event_redacted
= true`; no nullability change needed there. Quarantine writes `NULL` +
`payload_redacted = true`.

## Backend changes

### 1. Read the policy once, on the path that already reads the plan

The ingest hot path already does a single plan+usage round-trip via
`storage::get_org_plan_and_usage` (RFC-083, called from
`metering::enforce_plan_limit`), and already fetches the contract row
(`get_contract_identity` / `get_version`, ingest.rs:256/264). Extend the org
query to also return `store_event_payloads`, and the contract fetch to carry its
`store_event_payloads`. Both are already on the path — **no new round-trip; p99
unchanged.** For `org_id = None`, resolve to stored.

Add `src/plan.rs::payloads_stored(plan, org_switch, contract_switch) -> bool`
(paid && org_switch && contract_switch) so the rule lives in one place and is
unit-tested alongside `monthly_event_limit`.

### 2. Redact at the build sites when `store_payloads == false`

At each `QuarantineEventInsert` construction (three sites in `ingest.rs`:
normal ~L601, `deprecated_pin_quarantine` ~L794, envelope ~L1104) and each
`AuditEntryInsert` construction:

- `store_payloads == true`  → unchanged (body = transformed payload).
- `store_payloads == false` →
  - quarantine: `payload = None`, `payload_redacted = true`
  - audit: `raw_event = TransformedPayload::from_stored(json!({}))`,
    `raw_event_redacted = true`

Everything else on the row (contract, version, `violation_details`,
`violation_count`, `source_ip`, timing, `passed`) is written exactly as today.

`QuarantineEventInsert.payload` becomes `Option<TransformedPayload>`; the batch
insert (`quarantine_events_batch`) binds `NULL` for `None`. `AuditEntryInsert`
gains `raw_event_redacted: bool`.

### 3. Replay must refuse redacted rows

`replay_for_contract` / the RFC-081 handlers read `quarantine_events.payload` to
re-validate. A redacted row has no body to replay. Add a `Redacted` (or reuse a
clear existing variant) `ReplayItemOutcome` so the API returns `passed: false`
with a distinct reason — **never** replay `{}`/NULL and emit bogus violations.
`GET /quarantine` surfaces `payload_redacted` so the dashboard can show "body not
stored — enable payload storage to replay."

### 4. Purge (redact-in-place, row-preserving)

One storage helper does all purging — org-wide, per-contract, or the explicit
button all call it with a different scope filter:

```
purge_bodies(pool, org_id, contract_id: Option<Uuid>):
  UPDATE quarantine_events SET payload = NULL, payload_redacted = true
    WHERE contract_id IN (<org's contracts, optionally narrowed to contract_id>)
      AND payload IS NOT NULL;
  UPDATE audit_log SET raw_event = '{}'::jsonb, raw_event_redacted = true
    WHERE org_id = $org (optionally AND contract_id = $contract)
      AND raw_event_redacted = false;
```

No `DELETE`. Rows and metadata are retained; only the body column changes.
Callers:

- Org switch on→off handler: `purge_bodies(org_id, None)`.
- Per-contract "Purge body history" button: `purge_bodies(org_id, Some(cid))`.
- Per-contract override on→off: **does not** call purge (write-forward only).

Migration 004's `quarantine_replay_stamp_guard` only protects
`replayed_at` / `replayed_into_audit_id`, so nulling `payload` is allowed. Confirm
no append-only trigger blocks the `audit_log` column update (audit rows are
retained, only the body field is redacted — this is intended, not a violation of
audit immutability, which is about the *decision record*, not the source body).

### 5. API surface

- `GET /quarantine` (`QuarantinedEventOut`): add `payload_redacted: bool`; when
  true, `raw_event` is `null`.
- Org settings read/write for `store_event_payloads` (owner/admin only); flipping
  to `false` triggers the org-wide purge. Enforce paid plan server-side
  (ignore / keep `false` when `free`).
- Per-contract: read/write `store_event_payloads` on the contract (only
  effective when org switch is on), plus a `POST /contracts/{id}/purge-bodies`
  action (owner/admin) that runs the per-contract purge.

## Frontend changes (follow-up push)

- Org Settings: a "Store event payloads" master toggle wrapped in
  `<PlanGate minTier="growth" feature="Event payload storage">`, copy explaining
  it's required for quarantine replay, bodies are stored post-transform, and that
  turning it **off purges all stored bodies** (confirm dialog).
- Per-contract (Contract settings, enabled only when the org switch is on): a
  "Store event payloads" toggle (write-forward), and a **"Purge body history"**
  button — available regardless of toggle state — that redacts stored bodies for
  that contract while keeping all audit history. Destructive-action confirm.
- Quarantine tab: for `payload_redacted` rows, show violations/metadata and a
  "Body not stored" state instead of a payload drawer + replay button.

## Docs

- New `docs/event-payload-storage-reference.md` (the toggle, the policy table,
  the replay dependency).
- Update `docs/quarantine-replay-reference.md` (replay requires a stored body;
  redacted rows are non-replayable) and `docs/plan-gating-reference.md` (add
  "Event payload storage / replay" to the Growth+ matrix — replay is already
  listed; add the storage prerequisite).

## Testing

- `plan.rs`: `payloads_stored` truth table (free/growth/enterprise × on/off,
  unknown → false).
- Ingest: free org → quarantine row has `payload IS NULL`, `payload_redacted`,
  and audit row `raw_event = '{}'` + `raw_event_redacted`; metadata
  (`violation_details`, counts) intact. Paid opted-in → bodies present. Paid
  opted-out → redacted. `org_id = None` → bodies present.
- Replay: redacted source row → distinct non-replay outcome, no bogus violations,
  source untouched.
- Purge: `purge_bodies` nulls bodies + sets redacted markers, **row counts
  unchanged** in both tables (assert no rows deleted); metadata columns intact;
  already-redacted rows are idempotent.
- Per-contract: org-on + contract-off → that contract redacts while a sibling
  contract still stores; contract on→off does not purge history.
- Wire-shape lock tests in `src/quarantine.rs` updated for the new field.

## Phasing

1. Migration `034` + `plan.rs::payloads_stored` + org+contract policy read
   threaded through ingest + redaction at build sites + replay guard +
   `GET /quarantine` field. (Backend; write-path behavior-complete.)
2. `purge_bodies` helper + org on→off purge + `POST /contracts/{id}/purge-bodies`
   + org/contract settings read/write endpoints. (Backend; purge + controls.)
3. Dashboard: org master toggle, per-contract toggle + "Purge body history"
   button, Quarantine tab redacted state. (Frontend.)
4. Docs.

## Resolved decisions (2026-07-20)

- **Granularity:** org-level master switch **and** per-contract override; the
  per-contract flag is only consulted when the org switch is on.
- **Retroactive purge:** org switch on→off purges all stored bodies org-wide.
  Per-contract override on→off is write-forward only (no purge). A dedicated
  per-contract "Purge body history" action redacts stored bodies on demand.
- **Audit rows are never deleted.** Purge nulls the body column only; every
  `audit_log` and `quarantine_events` row and all its metadata is retained.

## Implementation notes (as built, 2026-07-20)

Branch `nightly-maintenance-2026-07-20-rfc086-gated-payload-storage`. `cargo
check` clean; 312 `contractgate-server` unit tests pass.

- **Migration `034_gated_payload_storage.sql`** — `orgs.store_event_payloads`
  (default false), `contracts.store_event_payloads` (default true),
  `quarantine_events.payload` made nullable + `payload_redacted`,
  `audit_log.raw_event_redacted`.
- **Gate** — `plan::payloads_stored(plan, org_switch, contract_switch)`;
  `ingest::resolve_store_payloads` reads `storage::get_org_payload_policy` (one
  indexed PK read on `orgs`, off the validation loop) and the per-contract flag
  from the already-fetched `ContractIdentity`. `org_id = None` → store;
  missing-org / lookup error → redact (fail-closed).
- **Redaction is per-batch** — `log_audit_entries_batch` and
  `quarantine_events_batch` take a `store_payloads: bool`; an ingest batch is one
  contract/org, so the decision is uniform. Bodies written as `'{}'` / SQL NULL
  with the `*_redacted` marker set.
- **Paths gated:** v0 ingest (`ingest.rs` — normal, deprecated-pin, envelope),
  v1 bulk ingest (`v1_ingest.rs`), egress validation (`egress.rs`), and the
  Kafka/Kinesis consumers (via `storage::contract_stores_payloads`, one indexed
  join per poll). Replay re-quarantine still stores (it only ever runs on
  already-stored bodies).
- **Replay** — `ReplayItemOutcome::Redacted` for rows with no stored body;
  never re-validates `{}`.
- **Endpoints** (`settings.rs`): `GET/PUT /settings/payload-storage` (org switch;
  PUT-off purges org-wide; enabling requires a paid plan),
  `PATCH /contracts/{id}/payload-storage` (per-contract, write-forward),
  `POST /contracts/{id}/purge-bodies`.
- **Dashboard:** org master toggle on the Account page (behind plan
  eligibility; off → org-wide purge confirm); per-contract toggle + "Purge body
  history" button in the Quarantine tab (shown when one contract is selected);
  Quarantine tab renders a "Body not stored" state and disables replay for
  `payload_redacted` rows. `payload_redacted` added to
  `QuarantinedEvent`; `store_event_payloads` surfaced on `ContractSummary`.
  Backend `cargo check` + bin tests green; dashboard `tsc --noEmit` clean.

- **DB integration tests** (`src/tests.rs::rfc086_payload_storage`, `#[ignore]`,
  run with `cargo test rfc086 -- --ignored` against a Postgres migrated through
  034): redaction-on-insert (NULL/`{}` bodies + markers, metadata intact),
  purge row-preservation (bodies redacted, **row counts unchanged**, idempotent
  second purge), and the `contract_stores_payloads` gate (free never stores;
  paid needs org switch + per-contract override). Compile-verified here; execute
  against a live DB in CI.

### Still open (not in this push)

- Nothing functional. Remaining is operational: run the `#[ignore]` DB tests in
  CI against a migrated Postgres, and apply migration 034 to environments.
