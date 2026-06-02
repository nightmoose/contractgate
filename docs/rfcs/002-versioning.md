# RFC-002: Contract Versioning

| Status        | Accepted (2026-04-18)                    |
|---------------|------------------------------------------|
| Author        | ContractGate team                        |
| Created       | 2026-04-18                               |
| Accepted      | 2026-04-18 — Alex sign-off on all three open questions |
| Target branch | `nightly-maintenance-2026-04-17`         |
| Tracking      | Post-demo feedback item #2               |
| Supersedes    | The nominal `contracts.version` column (single-value, unused by routing) |

## Summary

Promote `contracts.version` from a nominal text field into a first-class
versioning system with **strict, forward-only state transitions**
(`draft → stable → deprecated`), **full immutability** once a version leaves
`draft`, and **explicit per-request pinning** via the `X-Contract-Version`
header (with `@x.y.z` path suffix as a fallback).

**Multiple `stable` versions may coexist** — promoting a new stable does
*not* auto-deprecate the previous one. Unpinned traffic resolves to the
*latest* stable (by `promoted_at DESC`) by default (`multi_stable_resolution
= strict`). Contracts can opt into `fallback` mode per-contract: if the
latest-stable rejects the event, the validator then tries the other
`stable` versions in `promoted_at DESC` order and the first one that passes
wins — the exact version that matched is written to the audit row, so
"validated under" always reflects the contract that actually accepted the
event. `fallback` is off by default; strictness remains the product pitch.

Requests pinning a `deprecated` version have the **entire batch
quarantined** with a `deprecated_contract_version` violation — the pinned
version is recorded on the batch-rejected audit row so the chain of custody
is intact.

Contract identity (`name`, `description`) is mutable; every change writes
to `contract_name_history` via a Postgres trigger so a rename can be
reconstructed from audit data alone.

This is the prerequisite for RFC-003 (manual Replay Quarantine): without
per-event version tagging in the audit log, replay has no way to know which
compiled contract to re-validate against.

## Goals

1. Multiple versions per `contract_id`, each identified by a **semver** string (`MAJOR.MINOR.PATCH`).
2. States: `draft`, `stable`, `deprecated` — with legal transitions only:
   - `draft → stable` (promote)
   - `stable → deprecated` (deprecate)
   - No other moves. Ever.
3. **Immutability:** `stable` and `deprecated` versions are frozen. YAML, semver, and metadata cannot be edited. `draft` is freely editable (iterate before promoting).
4. **Deletion:** only `draft` versions can be deleted. `stable` and `deprecated` versions are retained **forever** — required for audit integrity.
5. **Version resolution:** header pin → path suffix → latest `stable`. Unknown version → 404. Deprecated pin → quarantine entire batch.
6. **Multi-stable coexistence:** multiple versions may hold `stable` simultaneously. Promoting a new stable never auto-deprecates a prior one. Per-contract `multi_stable_resolution` flag chooses between `strict` (default — latest-stable only, fail-closed) and `fallback` (on failure, retry against other stables; first pass wins).
7. **Audit:** every `audit_log`, `quarantine_events`, and `forwarded_events` row records the **exact `contract_version` that accepted/rejected the event**. Under `fallback` mode this is the version that actually matched (not the default latest-stable), so "validated under" is always honest. Batch-rejected audit rows (under atomic mode or deprecated-pin quarantine) record the pinned version.
8. **Identity-level metadata is mutable** with full history. `contracts.name` / `contracts.description` can change; a Postgres trigger mirrors every change into `contract_name_history` keyed by `contract_id`.
9. No tenant/org scoping (per-contract only, matching existing model).
10. No compatibility enforcement on publish (deferred — informational only).

## Non-goals

