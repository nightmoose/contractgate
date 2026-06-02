# Chunk 3 — Breaking-Change Story

**Theme:** Make versioning useful. Surface what breaks, who it breaks, and how to migrate.
**Why now:** Versioning + audit logs already exist. Highest-leverage value extraction.

## Items

- [ ] CLI `diff` with breaking-change output `[M]` — depends on Chunk 2 base CLI.
- [ ] Breaking-change impact estimator `[M]` — # of consumers / producers affected before merge. Builds on versioning + audit log.
- [ ] AI-assisted migration suggester `[M]` — propose non-breaking path when a field changes (rename → alias, type widen → coerce, etc.).
- [ ] Audit log search & filter UI `[M]` — extends existing audit logging; user-facing surface for impact data.
- [ ] Evolution diff summarizer `[S]` — *(can also live in Chunk 1; place wherever it ships first)*.

## Surface to reuse

- Contract versioning store.
- `audit_log` table + the contract_version honesty fix (see `feedback_audit_honesty.md`).
- Quarantine tab patterns for the diff/impact UI.

## Open questions for the conversation

1. "Breaking change" definition — locked taxonomy (field removed, type narrowed, enum reduced, required added, regex tightened) or extensible plugin model?
2. Impact estimator: count distinct producers seen in audit log over rolling window N. What N? 7d default?
3. Migration suggester: rule-based first (cheap, explainable) or LLM-backed? Patent posture — keep core deterministic.
4. UI: standalone "Diff" tab or panel inside Versions tab?
5. RFC required.

## Suggested first step

Lock the breaking-change taxonomy in `docs/rfcs/00X-breaking-change-taxonomy.md`. Wire `cargo`-side detection logic next; UI and CLI then both consume it.
