# RFC-012: Templates + Marketplace

| Status        | **Superseded** by RFC-017 (3 starters only); rest deferred (2026-04-27)|
|---------------|------------------------------------------------------------------------|
| Author        | ContractGate team                                                      |
| Created       | 2026-04-27                                                             |
| Target branch | `nightly-maintenance-YYYY-MM-DD` (set when work starts)                |
| Chunk         | Punchlist 07 — Templates + Marketplace                                 |
| Depends on    | RFC-007 (CLI surface), RFC-011 (SDK pull path)                         |

## Summary

Network-effect surface. Reusable contract starters, public + private,
discoverable in-app and pullable programmatically.

Six items:

1. **Public template library** — REST, event, gRPC, dbt model starters.
2. **Versioned template registry API** — SDKs/CLI pull templates.
3. **In-app template browser** — search + one-click import.
4. **Submission pipeline** — lint, test, review, publish.
5. **Organization-private template namespaces** — internal reusable patterns.
6. **Community ratings & usage stats** — anonymous thumbs + telemetry.

## Goals

1. Templates are first-class contracts, stored in the same `contracts` table
   with a flag — no parallel storage layer.
2. Pulling a template into your gateway is one command:
   `contractgate template pull rest/users` clones the template YAML into
   `.contracts/users.yaml` of the cwd.
3. The browser tab in dashboard surfaces templates with search, filter by
   tag, and one-click "Import to org".
4. Submission pipeline runs the existing contract validator on every PR
   plus a tag-and-license lint, then humans review.
5. Private namespaces re-use org isolation from RFC-001 (or fall back to
   per-contract scoping until that lands).
6. Ratings and usage are aggregate-only; no per-user telemetry.

## Non-goals