- **Compatibility checking on promote.** We do not compare old-stable vs. new-stable YAMLs to verify MAJOR/MINOR/PATCH bumps make sense. That's a future enhancement; semver is purely a human-readable string today.
- **Automatic version bumps.** Publishing a new version always requires an explicit semver from the client. No auto-increment.
- **Schema diffing / migration tooling.** Out of scope. If a v2 breaks v1 consumers, that's an operational problem.
- **Tenant/org-level version policies.** Deferred — per-contract is the only scoping today.
- **Auto-retry / replay.** RFC-003. This RFC makes the *data* replay needs available; the replay button itself is the next RFC.
- **Per-client "sticky" versioning.** Clients pin explicitly or get latest-stable. No "last used by client X" memory.

## Current state (as of 2026-04-18)

`supabase/migrations/001_initial_schema.sql`:
- `contracts` table has `version TEXT NOT NULL DEFAULT '1.0'` and `active BOOLEAN` — but neither is referenced by the ingest path. Effectively unused.
- `audit_log` has `contract_id` but no version column.
- `quarantine_events` (migration 002) likewise has no version column.

`src/ingest.rs`:
- Handler loads `CompiledContract` by `contract_id` from the in-memory cache in `AppState`. No version awareness.

`src/main.rs`:
- `AppState.contract_cache: Arc<RwLock<HashMap<Uuid, Arc<CompiledContract>>>>`.

`dashboard/`:
- Contracts page shows a single YAML per contract. No version UI.

Alex confirmed (2026-04-18) that **wiping existing data is fine** — this is still a dev/test environment, so the migration does not need to backfill.

## Design

### 1. Schema — split contracts into identity + versions

New migration `003_contract_versioning.sql`:

