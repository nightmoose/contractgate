# Finding — JSON null on optional fields fails type check

| Field | Value |
|-------|-------|
| Date | 2026-07-20 |
| Scenario | usgs_earthquake, github_events, nyc_311 |
| Surface | validation engine (Rust + Python SDK) |
| Severity | major (product semantics) / story (sales honesty) |
| Env | local SDK (mirrors gateway logic in `validation.rs`) |

## Product question

Q2 — Does the draft catch realistic bad data without rejecting good data?

## Observed

Real open-data feeds often include `"field": null` for sparse optional columns
(USGS `felt`/`alert`, GitHub `org_login`, 311 `closed_date`).

Gateway and SDK both treat a present `null` as a value to type-check. For a
field typed `string` / `integer` with `required: false`, **null → type_mismatch**.
Only **missing keys** skip validation for optional fields.

## Expected (buyer intuition)

Optional ≈ “null or omit is fine.” Many JSON producers emit null rather than
omitting keys.

## Evidence

- `src/validation.rs` `validate_fields`: `None` skips optional; `Some(value)`
  always type-checks (including `Value::Null`).
- Local dogfood run `findings/runs/20260720T201248Z` — 100% pass-batch failures
  until fixtures stripped nulls.

## Next experiment

1. **Docs / wizard copy:** “Omit optional fields; do not send null.”
2. **Product option (RFC candidate):** `null_as_absent: true` contract flag or
   default soften for optional fields.
3. Dogfood harness normalizes fixtures by dropping null keys (producer hygiene)
   so remaining failures are real quality issues.

## Resolution

- [x] Documented in dogfood findings
- [ ] Product decision (keep strict vs null-as-absent)
- [x] Harness normalizes samples for fair contract testing
