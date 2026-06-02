# RFC-015: Breaking-Change Demo Arc

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist v2 #2                                                        |
| Supersedes    | RFC-008 (broader scope; impact estimator deferred until audit volume)  |
| Depends on    | RFC-002 (versioning), RFC-006 (`/contracts/diff`), RFC-014 (CLI)       |

## Summary

Make versioning useful for demos. Five tightly coupled deliverables:

1. **Locked breaking-change taxonomy** — closed set, deterministic classifier.
2. **`severity` on `/contracts/diff` response** — additive fields, no shape break.
3. **Migration suggester** — rule-based, embedded in diff response.
4. **CLI `diff` subcommand** — wraps the endpoint, exits non-zero on `breaking`.
5. **Audit log search & filter UI** — filter bar above existing audit table.

Impact estimator deferred until real audit volume exists.

## Goals

1. Every diff entry carries a `severity` from the locked taxonomy.
2. CLI `diff` exits non-zero on any `breaking` change. `--strict` bumps
   `unknown` to non-zero too.
3. Migration suggester returns within 10ms of diff handler completing.
4. Audit search UI filters by contract, kind, producer, time window,
   free-text on payload. URL state shareable.
5. Validation engine p99 budget unaffected.

## Non-goals

- LLM-backed suggester. Trait reserved per RFC-006; rule-based ships.
- Impact estimator (`# producers / events affected`) — needs audit volume.
- Materialized view `audit_log_producer_window` — not built.
- `GET /contracts/:id/impact` endpoint — not built.
- Auto-apply suggested migration to YAML. Suggester emits proposals only.
- Cross-contract / consumer-graph impact — needs telemetry not collected.
- Breaking-change publish gate. CLI exits non-zero; consumer CI decides.

## Decisions

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Taxonomy extensibility | **Closed set in v1.** Adding a kind requires RFC amendment. |
| Q2 | Severity classifier | **Pure function `classify(kind, before, after) -> Severity`** in `crate::diff`. No DB, no config. |
| Q3 | Suggester surface | **Embedded in `/contracts/diff` response** as a sibling `suggestions` array. One round-trip. |
| Q4 | Suggester rules | **Six rules in v1** (table below). |
| Q5 | Audit UI search backend | **Reuse `/audit` with new query params** (`q`, `kind`, `producer_id`, `from`, `to`). No new endpoint. |
| Q6 | URL state | **Encoded as `?` query string** in dashboard route — shareable, bookmarkable. |

## Locked taxonomy (v1)

| Kind                    | Severity      | Trigger |
|-------------------------|---------------|---------|
| `field_added` (required) | breaking      | new field with `required: true`, no default |
| `field_added` (optional) | non_breaking  | new field with `required: false` |
| `field_removed`         | breaking      | field present in A, absent in B |
| `type_widened`          | non_breaking  | int → float; enum → string with same values |
| `type_narrowed`         | breaking      | float → int; string → enum |
| `type_changed` (other)  | breaking      | any other type swap |
| `required_added`        | breaking      | optional → required |
| `required_removed`      | non_breaking  | required → optional |
| `enum_value_added`      | non_breaking  | new value in `allowed_values` |
| `enum_value_removed`    | breaking      | value removed from `allowed_values` |
| `pattern_tightened`     | breaking      | longer regex / more anchors (heuristic) |
| `pattern_loosened`      | non_breaking  | inverse |
| `pattern_changed` (other)| unknown      | can't classify; flag for review |
| `min_increased`         | breaking      | numeric `min` raised |
| `min_decreased`         | non_breaking  | numeric `min` lowered |
| `max_decreased`         | breaking      | numeric `max` lowered |
| `max_increased`         | non_breaking  | numeric `max` raised |
| `transform_added`       | non_breaking  | new PII transform on a field |
| `transform_removed`     | breaking      | PII transform removed (raw values now stored) |

`unknown` exits 0 in CLI by default; `--strict` flips to non-zero.

## Design

### `/contracts/diff` response — additive fields

Existing (RFC-006):

```json
{
  "summary": "...",
  "changes": [{"kind": "...", "field": "...", "detail": "..."}]
}
```

After RFC-015 (additive only, no breaking change):

```json
{
  "summary": "...",
  "summary_severity": "non_breaking",
  "changes": [
    {
      "kind": "type_widened",
      "field": "amount",
      "detail": "integer → float",
      "severity": "non_breaking",
      "rationale": "widening conversion"
    }
  ],
  "suggestions": [
    {
      "field": "amount",
      "kind": "coerce_in_validator",
      "detail": "Accept integer values during deprecation; cast to float at validation."
    }
  ]
}
```

### Suggester rules (v1)

| Diff kind                | Suggestion |
|--------------------------|------------|
| `field_removed`          | "Mark deprecated for one release before removal" |
| `field_added` (required) | "Ship as optional first; promote after backfill" |
| `type_narrowed`          | "Coerce in validator during deprecation window" |
| `enum_value_removed`     | "Keep value but mark deprecated; reject in next major" |
| `required_added`         | "Default in validator; require in next major" |
| `pattern_tightened`      | "Run loose pattern with warning logs first" |

All other diff kinds emit no suggestion.

### CLI `diff`

```
contractgate diff <yaml-a> <yaml-b> [--json] [--strict]
  POST /contracts/diff with both files. Print summary + per-change
  rows. Exit 0 if all non_breaking, 1 if any breaking. With --strict,
  any unknown also exits 1.
```

### Audit log search UI

Audit tab gets a filter bar above the existing table:

- **Contract** — multi-select dropdown sourced from `GET /contracts`.
- **Kind** — multi-select: `pass`, `fail`, `quarantined`.
- **Producer** — text input matching `producer_id` exactly.
- **Time window** — `Last 1h | 24h | 7d | 30d | Custom`.
- **Free text** — full-text on `payload::text` via Postgres `to_tsvector`.

State encoded as `?contract=...&kind=fail&producer=...&window=24h&q=...`.
Reuses existing `<DataTable>` component. `/audit` endpoint extended
with the new query params; no new route.

## Test plan

- `tests/diff_severity.rs` — exhaustive matrix over the locked taxonomy.
- `tests/suggester.rs` — each of the six rules fires on its trigger.
- `tests/cli_diff.rs` — exit codes for non_breaking / breaking / unknown
  under default and `--strict`.
- Dashboard: Playwright test for audit tab — load with URL params, assert
  table filtered correctly.
- `/audit` query params: integration test asserts filter pushdown to SQL.

## Rollout

1. Sign-off this RFC.
2. `crate::diff::Severity` + `classify()` pure function. Unit-tested first.
3. Extend `infer_diff::diff_handler` to populate `severity`, `rationale`,
   `summary_severity`.
4. Suggester trait + `RuleBasedSuggester`. Wire into diff handler.
5. CLI `diff` subcommand (depends on RFC-014 scaffold).
6. `/audit` query params + filter pushdown.
7. Audit search UI on dashboard.
8. `cargo check && cargo test`; dashboard build.
9. Update `MAINTENANCE_LOG.md`.

## Deferred

- LLM-backed suggester impl.
- Impact estimator + materialized view + `/contracts/:id/impact`.
- Cross-contract impact graph.
- Auto-apply suggestions to YAML.
- Webhook on `breaking` diff publish — fits RFC-016 alerting if added.