```sql
-- Dev/test env: wipe dependent data so we can reshape cleanly.
TRUNCATE audit_log, quarantine_events, forwarded_events, contracts CASCADE;

-- Drop the now-obsolete single-version fields.
ALTER TABLE contracts DROP COLUMN version;
ALTER TABLE contracts DROP COLUMN active;
ALTER TABLE contracts DROP COLUMN yaml_content;

-- Identity-level resolution policy: strict (latest-stable only) or
-- fallback (try other stables on failure, first pass wins).  See §2b.
ALTER TABLE contracts
    ADD COLUMN multi_stable_resolution TEXT NOT NULL DEFAULT 'strict'
        CHECK (multi_stable_resolution IN ('strict', 'fallback'));

-- Optional human-readable metadata.  Mutable; changes mirrored to
-- contract_name_history below.
ALTER TABLE contracts
    ADD COLUMN description TEXT;

-- New table: one row per (contract, version) pair.
CREATE TABLE contract_versions (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id    UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    version        TEXT NOT NULL,       -- semver: "1.0.0", "2.1.3"
    state          TEXT NOT NULL
                   CHECK (state IN ('draft', 'stable', 'deprecated')),
    yaml_content   TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    promoted_at    TIMESTAMPTZ,          -- draft → stable timestamp
    deprecated_at  TIMESTAMPTZ,          -- stable → deprecated timestamp
    UNIQUE (contract_id, version)
);

CREATE INDEX idx_cv_contract_state
    ON contract_versions (contract_id, state);

-- Partial index: "latest stable per contract" is the hot path.
CREATE INDEX idx_cv_latest_stable
    ON contract_versions (contract_id, promoted_at DESC)
    WHERE state = 'stable';

-- Immutability guard: once state leaves 'draft', row is frozen.
CREATE OR REPLACE FUNCTION contract_versions_immutability_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.state IN ('stable', 'deprecated') THEN
        -- Only legal change is state: stable → deprecated.
        IF NEW.yaml_content   IS DISTINCT FROM OLD.yaml_content
        OR NEW.version        IS DISTINCT FROM OLD.version
        OR NEW.contract_id    IS DISTINCT FROM OLD.contract_id
        OR NEW.created_at     IS DISTINCT FROM OLD.created_at
        OR (OLD.state = 'stable'     AND NEW.state NOT IN ('stable','deprecated'))
        OR (OLD.state = 'deprecated' AND NEW.state <> 'deprecated')
        THEN
            RAISE EXCEPTION 'contract_versions is frozen once state leaves draft (id=%)', OLD.id;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contract_versions_frozen
    BEFORE UPDATE ON contract_versions
    FOR EACH ROW EXECUTE FUNCTION contract_versions_immutability_guard();

-- Deletion guard: drafts only.
CREATE OR REPLACE FUNCTION contract_versions_delete_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.state <> 'draft' THEN
        RAISE EXCEPTION 'only draft versions may be deleted (id=%, state=%)',
            OLD.id, OLD.state;
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contract_versions_deletable_draft_only
    BEFORE DELETE ON contract_versions
    FOR EACH ROW EXECUTE FUNCTION contract_versions_delete_guard();

-- Audit trail — every row now identifies the exact version used.
-- Under fallback mode this is the version that ACCEPTED the event, not the
-- default latest-stable.  "validated under" always reflects the actual
-- contract used.
ALTER TABLE audit_log         ADD COLUMN contract_version TEXT;
ALTER TABLE quarantine_events ADD COLUMN contract_version TEXT;
ALTER TABLE forwarded_events  ADD COLUMN contract_version TEXT;

CREATE INDEX idx_audit_contract_version
    ON audit_log (contract_id, contract_version);

-- Name-change history — small append-only table so a rename can be
-- reconstructed from audit data alone.  Intentionally tiny: (id, old_name,
-- new_name, changed_at).  Populated automatically by the trigger below.
CREATE TABLE contract_name_history (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id  UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    old_name     TEXT NOT NULL,
    new_name     TEXT NOT NULL,
    changed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cnh_contract_time
    ON contract_name_history (contract_id, changed_at DESC);

CREATE OR REPLACE FUNCTION contracts_name_history_trigger()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.name IS DISTINCT FROM OLD.name THEN
        INSERT INTO contract_name_history (contract_id, old_name, new_name)
        VALUES (OLD.id, OLD.name, NEW.name);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER contracts_record_rename
    AFTER UPDATE OF name ON contracts
    FOR EACH ROW EXECUTE FUNCTION contracts_name_history_trigger();

-- Refresh the summary view to group by version.
DROP VIEW v_ingestion_summary;
CREATE VIEW v_ingestion_summary AS
SELECT
    c.id   AS contract_id,
    c.name AS contract_name,
    a.contract_version,
    COUNT(a.id) AS total_events,
    SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)    AS passed_events,
    SUM(CASE WHEN NOT a.passed THEN 1 ELSE 0 END) AS failed_events,
    ROUND(
        SUM(CASE WHEN a.passed THEN 1 ELSE 0 END)::NUMERIC
        / NULLIF(COUNT(a.id), 0) * 100, 2
    ) AS pass_rate_pct,
    AVG(a.validation_us)::BIGINT AS avg_validation_us,
    MAX(a.created_at)            AS last_event_at
FROM contracts c
LEFT JOIN audit_log a ON a.contract_id = c.id
GROUP BY c.id, c.name, a.contract_version;
```

The two Postgres triggers (`immutability_guard`, `delete_guard`) enforce the
state machine at the storage layer — so even a SQL editor session can't
accidentally corrupt history. Application-layer checks are still the primary
defense; the triggers are belt-and-braces.

### 2. Version resolution order (ingest path)

```
POST /ingest/{contract_id}
Headers:
  X-Contract-Version: 1.2.3      (optional explicit pin)
Path fallback:
  POST /ingest/{contract_id}@1.2.3   (for clients that can't set headers)
```

Resolution:

