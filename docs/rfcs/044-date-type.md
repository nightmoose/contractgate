# RFC-044 — Native `date` Field Type

**Status:** Accepted  
**Date:** 2026-05-17  
**Branch:** `nightly-maintenance-2026-05-17-rfc044-date-type`

---

## Problem

`type: string` with `pattern: "^\d{4}-\d{2}-\d{2}$"` only validates format.
`2026-15-17` (month 15) passes. The engine has no calendar awareness.

## Solution

Add `FieldType::Date` — a first-class type that:
1. Requires the value to be a JSON string.
2. Parses it with `chrono::NaiveDate::parse_from_str` using `%Y-%m-%d`.
3. Rejects values that are syntactically formatted but calendrically impossible (month > 12, day > days-in-month, etc.).

## Contract Format

```yaml
- name: start_date
  type: date
  required: true
```

No extra keys needed. `pattern`, `min_length`, `max_length` are still accepted
on a `date` field and run after calendar validation.

`min` / `max` are not supported on `date` fields (chronological range constraints
are a future RFC).

## ODCS Mapping

| ContractGate | ODCS `logicalType` |
|---|---|
| `date` | `date` |

Round-trips cleanly. `logical_to_field_type` maps `"date"` and `"date32"` → `FieldType::Date`.

## Violation Kind

Invalid calendar value → `ViolationKind::PatternMismatch` with message:
```
Field 'start_date' value "2026-15-17" is not a valid calendar date (expected YYYY-MM-DD)
```

Non-string value → `ViolationKind::TypeMismatch` (existing behaviour).

## Migration / DB Impact

`FieldType` is stored as text in Supabase. No DB schema change required.
Existing contracts using `type: string` with a date pattern continue to work
exactly as before — they remain format-only. Authors must opt in to `type: date`
to get calendar validation.

## Breaking Changes

None for existing contracts. New `FieldType` variant is additive; the serde
`rename_all = "lowercase"` rule serialises it as `"date"`.
