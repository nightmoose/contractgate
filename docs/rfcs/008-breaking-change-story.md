# RFC-008: Breaking-Change Story

| Status        | Draft — awaiting sign-off                                              |
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 03 — Breaking-Change Story                                   |
| Depends on    | RFC-002 (versioning) — landed; RFC-006 (`/contracts/diff`) — landed; RFC-007 (CLI) |

## Summary

Make versioning useful. Surface what breaks, who it breaks, and how to migrate.

Five deliverables:

1. **Locked breaking-change taxonomy** — a closed set of change kinds, each
   classified `breaking | non_breaking | unknown`, computed deterministically
   from two contract versions.
2. **`contractgate diff` CLI** — wraps the existing `POST /contracts/diff`
   endpoint with rich human + `--json` output and a non-zero exit on any
   `breaking` change.
3. **Impact estimator** — `GET /contracts/:id/impact?from=v1&to=v2` returns
   the count of distinct producers and quarantine events that would be
   affected if the proposed diff were applied, computed from `audit_log`.
4. **Migration suggester** — rule-based proposals for non-breaking migration
   paths (rename → alias, type widen → coerce, enum reduce → keep deprecated).
   `Suggester` trait reserves an LLM backend slot, mirroring RFC-006's
   `DiffSummarizer` pattern. Patent-core stays deterministic.
5. **Audit log search & filter UI** — Audit tab gets a query bar
   (contract, version, kind, time window, free-text on payload) and a saved
   filter chip pattern. Reuses Quarantine tab table component.

## Goals

1. Every diff entry produced by `/contracts/diff` carries a `severity` field
   sourced from the locked taxonomy. CLI exits non-zero on any `breaking`.
2. Impact estimator hits a single materialized view; query stays under 50ms
   p99 even with 90 days of `audit_log` data.
3. Migration suggester returns within 10ms of the diff handler completing —
   it operates on the same in-memory `DiffChange` set, no extra round-trip.
4. Audit search UI re-uses the existing audit table; query state is in the URL
   so filters are shareable.
5. Validation engine p99 budget (<15ms) is unaffected — none of this work
   touches the hot path.

## Non-goals

- LLM-backed migration suggestions. Trait reserved; rule-based ships first.
- Producer attribution beyond what `audit_log.producer_id` already exposes.
- "Apply suggested migration" auto-edit of the contract YAML. Suggester
  emits a *proposal*; the user edits.
- Cross-contract impact (consumer graph). That depends on telemetry not yet
  collected — separate RFC after Chunk 4 lands `/metrics`.
- Breaking-change *prevention* gates (block publish on breaking). The CLI
  exits non-zero; whether that gate is enforced is the consumer's CI choice.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Taxonomy extensibility | **Closed set in v1.** Hand-curated list below. Adding a kind requires an RFC amendment. |
| Q2 | Severity classifier | **Pure function `classify(kind, before, after) -> Severity`** in `contractgate-core`. No DB, no config. |
| Q3 | Impact window | **Rolling 7 days, configurable via query param** `?window=7d` (also accepts `24h`, `30d`). |
| Q4 | Impact storage | **Materialized view `audit_log_producer_window`** refreshed every 5 minutes via background task. Avoids scanning audit_log on every estimate call. |
| Q5 | Suggester surface | **Embedded in `/contracts/diff` response** as a sibling `suggestions` array. One round-trip. |
| Q6 | Suggester rules | **Six rules in v1** (see Design §). LLM extension point reserved. |
| Q7 | Audit UI search backend | **Reuse existing `/audit` endpoint with new query params** (`q`, `kind`, `producer_id`, `from`, `to`). No new endpoint. |
| Q8 | URL state | **Encoded as `?` query string in dashboard route** so filters shareable + bookmarkable. |

## Locked taxonomy (v1)

| Kind                  | Severity     | Trigger |
|-----------------------|--------------|---------|
| `field_added` (required) | breaking      | new field with `required: true` and no default |
| `field_added` (optional) | non_breaking  | new field with `required: false` |
| `field_removed`         | breaking      | field present in A, absent in B |
| `type_changed` (widen)  | non_breaking  | int → float, enum → string with same values |
| `type_changed` (narrow) | breaking      | float → int, string → enum |
| `type_changed` (other)  | breaking      | any other type swap |
| `required_added`        | breaking      | optional → required |
| `required_removed`      | non_breaking  | required → optional |
| `enum_value_added`      | non_breaking  | new value in `allowed_values` |
| `enum_value_removed`    | breaking      | value removed from `allowed_values` |
| `pattern_tightened`     | breaking      | new pattern is stricter (heuristic: longer regex, more `^`/`$` anchors) |
| `pattern_loosened`      | non_breaking  | inverse |
| `pattern_changed` (other)| unknown      | can't classify by length heuristic — flag for review |
| `min_increased`         | breaking      | numeric `min` raised |
| `min_decreased`         | non_breaking  | numeric `min` lowered |
| `max_decreased`         | breaking      | numeric `max` lowered |
| `max_increased`         | non_breaking  | numeric `max` raised |
| `transform_added`       | non_breaking  | new PII transform on a field |
| `transform_removed`     | breaking      | PII transform removed (raw values now hit storage) |