| Input                      | Behaviour                                       |
|----------------------------|-------------------------------------------------|
| Header pin `1.2.3`         | Exact lookup on `(contract_id, '1.2.3')`.       |
| Path suffix `@1.2.3`       | Same. Header wins if both present (warn once). |
| Neither                    | Latest `stable` per `promoted_at DESC`.         |
| Pin → version not found    | **400** `VersionNotFound`.                      |
| Pin → version is `draft`   | Validate against it. Drafts are explicitly opt-in — this is how you QA a new version before promoting. |
| Pin → version is `stable`  | Validate against it.                            |
| Pin → version is `deprecated` | **Quarantine entire batch** (see §5).        |
| No stable exists, no pin   | **409** `NoStableVersion` — "pin a version explicitly or publish a stable one first". |

### 2b. Multi-stable resolution modes

Contracts may hold multiple `stable` versions at once. Promoting a new
stable never auto-deprecates the prior one; deprecation is an explicit
decision. The per-contract `multi_stable_resolution` flag chooses how
unpinned traffic resolves:

**`strict` (default).** Unpinned requests validate against the *single*
latest-stable (`promoted_at DESC`). If validation fails, the event is
quarantined as usual — with `contract_version` set to that latest-stable.
No retries against other versions. This preserves the product pitch:
"validated against the contract" means *the* contract, not "whichever one
said yes."

**`fallback` (opt-in).** Unpinned requests validate against latest-stable
first. On failure, the validator retries against the remaining `stable`
versions in `promoted_at DESC` order. The **first** version that accepts
the event wins, and that version is what goes into the `contract_version`
column on the audit row — so the audit trail honestly reflects the
contract that accepted the event, never the default that rejected it.

Key properties of `fallback`:

- **Only `stable` versions** are candidates. Drafts and deprecated
  versions are never tried — deprecation always means "no new traffic",
  fallback or not.
- **Retries run in parallel via rayon.** For N stables, the worst case is
  one parallel fan-out of N validations, not N sequential passes. At small
  N (< 10 typical) this is cheap; the CPU cost is bounded.
- **Quarantine still fires if nothing matches.** If no stable accepts the
  event, the audit row records the *latest-stable* as `contract_version`
  (the default the request was resolved to) and violation_details lists
  which versions were tried and each set of violations keyed by version,
  so debugging is possible. This keeps the "one event, one audit row"
  invariant intact.
- **Pinned requests are unaffected.** A request with an explicit
  `X-Contract-Version` header always resolves to that one version — no
  fallback — so clients that want deterministic routing can always get it.
- **Compliance mode (RFC-004) will reject `fallback` as incompatible.**
  Compliance mode needs a single source of truth for "what fields are
  declared." Mixing modes would make "declared" the superset-of-stables,
  which is the weakest possible enforcement. We'll add a CHECK /
  application-level guard when RFC-004 lands: `compliance_mode=true`
  implies `multi_stable_resolution=strict`.

### 3. In-memory cache key change

```rust
// Before (current)
Arc<RwLock<HashMap<Uuid, Arc<CompiledContract>>>>

// After
struct ContractCache {
    // Exact (contract, version) → compiled form.
    by_version: RwLock<HashMap<(Uuid, String), Arc<CompiledContract>>>,
    // contract_id → latest stable version string (for the unpinned path).
    latest_stable: RwLock<HashMap<Uuid, String>>,
}
```

Both maps are loaded on boot and invalidated on any write to
`contract_versions`. Writes are rare; reads are on the hot path. `RwLock` is
fine — no need for a lock-free structure yet.

### 4. State machine API

