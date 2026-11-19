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

Pro + Impact uses the Pro monthly event cap (25,000,000). There is no separate event cap. All of the constants above are in `src/plan.rs`. Retention for `impact` is 90 days, set in `log_retention_days` by migration 054. The public pricing page is not changed by this feature. Stripe checkout does not sell Pro + Impact; set `orgs.plan` to `impact` directly until billing is wired.

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

The comparison base is the previous version by creation time. A publish compares to the previous stable version when one exists. The first version has no base (`base_version` is null): the report records zero field changes and names the registered downstream assets without treating the initial fields as breaking. GET recomputes a stored first-version report if it still lists field changes from the old empty-baseline comparison.

## Lineage import

`POST /lineage` replaces the caller's lineage.

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

dbt `manifest.json`: models, seeds, snapshots, and sources become assets. `depends_on.nodes` becomes `feeds` edges (upstream to downstream). Exposures of type `dashboard` become dashboards with `displays` edges; other exposures become consumers. `meta.owner` or `config.meta.owner` is the owner. `meta.contract_id` or `meta.datacontractgate_contract` (or the same keys under `config.meta`) links the node to a contract in this org. Send the file text as `document`, or send the manifest object itself. `format` is `auto`, `edges`, or `dbt_manifest`.

`GET /lineage` lists up to 500 assets and 1000 edges plus the totals.

## Impact response

`GET` or `POST /contracts/{id}/versions/{version}/impact`

`changes[]` has `field`, `kind`, `severity` (`breaking` or `safe`), `detail`, and `explanation`.

Breaking kinds include `field_removed`, `field_renamed` (a field with `renamed_from`), `type_narrowed` (float to integer, or any to a specific type), `type_incompatible`, `required_added` (a new required field, or optional to required), `enum_value_removed`, and `constraint_narrowed` (tighter min, max, length, or pattern, or a new enum).

Safe kinds include `field_added_optional`, `type_widened` (integer to float, or anything to any), `required_relaxed`, `enum_value_added`, and `constraint_widened`.

`affected[]` has `name`, `kind`, `owner`, `hops`, `path`, and `explanation`. An unscoped edge carries every field change. A field-scoped edge carries only that field. The sentence is produced by a template. `explanation_source` is `template`. `summary_source` is `template` unless `CONTRACTGATE_IMPACT_LLM_URL` is set, in which case the gateway may POST the one-line summary there and fall back to the template if that call fails. The variable is optional.

## Dashboard

The contract version view has a Downstream impact section. It imports a manifest or an edge list and shows the report for the selected version. A 402 renders the upgrade message and does not render assets. The Upgrade link is a relative `/pricing` path so it stays on the current host; the API may still return the production pricing URL in `upgrade_url` for existing clients. The dashboard rewrites that production URL in the message text to `/pricing`.

## Migration

`supabase/migrations/054_lineage_impact.sql` adds `impact` to `orgs.plan`, plus `lineage_assets`, `lineage_edges`, and `contract_impact_reports`. Apply it to production before promoting a gateway build that should serve impact data. The process still starts without it.
