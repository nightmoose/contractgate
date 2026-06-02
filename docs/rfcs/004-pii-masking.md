# RFC-004: PII Masking at Ingest

| Status        | Accepted (2026-04-19)                                                   |
|---------------|-------------------------------------------------------------------------|
| Author        | ContractGate team                                                       |
| Created       | 2026-04-19                                                              |
| Accepted      | 2026-04-19 — Alex sign-off on Q1–Q7 (all recommendations chosen)        |
| Target branch | `nightly-maintenance-2026-04-19`                                        |
| Tracking      | Post-demo feedback item #4                                              |
| Depends on    | RFC-002 (versioning) — landed; RFC-003 (replay) — landed                |

## Summary

Add **per-field PII transforms** to the semantic contract. At ingest,
after validation passes, declared fields are rewritten — `mask`, `hash`,
`drop`, or `redact` — before anything is written to `audit_log`,
`quarantine_events`, or forwarded downstream. Raw PII never leaves the
validator.

Also add a per-version **compliance mode** flag: when enabled, events
containing fields not declared in `ontology.entities` fail validation
through the normal per-event violation path.

This is the largest surface-area change in the post-demo queue: it
touches the contract schema, the validator output, the storage layer,
the forward path, and the dashboard's payload rendering. It's also the
marquee compliance story — the difference between "we validate your
data" and "we validate your data *and* guarantee raw PII never hits
durable storage."

## Goals

1. Contract authors can declare a `transform` on any entity with one of
   four kinds: `mask`, `hash`, `drop`, `redact`.
2. **Validation sees the raw value**; transforms run *after* validation
   succeeds, so existing pattern/enum/min/max rules still work the way
   authors intuitively expect.
3. **Raw PII never lands in durable storage.** Transforms run on the
   payload before it is written to `audit_log.payload`,
   `quarantine_events.payload`, OR forwarded to the contract's
   downstream destination — including on validation failure.
4. **Deterministic hashing for analytics joins.** `kind: hash` uses a
   per-contract secret salt (HMAC-SHA256) so the same input produces the
   same output forever on that contract, enabling joins on hashed keys
   downstream without ever exposing raw values.
5. **Compliance mode** (per-version flag): rejects events that contain
   any field name not declared in `ontology.entities`. Fails as a normal
   per-event violation — batch continues under per-item semantics,
   rolls back under `atomic=true`.
6. **Backwards compatible.** Contracts without a `transform` block OR
   `compliance_mode` flag behave exactly as they do today.
7. **Salt isolation.** The per-contract hash salt is generated at
   contract creation, stored in `contracts.pii_salt`, and is never
   exposed through any API, audit row, or dashboard surface.

## Non-goals

- **Format-preserving encryption (FF1/FF3).** v1's `format_preserving`
  mask is character-class-preserving only (digit→digit, letter→letter,
  shuffled deterministically with the contract salt), not a formal FPE
  scheme. An FPE upgrade is a future RFC if the compliance surface
  demands it.
- **Salt rotation.** Rotating the per-contract salt invalidates every
  prior hash, breaking all downstream joins. Deferred — will need its
  own design (probably "retire old contract, start new one").
- **Tokenization / reversible transforms.** No vault, no "detokenize"
  endpoint. If you want reversibility, hash + keep the source of truth
  upstream. This is deliberate — ContractGate is not a PII vault.
- **Regime-specific compliance modes.** No GDPR / HIPAA / SOC2 presets
  in v1. `compliance_mode: true` is a single boolean — undeclared
  fields are rejected. Regime-specific tooling (e.g., pre-built
  transform bundles for "PHI fields") can layer on later.
- **Transform stacking.** Each entity gets exactly one transform. No
  "mask then hash" pipelines. Multi-transform is hard to reason about
  for compliance audits and we don't have a use case.
- **Cross-field transforms.** Transforms are scoped to a single named
  entity. "If `user_type = employee`, mask `salary`" is not expressible.
  If we need it, it becomes a future RFC.

## Current state

- Contract schema (`src/contract.rs::OntologyEntity`) already has:
  `name`, `type`, `required`, `pattern`, `min`, `max`, `enum`. No
  `transform` field yet.
- Validator (`src/validation.rs`) returns a `ValidationOutcome` with
  `Passed { payload }` / `Failed { violations }`. The `payload` passed
  through on success is currently the **raw** request JSON.