```
POST   /contracts                               Body: { name, description?, yaml_content,
                                                        multi_stable_resolution? }
                                                Creates the contract identity AND a v1.0.0 draft
                                                from yaml_content in a single transaction — a
                                                brand-new contract should be useable in one call.
                                                Defaults: multi_stable_resolution = "strict".

PATCH  /contracts/{id}                          Body: { name?, description?,
                                                        multi_stable_resolution? }
                                                Identity-level metadata only.  Name changes are
                                                mirrored to contract_name_history (§1, trigger).
                                                Does NOT touch yaml_content — that lives on
                                                versions.

POST   /contracts/{id}/versions                 Body: { version, yaml_content }
                                                Creates a new draft.  Fails 409 if
                                                (contract_id, version) already exists.

PATCH  /contracts/{id}/versions/{version}       Body: { yaml_content }
                                                Edit YAML.  Allowed ONLY if state=draft.

POST   /contracts/{id}/versions/{version}/promote
                                                draft → stable.  Sets promoted_at.
                                                Does NOT auto-deprecate the prior stable —
                                                multi-stable coexistence is a feature (§2b).

POST   /contracts/{id}/versions/{version}/deprecate
                                                stable → deprecated.  Sets deprecated_at.

DELETE /contracts/{id}/versions/{version}       Only if state=draft.

GET    /contracts/{id}/versions                 List (id, version, state, timestamps) ordered by created_at.
GET    /contracts/{id}/versions/{version}       Full version incl. yaml_content.
GET    /contracts/{id}/versions/latest-stable   Convenience lookup for dashboard UI.
GET    /contracts/{id}/name-history             List of prior names with timestamps.
```

All transition endpoints reject illegal moves with **409 Conflict** and a
clear message: `"cannot deprecate a draft — promote it to stable first"`.

### 5. Deprecated-pin batch quarantine

When the resolved version has `state = 'deprecated'`:

1. Validation does not run. There is nothing to debug per-event — the pin itself is the failure.
2. All input events are written to `quarantine_events` with:
   - `contract_version` = the pinned (deprecated) version.
   - A synthetic violation: `{ kind: "deprecated_contract_version", pinned_version, latest_stable }`.
3. A single `audit_log` row is written with `passed=false`, `contract_version=<pinned>`, `violation_details=[{kind:"deprecated_contract_version", batch_size, pinned_version, latest_stable}]`.
4. Response: **422** with a clear error (not 207 — this is a policy rejection, not a partial success).

This preserves the RFC-001 "batch-rejected audit row under atomic mode"
pattern and extends it to the versioning domain. The pinned version being
on the audit row means replay/debug tooling later can answer "why was this
batch rejected?" without guessing.

