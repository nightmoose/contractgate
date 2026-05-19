# PII Masking & Egress Leakage Guard Reference

ContractGate enforces a two-sided PII guarantee:

- **Ingest (RFC-004):** raw PII never lands in durable storage.
- **Egress (RFC-030):** raw PII and undeclared internal fields never leave the API.

Both directions reuse the same transform engine (`src/transform.rs`) and the
same per-contract `pii_salt`, so a value hashed on ingest produces an identical
hash on egress — downstream joins on hashed keys stay consistent.

---

## Field-level transforms

Declared in the contract YAML under `ontology.entities[*].transform`:

```yaml
ontology:
  entities:
    - name: user_email
      type: string
      required: true
      transform:
        kind: mask            # or: hash | drop | redact
        style: opaque         # only for mask; omit for default (opaque)
```

| `kind`   | Ingest result                          | Egress result                          |
|----------|----------------------------------------|----------------------------------------|
| `mask`   | `"****"` (opaque) or same-length scramble (format_preserving) | Same — identical output for same input |
| `hash`   | `"hmac-sha256:<hex>"` keyed on `pii_salt` | Same hash — salt continuity guaranteed |
| `drop`   | Field removed from stored payload      | Field absent from response             |
| `redact` | `"<REDACTED>"`                         | `"<REDACTED>"`                         |

Transforms apply to **top-level string fields only** in v1. A non-string field
with a `transform:` block is rejected at contract compile time.

### Mask styles

| `style`               | Behavior |
|-----------------------|----------|
| `opaque` (default)    | Replace entire value with `"****"`. Length does not leak. |
| `format_preserving`   | Preserve length + character class per position (digit→digit, letter→same-case letter, symbols unchanged). Deterministic per `(salt, field_name)`. |

---

## Egress leakage guard (RFC-030)

Applies to `POST /egress/{contract_id}` after field-level transforms.
Controlled by `egress_leakage_mode` at the contract-version level.

```yaml
# Set at the root of the contract YAML
egress_leakage_mode: strip    # off | strip | fail
```

| `egress_leakage_mode` | Behavior on an undeclared field in the outbound payload |
|-----------------------|----------------------------------------------------------|
| `off` (default)       | Field passes through untouched. Backwards-compatible. |
| `strip`               | Field removed from response. Name recorded in `stripped_fields` on the per-record outcome. No violation raised. |
| `fail`                | Field removed from response **and** a `LeakageViolation` is raised. The record is then subject to the RFC-029 disposition (`block` / `fail` / `tag`). |

> **Note on `fail` + `tag` disposition:** even in `tag` disposition (records
> pass through flagged), undeclared fields are still stripped. The leakage
> guarantee holds regardless of disposition.

### Per-record outcome fields

When leakage is active the egress response includes per-record `stripped_fields`:

```json
{
  "outcomes": [
    {
      "index": 0,
      "passed": false,
      "action": "blocked",
      "stripped_fields": ["cost_basis", "debug_trace"],
      "violations": [
        {
          "field": "cost_basis",
          "message": "Undeclared field 'cost_basis' found in egress payload ...",
          "kind": "leakage_violation"
        }
      ]
    }
  ]
}
```

`stripped_fields` is omitted from the JSON when empty (no undeclared fields
were stripped), keeping responses compact in the common case.

---

## Pipeline order

```
outbound payload
      │
      ▼
┌─────────────────────┐
│  validate()         │  ← sees raw values; rule checks run on original data
└────────┬────────────┘
         │
         ▼
┌─────────────────────┐
│  apply_transforms() │  ← RFC-004: drop/hash/mask/redact declared fields
│  + leakage guard    │     RFC-030: strip/fail undeclared fields
└────────┬────────────┘
         │
         ▼
  cleaned payload returned to caller
  + audit_log / quarantine (post-transform form, direction='egress')
```

**The payload returned by `POST /egress/...` is always the post-transform,
post-leakage payload.** Raw PII never appears in any API response.

---

## Database migration

Apply `supabase/migrations/018_egress_leakage_guard.sql` to add the
`contract_versions.egress_leakage_mode` column:

```bash
psql $DATABASE_URL -f supabase/migrations/018_egress_leakage_guard.sql
```

Existing rows default to `'off'` — no behavior change for deployed contracts
until they opt in.

---

## Salt continuity

`contracts.pii_salt` (introduced by RFC-004) is reused verbatim on egress.
A value hashed on ingest:

```
"hmac-sha256:a3f8c2..."
```

produces the same output when hashed on egress for the same contract and same
input. This means downstream analytics joins on hashed user IDs, order IDs,
or other keying fields work correctly across ingest and egress paths without
any additional configuration.

The salt is never serialized, never returned by any API endpoint, and is not
included in any response body.
