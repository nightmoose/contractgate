# deploy-contract Reference

**RFC:** 028 — Contract Queryability  
**Since:** nightly-2026-05-14

---

## Overview

`deploy-contract` promotes a contract YAML directly to `stable` in a single atomic operation.  It is the production deploy path — use `push` for iterative development (draft versions) and `deploy-contract` for CI-gated releases.

On success it:
- Finds or creates the contract identity by name.
- Inserts the version as `stable` with `parsed_json`, `source`, `deployed_by`, and `deployed_at` populated.
- Deprecates all previously-stable versions for this contract.

On failure it:
- Returns an error (no DB changes) if pending quarantine events exist for the contract.
- Returns a 409 if the `(name, version)` pair already exists.

---

## CLI Usage

```
cg deploy-contract <FILE> [OPTIONS]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `FILE`   | Path to the contract YAML file. |

### Options

| Flag | Env | Description |
|------|-----|-------------|
| `--source <NAME>` | — | PMS vendor or logical feed name (e.g. `yardi`, `realpage`, `entrata`). |
| `--deployed-by <ID>` | `CONTRACTGATE_DEPLOYED_BY` | CI job ID or username recorded on the version row. |
| `--dry-run` | — | Parse and validate locally without sending to the gateway. |
| `--json` | — | Emit machine-readable JSON. |
| `--api-key <KEY>` | `CONTRACTGATE_API_KEY` | Gateway API key (service-role required). |

### Examples

```bash
# Deploy from CI, recording source and job ID
cg deploy-contract contracts/orders.yaml \
  --source yardi \
  --deployed-by "$CI_JOB_ID"

# Dry-run: validate YAML without touching the gateway
cg deploy-contract contracts/events.yaml --dry-run

# Machine-readable output for downstream scripts
cg deploy-contract contracts/leases.yaml --json | jq .deprecated_count
```

---

## API Endpoint

```
POST /contracts/deploy
Authorization: x-api-key <service-role-key>
Content-Type: application/json
```

### Request Body

```json
{
  "name":         "orders",
  "yaml_content": "...",
  "source":       "yardi",
  "deployed_by":  "ci-job-42"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Contract name — must match `name:` in the YAML. |
| `yaml_content` | string | yes | Raw YAML content. Parsed server-side. |
| `source` | string | no | PMS vendor or feed name. |
| `deployed_by` | string | no | CI job ID or username. |

### Response `201 Created`

```json
{
  "contract_id":      "uuid",
  "version_id":       "uuid",
  "name":             "orders",
  "version":          "1.2.0",
  "source":           "yardi",
  "deployed_by":      "ci-job-42",
  "deployed_at":      "2026-05-14T10:30:00Z",
  "deprecated_count": 1
}
```

### Error Responses

| Status | Condition |
|--------|-----------|
| 400 | Pending quarantine events exist for this contract. Resolve them first. |
| 400 | YAML is invalid or unparseable. |
| 409 | `(name, version)` already exists in `contract_versions`. |
| 401 | Missing or invalid API key. |

---

## Supabase Queryability

After deploy, the following SQL queries work against Supabase:

```sql
-- What contract version was active during incident window X?
SELECT c.yaml_content
FROM audit_log a
JOIN contract_versions c
  ON c.contract_id = a.contract_id AND c.version = a.contract_version
WHERE a.event_id = 'evt_abc123';

-- Which active contracts allow monthly_rent = 0?
SELECT c.name, cv.version
FROM contract_versions cv
JOIN contracts c ON c.id = cv.contract_id
WHERE cv.state = 'stable'
  AND (cv.parsed_json -> 'ontology' -> 'entities'
       @> '[{"name":"monthly_rent","min":0}]');

-- Per-contract violation rates
SELECT contract_id, contract_version, passed, count(*)
FROM audit_log
GROUP BY 1, 2, 3
ORDER BY 4 DESC;
```

### `active_contracts_public` View

Auditors and external stakeholders can be granted SELECT on this view without full DB access:

```sql
-- Grant auditor access
GRANT SELECT ON active_contracts_public TO <auditor_role>;

-- Query active contracts
SELECT name, version, source, deployed_at, deployed_by
FROM active_contracts_public;
```

Columns: `contract_id`, `name`, `version`, `source`, `deployed_at`, `deployed_by`, `parsed_json`.

---

## Admin-Only Deprecation

Only the `deploy-contract` path (service-role API key) can deprecate stable versions.  Regular authenticated users cannot call `POST /contracts/deploy` — the standard auth middleware enforces this via org-scoped RLS.

To manually deprecate a version without deploying a replacement, use the existing endpoint:

```
POST /contracts/{id}/versions/{version}/deprecate
```

This requires a service-role key and is blocked if the version has pending quarantine events.
