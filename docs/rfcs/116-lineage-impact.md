# RFC-116 — Downstream impact analysis

**Status:** Accepted
**Date:** 2026-11-19

## Problem

A contract change does not say which tables, models, dashboards, or consumers sit downstream, or which field edits are breaking. LineageGate (a separate graph app) answers that question for a synthetic warehouse. DataContractGate does not.

## Decision

Keep the lineage graph in the existing Postgres database. Do not add FalkorDB, and do not call the separate lineagegate app.

- `lineage_assets` and `lineage_edges` are org-scoped. An edge points downstream: the upstream asset feeds the downstream asset. `field_name`, when set, limits that edge to one contract field.
- Import replaces the org graph. The body is a dbt `manifest.json` or an edge list (JSON or YAML). A dbt node links to a contract when `meta.contract_id` or `meta.datacontractgate_contract` matches a contract id or name in the org.
- When a version is proposed (`POST /contracts`, `POST /contracts/{id}/versions`, a draft YAML edit) or published (`promote`, `deploy`), the gateway stores an impact report if the owning org is entitled and migration 054 is applied. A caller can recompute with `POST /contracts/{id}/versions/{version}/impact`.
- The walk is a recursive SQL query, at most 8 hops, with cycle detection. Field changes are classified in process: removed or renamed fields, type narrowing, new required fields, enum removals, and tighter bounds are breaking. Optional additions, type widening, and relaxed bounds are safe.
- Each affected asset gets a plain-English sentence from a template (name, kind, owner, hop count, path, breaking and safe changes). `CONTRACTGATE_IMPACT_LLM_URL` may rewrite the one-line summary. It is unset by default. Asset sentences stay on the templates. No new secret is required.
- The feature is the Impact plan (`orgs.plan = 'impact'`) and Enterprise. Pro is not included until `IMPACT_ANALYSIS_PLANS` in `src/plan.rs` says so. Self-host is included for every stored plan. Free orgs receive HTTP 402 `plan_limit` and no asset data. Caps (`IMPACT_ASSET_LIMIT`, `IMPACT_EDGE_LIMIT`, `IMPACT_MONTHLY_EVENT_LIMIT`) are constants in the same file. The public pricing page is unchanged.

The gateway boots if migration 054 is not applied. Version writes skip the report. Entitled impact routes return 503 until the tables exist.

## Dropped from LineageGate

FalkorDB, the chat agent, incident memory, failed-job triage, PageRank and betweenness, and the standalone UI. Those do not answer "what does this contract change break?" inside DataContractGate.

## API

See [`docs/impact-analysis-reference.md`](../impact-analysis-reference.md).
