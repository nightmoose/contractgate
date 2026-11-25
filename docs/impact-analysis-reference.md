# Downstream impact analysis

DataContractGate stores lineage for an organization and, when a contract version is proposed or published, reports which downstream assets are affected and which field changes are breaking.

The graph lives in Postgres (`lineage_assets`, `lineage_edges`). Nothing in this feature calls FalkorDB or the separate lineagegate app.

## Plans

The stored plan value is `impact`, shown to customers as Pro + Impact (migration `054_lineage_impact.sql`). It is a superset of Pro: the 25 million monthly events, the Pro rate limit, 90-day retention, replay, payload storage, Connect Fivetran, and every other `is_paid` gate, plus impact analysis. Enterprise includes impact analysis and is not capped on assets or edges. Pro (`growth`, `aisg_pro`) does not include impact analysis. Self-host (`CONTRACTGATE_SELF_HOSTED`) includes it for every stored plan. Free and AISchemaGate Free receive HTTP 402 and no asset names.

| Constant | Default | Meaning |
|---|---|---|
| `IMPACT_PLAN` | `impact` | Stored `orgs.plan` value |
| `IMPACT_PLAN_NAME` | Pro + Impact | Name in the upgrade message |
| `IMPACT_ANALYSIS_PLANS` | `impact`, `enterprise` | Plans that may import lineage and read impact |
| `IMPACT_ASSET_LIMIT` | 2000 | Assets on Pro + Impact. Enterprise is uncapped |
| `IMPACT_EDGE_LIMIT` | 20000 | Edges on Pro + Impact. Enterprise is uncapped |
| `IMPACT_MAX_HOPS` | 8 | Downstream hops, including the first |

Pro + Impact uses the Pro monthly event cap (25,000,000). There is no separate event cap. All of the constants above are in `src/plan.rs`. Retention for `impact` is 90 days, set in `log_retention_days` by migration 054. The dashboard pricing page shows a Pro + Impact card ($499/month or $4,990/year). Stripe Checkout sells it once `STRIPE_PRICE_IMPACT_MONTHLY` and `STRIPE_PRICE_IMPACT_ANNUAL` are set on the dashboard deployment; until then the card shows "Talk to us". The webhook sets `orgs.plan` to `impact` for those prices. See `docs/stripe-billing-reference.md`.

A plan that is not entitled gets:

```json
{
  "error": "plan_limit",
  "code": "plan_limit",
  "resource": "impact_analysis",
  "limit": 0,
  "current": 0,
  "message": "Downstream impact analysis is included with Pro + Impact and Enterprise. Upgrade at https://app.datacontractgate.com/pricing"
}
```

The body has no assets, owners, or field changes.

## When a report is computed

The gateway writes a report synchronously, in the request that changed the contract, when the owning org is entitled and migration 054 is applied:

| Action | Trigger stored on the report |
|---|---|
| `POST /contracts` (first draft) | `version_created` |
| `POST /contracts/{id}/versions` | `version_created` |
| `PATCH` of a draft's YAML | `version_created` |
| Promote to stable | `version_published` |
| `POST /contracts/deploy` | `version_published` |
| `POST .../impact` | `recompute` |

`GET /contracts/{id}/versions/{version}/impact` returns the newest stored report. If none exists, it computes one (`recompute`). A failure to store the report does not fail the version write. If the migration is missing, the version write still succeeds and the impact routes return 503 for an entitled org.

The comparison base is the previous version by creation time, never a version created after the target. A publish compares to the previous stable version when one exists, and likewise ignores a stable version created after the target. The first version has no base (`base_version` is null): the report records zero field changes and names the registered downstream assets without treating the initial fields as breaking. GET recomputes a stored first-version report if it still lists field changes from the old empty-baseline comparison, and a stored report whose base is not an earlier version (the old lookup compared older versions with newer ones). GET also recomputes a stored report when the organization's lineage has been re-imported since that report was computed, so every version reflects the current graph, not only the version selected at import time. Each report carries a `lineage_fingerprint` identifying the graph it was computed against.

## Lineage import

`POST /lineage` replaces the caller's lineage. Importing replaces all lineage for your organization, not just this contract.

Edge list (JSON or YAML):

```yaml
edges:
  - upstream: contract:<contract-uuid-or-name>
    downstream: model:stg_orders
    downstream_name: stg_orders
    owner: finance@example.com
  - from: { kind: model, key: stg_orders, name: stg_orders }
    to: { kind: dashboard, key: finance.revenue, name: Revenue, owner: bi@example.com }
    edge: displays
    field: amount
```

