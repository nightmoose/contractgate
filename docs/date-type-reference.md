# `date` Contract Field Type — Reference

**RFC:** 044  
**Status:** Accepted  
**Added:** nightly-maintenance-2026-05-17-rfc044-date-type; doc added 2026-05-24

---

## Overview

The `date` field type enforces calendar-aware date validation on JSON string
values. Unlike `type: string` with a `pattern` constraint (which only checks
format), `type: date` also verifies that the value represents a real calendar
date — month ≤ 12, day ≤ days in the month, no February 30th, and so on.

---

## Contract format

```yaml
ontology:
  entities:
    - name: start_date
      type: date
      required: true

    - name: end_date
      type: date
      required: false
```

No extra keys are needed. The `type: date` declaration is sufficient to activate
calendar validation.

The following constraints are still accepted on a `date` field and run **after**
calendar validation:

- `pattern` — additional regex match on the string value.
- `min_length` / `max_length` — string length bounds (rarely useful since all
  valid YYYY-MM-DD values are 10 characters, but accepted for consistency).

`min` and `max` (numeric range bounds) are **not** supported on `date` fields.
Chronological range constraints are a future RFC.

---

## Validation rules

1. The value must be a JSON string. A non-string value (number, boolean, null)
   produces a `type_mismatch` violation.
2. The string must parse as a valid calendar date in `YYYY-MM-DD` format using
   `chrono::NaiveDate::parse_from_str`. Values that are syntactically formatted
   but calendrically impossible — month > 12, day > last day of that month,
   negative zero, etc. — produce a `pattern_mismatch` violation.

---

## Violation messages

| Scenario | `kind` | Example message |
|---|---|---|
| Value is not a string | `type_mismatch` | `"Field 'start_date' expected string, got number"` |
| String is not a valid calendar date | `pattern_mismatch` | `"Field 'start_date' value \"2026-15-17\" is not a valid calendar date (expected YYYY-MM-DD)"` |

---

## Examples

### Valid values

```json
{ "start_date": "2026-01-15" }
{ "start_date": "2000-02-29" }   // 2000 is a leap year — valid
{ "start_date": "1999-12-31" }
```

### Invalid values

```json
{ "start_date": "2026-15-17" }   // month 15 — calendar violation
{ "start_date": "2023-02-29" }   // 2023 is not a leap year — calendar violation
{ "start_date": "20260115" }     // wrong format — parse failure
{ "start_date": 20260115 }       // number, not string — type_mismatch
{ "start_date": "2026-1-5" }     // single-digit month/day — parse failure
```

---

## ODCS mapping

| ContractGate type | ODCS `logicalType` |
|---|---|
| `date` | `date` |

ODCS round-trips cleanly: `logical_to_field_type` maps both `"date"` and
`"date32"` to `FieldType::Date`.

---

## Migration / storage notes

`FieldType` is serialised as lowercase text in the database
(`serde rename_all = "lowercase"`). No DB schema change was required to add
this type — new contracts simply store `"date"` where they previously would have
stored `"string"`. Existing contracts using `type: string` with a date pattern
continue to work exactly as before; they remain format-only. Authors must opt in
to `type: date` to get calendar validation.

---

## Comparison: `type: string` + `pattern` vs `type: date`

| Behaviour | `type: string` + `pattern: "^\d{4}-\d{2}-\d{2}$"` | `type: date` |
|---|---|---|
| Format check (YYYY-MM-DD shape) | ✓ | ✓ |
| Rejects month > 12 | ✗ | ✓ |
| Rejects invalid day for month | ✗ | ✓ |
| Rejects February 30 | ✗ | ✓ |
| Validates leap-year February 29 | ✗ | ✓ |
| Additional `pattern` constraint | ✓ | ✓ (runs after calendar check) |

---

## Related

- [RFC-044](rfcs/044-date-type.md) — design rationale and acceptance criteria.
- [Contract format reference](../CLAUDE.md) — full contract YAML schema.