Per Alex (2026-04-18 Q#4): the pinned version is recorded precisely because
that is the version that drove the quarantine decision. Recording `null`
would erase the "why".

### 6. Per-event audit tagging

Every successful or failing per-event audit row now writes
`contract_version` alongside `contract_id`. This is what RFC-003 (replay)
will key off: "replay quarantine rows for contract X that were originally
validated against version 1.2.3, using the current latest-stable instead."

The `AuditEntryInsert` and `QuarantineEventInsert` structs from RFC-001
grow one field each. The batch-insert SQL (`UNNEST(...)`) grows one typed
array parameter. No structural rewrite.

### 7. Compiled contract loading

On boot, `AppState::new` loads every non-draft version into the cache.
Drafts are lazily loaded on pin — they're a rarer path and keeping the cache
hot with drafts pollutes memory during heavy iteration.

```rust
// Eager load: every stable + deprecated version
SELECT contract_id, version, yaml_content
  FROM contract_versions
 WHERE state IN ('stable', 'deprecated');

// Lazy load: drafts fetched on first pin, cached with a short TTL
// (since they're mutable, we don't want to serve a stale compile).
```

Draft TTL: 10 seconds. Enough to amortize repeated requests during manual
QA, short enough that an edit is reflected almost immediately.

### 8. Error model

New `AppError` variants:

```rust
VersionNotFound { contract_id: Uuid, version: String }     // → 400
NoStableVersion { contract_id: Uuid }                       // → 409
InvalidStateTransition { from: String, to: String, version: String } // → 409
ImmutableVersion { version: String, state: String }         // → 409
DeprecatedVersionPinned { contract_id: Uuid, version: String, latest_stable: Option<String> } // → 422 (special: triggers quarantine path)
```

The `DeprecatedVersionPinned` case is unusual — it's not really an error,
it's a quarantine decision. In the handler it's pattern-matched before
being converted to an HTTP response so we can run the quarantine-write
path first. Everywhere else, it maps to 422.

### 9. Dashboard changes

Minimum surface area to keep this RFC tight:

- Contracts list page: show "latest stable" badge + version count per contract.
- New "Versions" tab on the contract detail page:
  - Table of versions with state badges (draft / stable / deprecated).
  - Promote / Deprecate / Delete (drafts only) buttons with confirm dialogs.
  - "New draft version" button — opens a YAML editor pre-filled with the current latest-stable YAML.
- Playground: add a "version" dropdown next to the contract dropdown. Defaults to "latest stable".

No RLS changes — existing per-role policies carry over since the new table
is scoped the same way.

### 10. Metrics / tracing

Add to each ingest span:

- `contract_version` — resolved version, always populated.
- `version_pin_source` — `"header" | "path" | "default_stable"`.
- `deprecated_pin_rejected` — bool.

Useful for spotting clients that are still pinning a deprecated version
long after it was deprecated.

## Test plan

Unit tests (`src/tests.rs → mod versioning`):

1. `create_version_on_existing_contract_default_draft`
2. `duplicate_version_on_same_contract_rejected`
3. `promote_draft_to_stable`
4. `promote_already_stable_rejected`
5. `promote_deprecated_rejected`
6. `deprecate_stable_to_deprecated`
7. `deprecate_draft_rejected`
8. `deprecate_already_deprecated_rejected`
9. `edit_yaml_on_draft_ok`
10. `edit_yaml_on_stable_rejected_409`
11. `edit_yaml_on_deprecated_rejected_409`
12. `delete_draft_ok`
13. `delete_stable_rejected_409`
14. `delete_deprecated_rejected_409`
15. `ingest_unpinned_uses_latest_stable`
16. `ingest_header_pin_exact_match`
17. `ingest_path_suffix_pin_exact_match`
18. `ingest_header_wins_over_path_if_both`
19. `ingest_pinned_draft_ok`
20. `ingest_pinned_deprecated_quarantines_entire_batch`
21. `ingest_pinned_unknown_version_returns_400`
22. `ingest_no_stable_and_no_pin_returns_409`
23. `audit_row_records_exact_version_used`
24. `deprecated_pin_audit_records_pinned_version_not_null`
25. `postgres_trigger_blocks_update_to_stable_yaml` *(integration test against real PG; guarded by an env flag so it doesn't run in default `cargo test`)*
26. `promote_does_not_auto_deprecate_prior_stable` — promoting v1.1.0 leaves v1.0.0 stable.
27. `strict_mode_unpinned_failure_quarantines_without_retry` — contract with `multi_stable_resolution=strict` and two stables: event that fails latest-stable is quarantined with latest-stable recorded, other stable is *not* tried.
28. `fallback_mode_unpinned_failure_retries_other_stables` — contract with `multi_stable_resolution=fallback` and two stables: event that fails latest-stable but passes v1.0.0 is accepted, audit records v1.0.0.
29. `fallback_mode_all_stables_fail_quarantines_with_latest_stable` — audit row records default latest-stable; violation_details lists per-version attempts.
30. `fallback_mode_deprecated_not_tried` — deprecated version is never a fallback candidate, even if its shape would accept the event.
31. `fallback_mode_draft_not_tried` — same, for drafts.
32. `fallback_respects_pinned_request` — pinned request under fallback contract still validates against only the pinned version.
33. `post_contracts_auto_creates_v1_0_0_draft` — single-call create flow, version exists with state=draft afterward.
34. `patch_contract_name_triggers_history_row` — name change writes to `contract_name_history`; unchanged name does not.
35. `patch_contract_description_does_not_trigger_history` — trigger is scoped to name only.

Integration smoke:

- Create contract → add draft v1.0.0 → promote → ingest (200) → add draft v1.1.0 → promote (v1.0.0 auto-deprecate? see Open Q #1) → ingest header-pin v1.0.0 → expect 422 quarantine.
- Hammer the ingest path with 50/50 unpinned and header-pinned requests across 3 versions; verify cache is hot for all and no N+1 queries fire.

## Rollout

Single PR to `nightly-maintenance-2026-04-17`:

1. Migration `003_contract_versioning.sql` — destructive (truncates dev data), adds `contract_versions`, `contract_name_history`, `multi_stable_resolution` column on `contracts`, `contract_version` columns on audit/quarantine/forwarded_events, and the two immutability + name-history triggers.
2. Rust: new `storage::contract_versions` module, ingest handler rewrite for resolution + optional fallback fan-out, state-transition endpoints, identity-metadata PATCH endpoint, audit writes tagged with the version that actually validated.
3. Dashboard: versions tab with promote/deprecate/delete controls, playground version dropdown, rename-with-history UI on the contract detail page.
4. `MAINTENANCE_LOG.md` entry.
5. No compat shim needed — dev/test env.

Breaking API changes for anyone hitting the existing `/contracts/{id}` PATCH with yaml_content:

- The payload shape changes (yaml_content now lives on a version, not the contract). The dashboard is the only known caller, and it'll be updated in-PR.

## Decisions (2026-04-18 Alex sign-off)

1. **Multiple stables allowed — no auto-deprecate on promote.** Contracts
   with genuinely distinct valid shapes should be able to hold multiple
   stable versions concurrently. Promoting v1.1.0 leaves v1.0.0 in
   `stable`; deprecation is always an explicit decision.

2. **Per-contract `multi_stable_resolution` flag.** Alex's initial
   thought was "on unpinned failure, try all stables." On reflection
   (with pushback from this author), he endorsed making it a **per-contract
   opt-in** rather than global default. Rationale:

   - The strictness pitch is the product moat. Default fallback weakens it.
   - Audit ambiguity under fallback is real — "validated under which
     version?" for unpinned traffic becomes "first one that passed,"
     which is non-deterministic-looking in an audit export. Making
     fallback an explicit per-contract policy (with a flag on the
     `contracts` row) means the permissiveness is itself an auditable
     decision, not emergent behaviour.
   - Deprecation keeps its meaning: even under fallback, deprecated
     versions are never tried, so deprecation still applies migration
     pressure.
   - Audit honesty: under fallback, the `contract_version` column always
     records the version that *actually accepted* the event — Alex's
     correction ("the contract used in the actual validation was stored
     with each record") is exactly what we implement.

3. **`POST /contracts` auto-creates `v1.0.0` draft.** A brand-new
   contract is usable after a single call; no two-step dance.

4. **Contract-level `name` and `description` are mutable.** `contracts.id`
   remains the audit anchor — renames don't invalidate any prior
   validation. Every change to `name` writes a row to
   `contract_name_history` via a trigger, so "prove someone saw X before
   it got renamed to Y" is answerable from the audit data alone.

5. **Pinned deprecated version → quarantine whole batch, record the
   pinned version** (carried over from RFC-002 initial draft and confirmed
   by Alex).

## Open questions

None. All design decisions are locked as of 2026-04-18.

## Dependencies / follow-ups

- **RFC-003 (Replay Quarantine)** — unblocks directly. Replay will filter
  `quarantine_events` on `contract_version` + `contract_id` and re-validate
  against the current latest-stable.
- **RFC-004 (PII masking)** — touches the same schema. We'll add the
  `compliance_mode` flag to `contracts` (identity-level, not per-version)
  when RFC-004 lands so policy isn't version-scoped.
- **Postgres `contract_versions` partitioning** — not needed until the
  table grows past ~millions of rows, which with "versions forever" will
  eventually happen. Track for later; not blocking.