- Paid marketplace tier / revenue-sharing for template authors. Free in v1.
- Template forking / inheritance. v1 templates are flat copies.
- Template-driven contract generation across formats (e.g. "REST → dbt
  schema") — that's an inference job, not a template job.
- AI-recommended templates on producer registration — separate RFC.

## Decisions (recommended — flag any to override)

| #  | Question | Recommendation |
|----|----------|----------------|
| Q1 | Hosting | **Templates served from the main gateway API** under `/templates/*`. Same domain as everything else. |
| Q2 | Storage | **`contracts` table + `is_template bool` + `template_namespace text`** ('public', 'org:<id>'). No new table. |
| Q3 | Submission gate | **GitHub PR-based**: templates live in `contractgate-templates` repo; merge-to-main publishes via webhook. Lint + automated tests + human approve. |
| Q4 | Private namespace scope | **Per-org via RFC-001 org_id** when tenancy lands; until then, a per-contract `template_namespace = "org:<contract_owner_id>"` fallback. |
| Q5 | Ratings | **Anonymous thumbs up/down per signed-in user, aggregated.** Stored on `template_ratings` table. |
| Q6 | Usage stats | **Increment counter on import, not on view.** `template_imports` table with org_id + template_id + ts. |
| Q7 | Versioning | **Reuse RFC-002 contract versioning.** Each template is a contract; new versions are new versions. CLI `template pull --version`. |
| Q8 | Seed corpus | **5–10 starters at launch**: REST event, REST resource, Kafka event, gRPC unary, dbt model, OpenAPI-derived, login event, payment event, IoT telemetry, audit log. |

## Current state

- No template concept anywhere.
- `contracts` table has the columns we'd want; new fields would be
  additive migration.
- Dashboard has tabs for Contracts, Audit, Quarantine, Versions, Playground.
  Templates tab is new.

## Design

### Schema additions

```sql
ALTER TABLE contracts
    ADD COLUMN is_template bool NOT NULL DEFAULT false,
    ADD COLUMN template_namespace text,            -- 'public' or 'org:<id>'
    ADD COLUMN template_tags text[] NOT NULL DEFAULT '{}',
    ADD COLUMN template_description text;          -- markdown short blurb

CREATE TABLE template_ratings (
    template_id uuid REFERENCES contracts(id) ON DELETE CASCADE,
    rater_id    uuid NOT NULL,                     -- API key id (today) / user id (post-tenancy)
    score       smallint NOT NULL CHECK (score IN (-1, 1)),
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (template_id, rater_id)
);

CREATE TABLE template_imports (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id uuid REFERENCES contracts(id),
    importer_id uuid NOT NULL,                     -- org_id when tenancy lands
    imported_at timestamptz NOT NULL DEFAULT now()
);
```

### Registry API

```
GET  /templates                      # list, paginated; ?namespace=public|org:<id>&tag=<...>&q=<...>
GET  /templates/:id                  # full contract YAML + metadata
POST /templates/:id/import           # copies into caller's org as a normal contract
POST /templates/:id/rate             # body: {score: 1 | -1}
```

`/templates` responses include aggregate ratings + import count:

```json
{
  "id": "...",
  "name": "rest_event",
  "namespace": "public",
  "tags": ["rest", "event", "starter"],
  "description": "Generic REST event…",
  "latest_version": "v3",
  "ratings": {"up": 142, "down": 4, "score": 0.97},
  "imports_total": 1289,
  "imports_30d": 87
}
```

### CLI surface (additive to RFC-007)

```
contractgate template list [--namespace public] [--tag rest]
contractgate template pull <name-or-id> [--version vN] [--out path]
contractgate template publish <path>     # PR helper for the templates repo
```

### Dashboard browser tab

New route `dashboard/app/(authed)/templates/page.tsx`:

- Search bar (full-text on name/description/tags).
- Tag filter chips.
- Card grid: title, description excerpt, tags, ratings, imports.
- Card click → drawer with full YAML preview + "Import" CTA.
- Import calls `POST /templates/:id/import` and routes to the imported
  contract's detail page.

### Submission pipeline

Templates repo `contractgate-templates`:

```
templates/
├── rest_event.yaml
├── rest_resource.yaml
├── kafka_event.yaml
├── ...
└── _meta/
    ├── rest_event.json     # tags, description, owner
    └── ...
```

Repo-level GitHub Action:
1. On PR: run `contractgate validate` (CLI from RFC-007) on every changed
   YAML; lint `_meta/*.json` against a JSON Schema; assert tags chosen
   from a closed list.
2. Human review (CODEOWNERS).
3. On merge to main: webhook hits `POST /admin/templates/sync` on the
   gateway, which clones the repo and upserts each template as
   `is_template=true, namespace='public'`.

### Private templates

Identical mechanics, namespace `'org:<id>'`, written via:

```
contractgate template publish ./my-internal-event.yaml --private
```

Until RFC-001 lands, `org` is approximated by the API key's `owner_id`.

## Test plan

- `tests/templates_api.rs` — list / get / import / rate flows.
- `tests/template_import_creates_contract.rs` — import a template, assert a
  new contract row exists with same YAML and `is_template=false`.
- Submission repo: GitHub Actions test run on a fixture PR.
- Dashboard: Playwright happy-path — search → preview → import.
- Ratings dedup: same rater rates twice, count stays at 1 (with possibly
  flipped sign).

## Rollout

1. Sign-off this RFC.
2. Migrations for `is_template`, `template_namespace`, `template_tags`,
   `template_description`, `template_ratings`, `template_imports`.
3. Registry API endpoints + handlers.
4. CLI subcommands (`template list`, `template pull`, `template publish`).
5. `contractgate-templates` repo bootstrap + 5–10 seed starters + GH
   Actions lint + sync webhook.
6. Dashboard Templates tab.
7. Ratings + imports counters.
8. `cargo check && cargo test`; dashboard build.
9. Update `MAINTENANCE_LOG.md`.

## Deferred

- Paid tier.
- Forking / inheritance.
- AI recommendations on producer registration.
- Cross-format generation (REST → dbt etc.).