`unknown` exits 0 in the CLI by default; `--strict` bumps it to non-zero.

## Design

### `POST /contracts/diff` extension (additive)

Existing response (RFC-006):

```json
{
  "summary": "...",
  "changes": [{"kind": "...", "field": "...", "detail": "..."}, ...]
}
```

New fields, fully additive:

```json
{
  "summary": "...",
  "changes": [
    {
      "kind": "type_changed",
      "field": "amount",
      "detail": "integer → float",
      "severity": "non_breaking",     // NEW
      "rationale": "widening conversion"  // NEW
    }
  ],
  "suggestions": [                    // NEW
    {
      "field": "amount",
      "kind": "coerce_in_validator",
      "detail": "Accept integer values during a deprecation window; cast to float at validation."
    }
  ],
  "summary_severity": "non_breaking"  // NEW — max() over change severities
}
```

### `GET /contracts/:id/impact`

```
GET /contracts/:id/impact?from=v1&to=v2&window=7d
→ 200 OK
{
  "from_version": "v1",
  "to_version": "v2",
  "window": "7d",
  "producers_affected": 14,
  "events_affected": 28471,
  "quarantine_predicted": 312,        // estimated using rule classifier
  "breakdown_by_field": [
    {"field": "amount", "events": 28000, "kinds": ["type_changed"]}
  ]
}
```

Implementation: query the materialized view, join against the diff's
`changes` array, count unique `producer_id` and sum `event_count`.

### Materialized view (`audit_log_producer_window`)

```sql
CREATE MATERIALIZED VIEW audit_log_producer_window AS
SELECT
  contract_id,
  contract_version,
  producer_id,
  date_trunc('hour', created_at) AS hour_bucket,
  count(*) AS event_count
FROM audit_log
WHERE created_at >= now() - interval '90 days'
GROUP BY 1, 2, 3, 4;

CREATE INDEX ON audit_log_producer_window (contract_id, hour_bucket);
```

Refreshed every 5 minutes by a Tokio task in `src/main.rs::spawn_refresh_loop()`.

### Suggester rules (v1)

| Diff kind                | Suggestion                                     |
|--------------------------|------------------------------------------------|
| `field_removed`          | "Mark deprecated for one release before removal" |
| `field_added` (required) | "Ship as optional first; promote after backfill" |
| `type_changed` (narrow)  | "Coerce in validator during deprecation window" |
| `enum_value_removed`     | "Keep value but mark deprecated; reject in next major" |
| `required_added`         | "Default in validator; require in next major" |
| `pattern_tightened`      | "Run loose pattern with warning logs first"   |

All other diff kinds emit no suggestion.

### Audit log search UI

Audit tab gets a query bar above the existing table. Filters:

- **Contract** — multi-select dropdown sourced from `GET /contracts`.
- **Kind** — multi-select: `pass`, `fail`, `quarantined`.
- **Producer** — text input (matches `producer_id` exactly).
- **Time window** — `Last 1h | 24h | 7d | 30d | Custom`.
- **Free text** — full-text on `payload::text` (Postgres `to_tsvector`).

State encoded as `?contract=...&kind=fail&producer=...&window=24h&q=...`.
Reuses Quarantine tab's `<DataTable>` component.

## Test plan

- `tests/diff_severity.rs` — exhaustive matrix over the locked taxonomy.
- `tests/impact_estimator.rs` — seed 10k audit rows, assert producer + event
  counts within ±1.
- `tests/suggester.rs` — each of the six rules fires on its trigger and
  no others.
- Dashboard: Playwright test for audit tab — load with URL params, assert
  table filtered correctly.
- Materialized view refresh — manual smoke; assert `< 200ms` refresh on
  1M-row audit_log.

## Rollout

1. Sign-off this RFC.
2. `contractgate-core::diff::Severity` enum + `classify()` pure function.
3. Extend `infer_diff::diff_handler` to populate `severity`, `rationale`,
   `summary_severity`.
4. Suggester trait + `RuleBasedSuggester`. Wire into diff handler.
5. Materialized view migration + refresh task.
6. `GET /contracts/:id/impact` endpoint.
7. CLI `diff` subcommand (depends on RFC-007 CLI scaffold) — non-zero exit
   on `breaking`, `--strict` for `unknown`.
8. Audit search UI on dashboard.
9. `cargo check && cargo test`; dashboard build.
10. Update `MAINTENANCE_LOG.md`.

## Deferred

- LLM-backed suggester impl.
- Cross-contract / consumer-graph impact (needs Chunk 4 telemetry).
- Auto-apply suggestions to YAML.
- Webhook fire on `breaking` diff publish — fits Chunk 4 alerting work.