`edge` is `feeds` (the default), `displays`, `refreshes`, or `owned_by`. `field` limits that edge to one contract field. Kinds are `contract`, `dataset`, `table`, `model`, `dashboard`, `consumer`, and `job`.

Each endpoint can be given either form: a flat `kind:key` string (`upstream: contract:<contract-uuid-or-name>`), or an object with a bare key (`{ kind: contract, key: <contract-uuid-or-name> }`). An object key prefixed with its own kind (`{ kind: contract, key: "contract:<contract-uuid-or-name>" }`) is also accepted; the duplicate prefix is stripped so the node still links.

dbt `manifest.json`: models, seeds, snapshots, and sources become assets. `depends_on.nodes` becomes `feeds` edges (upstream to downstream). Exposures of type `dashboard` become dashboards with `displays` edges; other exposures become consumers. `meta.owner` or `config.meta.owner` is the owner. `meta.contract_id` or `meta.datacontractgate_contract` (or the same keys under `config.meta`) links the node to a contract in this org. Send the file text as `document`, or send the manifest object itself. `format` is `auto`, `edges`, or `dbt_manifest`.

`GET /lineage` lists up to 500 assets and 1000 edges plus the totals.

## Impact response

`GET` or `POST /contracts/{id}/versions/{version}/impact`

`changes[]` has `field`, `kind`, `severity` (`breaking` or `safe`), `detail`, and `explanation`.

Breaking kinds include `field_removed`, `field_renamed` (a field with `renamed_from`), `type_narrowed` (float to integer, or any to a specific type), `type_incompatible`, `required_added` (a new required field, or optional to required), `enum_value_removed`, and `constraint_narrowed` (tighter min, max, length, or pattern, or a new enum).

Safe kinds include `field_added_optional`, `type_widened` (integer to float, or anything to any), `required_relaxed`, `enum_value_added`, and `constraint_widened`.

`affected[]` has `name`, `kind`, `owner`, `hops`, `path`, `explanation`, `severity`, and `changes`. An unscoped edge carries every field change. A field-scoped edge carries only that field. The sentence is produced by a template. `explanation_source` is `template`. `summary_source` is `template` unless `CONTRACTGATE_IMPACT_LLM_URL` is set, in which case the gateway may POST the one-line summary there and fall back to the template if that call fails. The variable is optional.

`affected[].severity` is `breaking` when any field change reaching that asset is breaking, `safe` when every reaching change is safe, or `none` when the asset is listed with no reaching field changes (the first-version case, where there is no base to classify against). `affected[].changes` is the subset of the top-level `changes[]` that reaches that asset once field-scoped edges on its path have narrowed the set — the same list `explanation` is built from, as structured data instead of a sentence.

## Lineage graph

`GET /contracts/{id}/lineage/graph`

Same gate as the impact routes (Pro + Impact or Enterprise; 402 with no asset data otherwise) and the same assets and edges as `GET /lineage` (capped at 500 assets, 1000 edges), scoped to the contract's owning org. Adds `focus_keys`, the asset keys that belong to this contract (usually the contract's own node plus any asset the import linked via `meta.contract_id`), so the dashboard can highlight or scope the view to them. It does not take a version or compute impact; combine it client-side with `affected[].severity` and `affected[].changes` from the impact response for a selected version to color nodes.

## Dashboard

The contract version view has a Downstream impact section. It imports a manifest or an edge list and shows the report for the selected version. A 402 renders the upgrade message and does not render assets. The Upgrade link is a relative `/pricing` path so it stays on the current host; the API may still return the production pricing URL in `upgrade_url` for existing clients. The dashboard rewrites that production URL in the message text to `/pricing`.

A Graph / List toggle switches between an interactive lineage graph (React Flow, dagre layout) and the plain list above. The graph renders every asset and edge for the org, scoped to this contract's connected component by default (toggle to "Whole org" for the full graph), colors nodes by `affected[].severity` for the selected version, and shows an edge's field scope on hover. Clicking a node opens a details panel with the field changes reaching it. The graph falls back to the list automatically if its own request fails or the migration is not applied (but not on a 402, which shows the upgrade message instead).

## Migration

`supabase/migrations/054_lineage_impact.sql` adds `impact` to `orgs.plan`, plus `lineage_assets`, `lineage_edges`, and `contract_impact_reports`. Apply it to production before promoting a gateway build that should serve impact data. The process still starts without it.