- Storage writes (`src/storage.rs::log_audit_entry_batch`,
  `quarantine_events_batch`) take `payload: serde_json::Value`
  verbatim from the ingest handler.
- Forward path (`src/ingest.rs::forward_to_destination`) also sends
  the raw payload.
- No `compliance_mode` flag on `contract_versions`.
- No salt column on `contracts`.

What's missing (see **Rollout** below):
- Contract schema: `transform` on entity; `compliance_mode` on version.
- Migration `005_pii_transforms.sql`: `contracts.pii_salt`,
  `contract_versions.compliance_mode`.
- Transform engine: new module `src/transform.rs` that takes a
  `ValidatedPayload` + compiled contract and emits a `TransformedPayload`.
- Validator: surface undeclared-field check when `compliance_mode = true`.
- Audit + quarantine write path: accept `TransformedPayload`, not raw.
- Forward path: accept `TransformedPayload`.
- Contract creation: generate `pii_salt` server-side, never return it.
- Dashboard: render transformed payloads (they are what's stored), show
  a badge for fields that were masked/hashed.

## Design

### Contract schema (inline transforms)

New optional `transform` block on each ontology entity:

```yaml
version: "1.0"
name: "user_events"
description: "Contract for user interaction events"
compliance_mode: true    # new, default false

ontology:
  entities:
    - name: email
      type: string
      required: true
      pattern: "^[^@\\s]+@[^@\\s]+\\.[^@\\s]+$"
      transform:
        kind: hash            # deterministic HMAC-SHA256 with contract salt

    - name: phone
      type: string
      required: false
      transform:
        kind: mask
        style: format_preserving   # "+1 415-555-0199" -> "+1 839-204-7613"

    - name: ssn
      type: string
      required: false
      transform:
        kind: redact          # replaced with "<REDACTED>" sentinel

    - name: internal_debug_blob
      type: string
      required: false
      transform:
        kind: drop            # field removed from stored/forwarded payload

    - name: event_type
      type: string
      required: true
      enum: ["click", "view", "purchase", "login"]
      # no transform — passes through as-is
```

Semantics:
- `kind: mask`
  - `style: opaque` (default) → value replaced with `"****"` (a single
    fixed sentinel; length doesn't leak).
  - `style: format_preserving` → same length, same character class per
    position. Uses a deterministic shuffle keyed on `contract.pii_salt +
    entity_name + char_class_index`. Preserves digit/letter/symbol
    boundaries; does NOT preserve ordering or checksum validity. Never
    round-trippable.
- `kind: hash` → `"hmac-sha256:" + hex(HMAC-SHA256(contract.pii_salt, value))`.
  Output is a prefixed hex string so audit readers can spot it at a
  glance. Deterministic per contract — joins on hashed keys work forever.
- `kind: drop` → field is removed from the payload entirely before
  storage / forward. Not represented by any sentinel.
- `kind: redact` → value replaced with the sentinel string
  `"<REDACTED>"`. Field stays present (so schemas expecting it don't
  break).

Transform applies to string values only in v1. Transforming a non-string
value (number, boolean, object, array) is a **contract-compile error**
reported at contract save time — we reject the contract rather than let
the runtime surprise you. Numeric PII (e.g., account numbers) should be
typed as `string` in the contract.

### Compliance mode

Per-version flag: `compliance_mode: true|false` (default `false`). When
true, the validator treats *any* field in the inbound event whose name
is not in `ontology.entities` as a violation:

```json
{
  "code": "UNDECLARED_FIELD",
  "field": "tracking_pixel_id",
  "message": "Field 'tracking_pixel_id' is not declared in the contract ontology. Compliance mode rejects undeclared fields."
}
```

Failure semantics are identical to any other violation:
- In a per-item batch: the event is quarantined, others continue.
- In `atomic=true` batch: any UNDECLARED_FIELD in any event rolls back
  the whole batch (422).
- The original quarantine row stores the **transformed** payload, same
  as any other quarantine — so an undeclared field that happens to
  contain raw PII still doesn't land on disk as-is (the transform pass
  just has nothing to do for undeclared fields, which are dropped from
  the stored payload when `compliance_mode = true`; see "Validate-then-
  transform ordering" below).

### Validate-then-transform ordering (pipeline)

```
raw event JSON
     │
     ▼
┌────────────────────┐
│ 1. Validate        │  ← sees raw values. pattern/enum/min/max/required
│                    │    all work on original data. Compliance mode
│                    │    raises UNDECLARED_FIELD here.
└─────────┬──────────┘
          │
     pass │ fail
          │   │
          │   └──► 2b. Transform raw → transformed payload
          │       3b. Write transformed to quarantine_events.payload
          │       4b. Return per-item violation in response
          │
          ▼
 2a. Transform raw → transformed payload
 3a. Write transformed to audit_log.payload
 4a. Forward transformed payload to destination (if configured)
 5a. Return per-item `ok` in response
```

Key invariant: **storage writes and forward writes always receive the
post-transform payload, never the raw.** Pass or fail, declared or
undeclared, the raw JSON does not exit the in-memory validator.

Open question: should the HTTP response body echo the transformed
payload or the raw payload back to the caller? See **Open questions**
below.

### Per-contract salt

Migration adds:

```sql
ALTER TABLE contracts
    ADD COLUMN pii_salt BYTEA NOT NULL DEFAULT gen_random_bytes(32);
```

The `gen_random_bytes(32)` default means existing rows get salt on
migration (safe — no existing contract uses hash transforms yet).

The salt is:
- generated at contract creation (or by the DEFAULT on migration),
- loaded into the `CompiledContract` once at compile time,
- used only inside `transform.rs` for `kind: hash` and for seeding the
  `format_preserving` mask shuffle,
- never serialized to `audit_log`, API responses, or dashboard state,
- never re-generated — rotating it is a RFC-0XX problem, not this one.

Contract-read paths (`GET /contracts/:id`) explicitly strip `pii_salt`
in the response serializer. A unit test asserts this.

### Transform engine (`src/transform.rs`)

```rust
pub struct TransformedPayload(serde_json::Value);

pub fn apply_transforms(
    compiled: &CompiledContract,
    raw: serde_json::Value,
) -> TransformedPayload {
    let mut obj = raw.as_object().cloned().unwrap_or_default();

    for entity in &compiled.ontology_entities {
        let Some(t) = &entity.transform else { continue };
        let Some(val) = obj.get(&entity.name) else { continue };
        let Some(s) = val.as_str() else { continue };  // compile-time ensures string

        let replacement = match t.kind {
            TransformKind::Drop   => { obj.remove(&entity.name); continue; }
            TransformKind::Redact => json!("<REDACTED>"),
            TransformKind::Mask { style: MaskStyle::Opaque } => json!("****"),
            TransformKind::Mask { style: MaskStyle::FormatPreserving } => {
                json!(format_preserving_mask(s, &compiled.pii_salt, &entity.name))
            }
            TransformKind::Hash => json!(format!(
                "hmac-sha256:{}",
                hmac_sha256_hex(&compiled.pii_salt, s.as_bytes())
            )),
        };
        obj.insert(entity.name.clone(), replacement);
    }

    // Compliance mode: drop undeclared fields from the stored payload.
    // (The violation was already raised by the validator; this strips
    // the raw value so it can't leak through the audit/forward path.)
    if compiled.compliance_mode {
        let declared: HashSet<&str> = compiled.ontology_entities
            .iter().map(|e| e.name.as_str()).collect();
        obj.retain(|k, _| declared.contains(k.as_str()));
    }

    TransformedPayload(serde_json::Value::Object(obj))
}
```

The storage helpers are tightened to the new type so the compiler
prevents accidental raw writes:

```rust
pub async fn log_audit_entry_batch(
    db: &PgPool,
    entries: Vec<AuditEntryInsert>,  // payload: TransformedPayload
) -> Result<...> { ... }

pub async fn quarantine_events_batch(
    db: &PgPool,
    rows: Vec<QuarantineEventInsert>,  // payload: TransformedPayload
) -> Result<...> { ... }
```

### Handler flow changes (`src/ingest.rs`)

Current pass path (simplified):
```rust
for event in events {
    match validate(&compiled, &event) {
        Ok(())   => audit_inserts.push(AuditEntryInsert { payload: event, ... }),
        Err(v)   => quar_inserts.push(QuarantineEventInsert { payload: event, violations: v, ... }),
    }
}
```

New pass path:
```rust
for event in events {
    let outcome = validate(&compiled, &event);   // may raise UNDECLARED_FIELD under compliance_mode
    let transformed = apply_transforms(&compiled, event);
    match outcome {
        Ok(())   => audit_inserts.push(AuditEntryInsert { payload: transformed, ... }),
        Err(v)   => quar_inserts.push(QuarantineEventInsert { payload: transformed, violations: v, ... }),
    }
}
```

`validate` no longer consumes the event; `apply_transforms` takes
ownership after. Still single-pass, still rayon-parallelizable.

### Replay (RFC-003) interaction

Replay sources the payload from `quarantine_events.payload`, which is
already the transformed form. That means:

- A replay pass writes the *transformed* payload to the new `audit_log`
  row (identical to the source quarantine row, just passing now).
- Transforms are **not** re-applied at replay time — they were already
  applied at ingest. Re-applying `hash` would be a no-op (deterministic);
  re-applying `mask opaque` would also be a no-op; but re-applying
  `format_preserving` to an already-masked string would re-shuffle an
  already-shuffled value, which is meaningless.
- This creates a useful invariant: once a value is transformed, it's
  transformed forever, no matter how many replays it goes through.

Forward-on-replay (RFC-003 Q3 = yes) is unchanged — the forward
destination receives the same transformed payload it would have received
had the event passed the first time.

### Dashboard changes

- Audit log detail page: render the stored payload as-is (it is the
  transformed form). Add a small gray "🔒 transformed" badge next to
  fields that the contract marks with a transform — the dashboard
  learns which fields are transformed by reading the contract's
  ontology, not by inspecting the stored values (too error-prone).
- Contract edit page (VisualBuilder): add a "Transform" dropdown to
  each entity row (None / Mask / Hash / Drop / Redact) with a style
  sub-dropdown for Mask (Opaque / Format Preserving).
- Contract edit page: add a "Compliance mode" toggle at the version
  level with hover tooltip "Rejects events containing any field not
  declared below."
- Contract-create / version-publish confirmation: warn if the contract
  has `compliance_mode: true` but no `transform` on any string entity
  ("You've enabled compliance mode but no fields are masked — raw
  string values will still be stored. Are you sure?").

## Decisions (signed off 2026-04-19)

- **Q1 → Validate raw, then transform.** Validator sees original values
  so existing pattern/enum/min/max rules work intuitively. Transforms
  run after validation on pass *and* fail paths. Audit/quarantine/
  forward all receive the transformed payload.
- **Q2 → Inline on each entity.** `transform:` block co-located with
  the entity it applies to. Matches how `pattern:` / `enum:` / `min:`
  are already co-located; keeps field config in one place.
- **Q3 → Apply transforms before quarantine write.** "Never store raw
  PII" is absolute. Quarantined events get transformed too; replay
  works off the already-transformed form (see Replay interaction).
- **Q4 → Per-event 422 violation.** Compliance-mode undeclared-field
  rejection uses the existing per-item violation channel, not a
  whole-batch reject. Consistent with every other validation failure.
- **Q5 → Echo transformed payload.** On successful ingest, the HTTP
  response body contains the post-transform payload, not the raw.
  Keeps "raw PII never leaves the validator" true end-to-end. Announce
  as a breaking change in the release notes for any caller that
  depended on the echo-back matching the request byte-for-byte; the
  breakage is scoped to contracts that actually declare transforms.
- **Q6 → Seeded PRNG per-position scramble** for
  `mask: format_preserving`. ChaCha20 seeded with
  `HMAC-SHA256(contract.pii_salt, entity_name)`. Digit→digit,
  letter→letter (case-preserving), symbols pass through. Deterministic
  per contract, not reversible, not a formal FPE scheme. A proper
  FF1/FF3 upgrade stays on the non-goals list until a compliance
  surface demands it.
- **Q7 → Apply the field's transform to `violations[].actual_value`.**
  Both the API response and the stored `quarantine_events.violations`
  blob carry the transformed value for fields that declare a
  transform. "Raw never leaves" stays honest end-to-end. Debugging UX
  on pattern-mismatch violations for transformed fields gets slightly
  worse — accepted trade-off; the dashboard can compensate by
  rendering the violation's `pattern` + `message` prominently.

Original option lists retained in the AskUserQuestion rounds from
2026-04-19 (see session transcript).

## Test plan

### Unit (no DB)
1. `apply_transforms` with each kind: `mask:opaque`, `mask:format_preserving`,
   `hash`, `drop`, `redact`. Round-trip through JSON.
2. `hash` is deterministic: same salt + same input → same output.
3. `hash` is salt-isolating: same input, different contract salts →
   different outputs.
4. `format_preserving` preserves char class per position:
   `"+1 415-555-0199"` → `/^\+1 \d{3}-\d{3}-\d{4}$/`.
5. `drop` removes the field; JSON object no longer has the key.
6. `redact` replaces value with `"<REDACTED>"` and keeps the key.
7. Non-string transform field in the contract YAML → compile-time
   error at contract save (not a runtime surprise at ingest).
8. Compliance mode: event with undeclared field raises
   `UNDECLARED_FIELD` violation.
9. Compliance mode: after transform, stored payload does NOT contain
   the undeclared field.
10. Compliance mode off: undeclared fields pass through untouched
    (backwards compat).
11. `apply_transforms` on a contract with no transforms is an identity
    (returns equivalent object).
12. Serializer for `GET /contracts/:id` does NOT include `pii_salt`.
13. The same for `GET /contracts/:id/versions/:v`.

### DB-backed integration
14. Ingest with a `hash` transform → `audit_log.payload.email` is a
    `hmac-sha256:...` hex string, never the raw email.
15. Ingest with a `drop` transform → the field is absent from
    `audit_log.payload`.
16. Failed ingest (validation violation on a different field) with
    masked `email` → `quarantine_events.payload.email` is masked.
17. Failed ingest under compliance mode (undeclared field) →
    quarantine row's payload contains only declared fields.
18. Replay of a quarantine row with a `format_preserving` mask → the
    audit_log row's masked value equals the quarantine row's masked
    value (no double-transform).
19. Forward destination receives the transformed payload (mock server
    asserts). For a `hash` transform, forward body's email field is the
    hash, not the raw.
20. Two contracts with identical ontologies and identical `email`
    values hash to **different** outputs (salt isolation).
21. `contracts.pii_salt` is not returned by any public API (GET
    contract, GET version, GET audit, GET quarantine).

### Dashboard (Playwright / manual)
22. Visual builder: add a transform to an entity, save, reload — the
    transform persists.
23. Audit detail page: transformed fields show the 🔒 badge.
24. Compliance mode toggle fires the "no masked fields" warning when
    the contract has none.

## Rollout

1. Write migration `005_pii_transforms.sql`:
   - `ALTER TABLE contracts ADD COLUMN pii_salt BYTEA NOT NULL DEFAULT gen_random_bytes(32);`
   - `ALTER TABLE contract_versions ADD COLUMN compliance_mode BOOLEAN NOT NULL DEFAULT false;`
2. Extend `src/contract.rs`:
   - `Transform { kind: TransformKind, style: Option<MaskStyle> }` on
     `OntologyEntity` (optional).
   - `compliance_mode: bool` on the version-level struct.
   - `pii_salt: Vec<u8>` on `CompiledContract`, stripped by the HTTP
     serializer for contract-read paths.
3. Compile-time validation: reject contracts declaring a transform on
   non-string entity types.
4. New `src/transform.rs`: `apply_transforms`, `hmac_sha256_hex`,
   `format_preserving_mask`. Unit tests §1–7, §11.
5. Extend validator (`src/validation.rs`): UNDECLARED_FIELD violation
   under `compliance_mode = true`. Unit tests §8–10.
6. Tighten storage types so `AuditEntryInsert::payload` and
   `QuarantineEventInsert::payload` take `TransformedPayload`, not
   `serde_json::Value`. Compiler-enforced: you cannot write raw into
   audit/quarantine without going through `apply_transforms`.
7. Update ingest handler (`src/ingest.rs`) to the
   validate → transform → write pipeline.
8. Update forward path to send `TransformedPayload`.
9. Update replay handler (`src/replay.rs`) to take
   `TransformedPayload` out of the source quarantine row and pass it
   through unchanged (no re-transform).
10. Contract CRUD: generate/accept salt server-side, never echo it.
    Unit tests §12–13.
11. Dashboard: VisualBuilder transform dropdowns; audit-detail badge;
    compliance-mode toggle + warning.
12. DB integration tests §14–21.
13. Dashboard tests §22–24.
14. Update MAINTENANCE_LOG with the RFC-004 rollout entry.

## Decisions locked in before implementation

- Q1–Q7 — all signed off 2026-04-19. No open questions remain.

Work top-down through the rollout list on
`nightly-maintenance-2026-04-19`.
